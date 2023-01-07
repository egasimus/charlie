use crate::prelude::*;
use crate::app::App;
use crate::controller::Controller;

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        drm::{DrmDevice, GbmBufferedSurface, DrmEvent, DrmError},
        egl::{EGLContext, EGLDisplay},
        winit::{self, WinitGraphicsBackend, WinitInputBackend},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        session::{Session, Signal as SessionSignal, auto::{AutoSession, AutoSessionNotifier}},
        udev::{UdevBackend, primary_gpu},
        renderer::{Bind}
    },
    reexports::{
        calloop::{Dispatcher, RegistrationToken, timer::{Timer, TimerHandle}},
        drm::control::{
            Device,
            SystemError,
            crtc::Handle as CrtcHandle,
            connector::{Info as ConnectorInfo, State as ConnectorState, Interface},
            encoder::{Info as EncoderInfo}
        },
        gbm::Device as GbmDevice,
        input::Libinput,
        nix::{fcntl::OFlag, sys::stat::dev_t},
    },
    utils::signaling::{
        Linkable,
        SignalToken,
        Signaler
    },
};

pub trait Backend: Sized {
    type Renderer;
    type Input;
    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        unimplemented!()
    }
    fn post_init (app: &mut App<Self>, event_loop: &EventLoop<'static, App<Self>>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
    fn renderer (&mut self) -> &mut Gles2Renderer {
        unimplemented!()
    }
    fn draw (app: &mut App<Self>) -> Result<(), SwapBuffersError> {
        unimplemented!()
    }
    fn load_texture (&mut self, path: impl AsRef<Path>) -> Result<Gles2Texture, Box<dyn Error>> {
        unimplemented!()
    }
    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
    fn add_output (app: &mut App<Self>, name: impl AsRef<str>) {
        unimplemented!()
    }
}

pub struct Winit {
    pub renderer: <Self as Backend>::Renderer,
    pub input:    <Self as Backend>::Input,
}

impl Backend for Winit {
    type Renderer = WinitGraphicsBackend;
    type Input    = WinitInputBackend;
    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        let (renderer, input) = winit::init(log.clone())?;
        Ok(Self { renderer, input })
    }
    fn post_init (app: &mut App<Self>, event_loop: &EventLoop<'static, App<Self>>) -> Result<(), Box<dyn Error>> {
        let log = app.log.clone();
        let display = app.display.clone();
        event_loop.handle().insert_source(Generic::from_fd(
            app.display.borrow().get_poll_fd(),
            Interest::READ,
            CalloopMode::Level
        ), move |_, _, state: &mut App<Self>| {
            let mut display = display.borrow_mut();
            match display.dispatch(std::time::Duration::from_millis(0), state) {
                Ok(_) => {
                    Ok(PostAction::Continue)
                },
                Err(e) => {
                    error!(log, "I/O error on the Wayland display: {}", e);
                    state.running.store(false, Ordering::SeqCst);
                    Err(e)
                }
            }
        })?;
        app.backend.renderer().bind_wl_display(&app.display.borrow())?;
        init_dmabuf_global(
            &mut *app.display.borrow_mut(),
            app.backend.renderer().dmabuf_formats().cloned().collect::<Vec<_>>(),
            move |buffer, mut state| state.get::<App<Self>>().unwrap()
                .backend.renderer()
                .import_dmabuf(buffer).is_ok(),
            app.log.clone()
        );
        Ok(())
    }
    fn renderer (&mut self) -> &mut Gles2Renderer {
        self.renderer.renderer()
    }
    fn load_texture (&mut self, path: impl AsRef<Path>) -> Result<Gles2Texture, Box<dyn Error>> {
        import_bitmap(
            self.renderer(),
            &image::io::Reader::open(path)?
                .with_guessed_format().unwrap()
                .decode().unwrap()
                .to_rgba8()
        )
            .map_err(Into::<Box<dyn Error>>::into)
    }
    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        self.input
            .dispatch_new_events(|event| controller.process_input_event(event))
            .map_err(Into::<Box<dyn Error>>::into)
            //.map_err(|e|Box::new(e) as Box<dyn Error>)
    }
    fn draw (app: &mut App<Self>) -> Result<(), SwapBuffersError> {
        let workspace = app.workspace.borrow();
        let result = app.backend.renderer.render(|mut renderer, mut frame| {
            // This is safe to do as with winit we are guaranteed to have exactly one output
            frame.clear([0.8, 0.8, 0.8, 1.0])?;
            app.compositor.borrow().draw(&mut renderer, &mut frame, &workspace)?;
            app.controller.draw(&mut renderer, &mut frame, 1.0)?;
            Ok(())
        })?;
        app.backend.renderer.window().set_cursor_visible(app.controller.cursor_visible.get());
        result
    }
    fn add_output (app: &mut App<Self>, name: impl AsRef<str>) {
        app.compositor.borrow_mut().add_output(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode {
                size:    app.backend.renderer.window_size().physical_size,
                refresh: 60_000
            }
        );
    }
}

pub struct Udev {
    session:     AutoSession,
    notifier:    AutoSessionNotifier,
    signaler:    Signaler<SessionSignal>,
    input:       <Self as Backend>::Input,
    renderer:    <Self as Backend>::Renderer,
    primary_gpu: Option<PathBuf>,
    backends:    HashMap<dev_t, UdevData>,
    timer:       Timer<(u64, CrtcHandle)>,
}

pub struct UdevData {
    _restart_token:     SignalToken,
    surfaces:           Rc<RefCell<HashMap<CrtcHandle, Rc<RefCell<SurfaceData>>>>>,
    pointer_images:     Vec<(xcursor::parser::Image, Gles2Texture)>,
    renderer:           Rc<RefCell<Gles2Renderer>>,
    gbm:                GbmDevice<SessionFd>,
    registration_token: RegistrationToken,
    event_dispatcher:   Dispatcher<'static, DrmDevice<SessionFd>, App<Udev>>,
    dev_id: u64,
}

impl Backend for Udev {
    type Renderer = UdevBackend;
    type Input    = LibinputInputBackend;
    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        // Init session
        let (session, notifier) = AutoSession::new(log.clone())
            .expect("Could not initialize session.");
        let session_signal = notifier.signaler();
        // Init input
        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<AutoSession>>(
            session.clone().into()
        );
        let seat_name = "udev-seat";
        libinput.udev_assign_seat(seat_name).unwrap();
        let mut input = LibinputInputBackend::new(libinput, log.clone());
        input.link(session_signal);
        // Init render
        let renderer = UdevBackend::new(seat_name, log.clone())?;
        let primary_gpu = primary_gpu(&session.seat()).unwrap_or_default();
        let timer = Timer::new().unwrap();
        Ok(Self {
            session,
            notifier,
            signaler: session_signal.clone(),
            input,
            renderer,
            primary_gpu,
            backends: HashMap::new(),
            timer,
        })
    }
    fn post_init (app: &mut App<Self>, event_loop: &EventLoop<'static, App<Self>>) -> Result<(), Box<dyn Error>> {
        let render_event_source = event_loop.handle().insert_source(
            app.backend.timer,
            |(dev_id, crtc), _, app| { app.render_udev(dev_id, Some(crtc)) }
        )?;
        let libinput_event_source = event_loop.handle().insert_source(
            app.backend.input,
            move |event, _, app| { app.controller.process_input_event(event) }
        )?;
        let session_event_source = event_loop.handle().insert_source(
            app.backend.notifier,
            move |(), &mut (), app| { /*do nothing club*/ }
        )?;
        for (dev, path) in app.backend.renderer.device_list() {
            Udev::device_added(app, dev, path.into())?;
        }
        Ok(())
    }
}

impl Udev {

    const FLAGS: OFlag = OFlag::O_RDWR | OFlag::O_CLOEXEC | OFlag::O_NOCTTY | OFlag::O_NONBLOCK;

    fn device_added (
        app:       &mut App<Self>,
        device_id: dev_t,
        path:      PathBuf
    ) -> Result<(), Box<dyn Error>> {
        // Try to open the device
        let session = app.backend.session;
        let fd  = SessionFd(session.open(&path, Udev::FLAGS)?);
        let drm = DrmDevice::new(fd.clone(), true, app.log.clone())?;
        let gbm = GbmDevice::new(fd)?;
        let egl = EGLDisplay::new(&gbm, app.log.clone())?;
        let context = EGLContext::new(&egl, app.log.clone())?;
        let renderer = unsafe { Gles2Renderer::new(context, app.log.clone()).unwrap() };
        let renderer = Rc::new(RefCell::new(renderer));
        if path.canonicalize().ok() == app.backend.primary_gpu {
            info!(app.log, "Initializing EGL Hardware Acceleration via {:?}", path);
            if renderer.borrow_mut().bind_wl_display(&*app.display.borrow()).is_ok() {
                info!(app.log, "EGL hardware-acceleration enabled");
            }
        }
        let backends = Self::scan(&mut app, &mut drm, &gbm, &mut *renderer.borrow_mut());
        let backends = Rc::new(RefCell::new(backends));
        let dev_id   = drm.device_id();
        let handle   = app.handle.clone();
        let restart_token = app.backend.signaler.register(move |signal| match signal {
            SessionSignal::ActivateSession | SessionSignal::ActivateDevice { .. } => {
                handle.insert_idle(move |app| app.render_udev(dev_id, None));
            }
            _ => {}
        });
        drm.link(app.backend.signaler.clone());
        let event_dispatcher = Dispatcher::new(drm, move |event, _, app: &mut App<_>| match event {
            DrmEvent::VBlank(crtc) => { app.render_udev(dev_id, Some(crtc)); },
            DrmEvent::Error(error) => { error!(app.log, "{:?}", error) }
        });
        let registration_token = app.handle.register_dispatcher(event_dispatcher.clone()).unwrap();
        trace!(app.log, "Backends: {:?}", backends.borrow().keys());
        for backend in backends.borrow_mut().values() {
            // render first frame
            trace!(app.log, "Scheduling frame");
            Self::schedule_initial_render(
                backend.clone(), renderer.clone(), &app.handle, app.log.clone()
            );
        }
        app.backend.backends.insert(dev_id, UdevData {
            dev_id,
            event_dispatcher,
            gbm,
            pointer_images:     Vec::new(),
            registration_token,
            renderer,
            _restart_token:     restart_token,
            surfaces:           backends,
        });
        Ok(())
    }

    fn scan (
        app: &mut App<Self>,
        drm: &mut DrmDevice<SessionFd>,
        gbm: &GbmDevice<SessionFd>,
        renderer: &mut Gles2Renderer,
    ) -> HashMap<CrtcHandle, Rc<RefCell<SurfaceData>>> {
        let logger      = app.log.clone();
        let compositor  = app.compositor.borrow_mut();
        let signaler    = app.backend.signaler;
        let res_handles = drm.resource_handles().unwrap(); // Get a set of all modesetting
                                                           // resource handles (excluding planes)
        let connectors: Vec<ConnectorInfo> = res_handles.connectors().iter()
            .map(|conn| drm.get_connector(*conn).unwrap()) // Use first connected connector
            .filter(|conn| conn.state() == ConnectorState::Connected)
            .inspect(|conn| info!(logger, "Connected: {:?}", conn.interface()))
            .collect();
        let mut backends = HashMap::new(); // very naive way of finding good
                                           // crtc/encoder/connector combinations:
                                           // This problem is np-complete.
        for connector in connectors {
            let encoder_infos = connector.encoders().iter().filter_map(|e| *e)
                .flat_map(|encoder_handle| drm.get_encoder(encoder_handle))
                .collect::<Vec<EncoderInfo>>();
            'outer: for encoder_info in encoder_infos {
                for crtc in res_handles.filter_crtcs(encoder_info.possible_crtcs()) {
                    if let Entry::Vacant(entry) = backends.entry(crtc) {
                        info!(logger, "Trying to setup connector {:?}-{} with crtc {:?}",
                            connector.interface(), connector.interface_id(), crtc,);
                        let surface = drm.create_surface(
                            crtc, connector.modes()[0], &[connector.handle()],
                        );
                        let surface = match surface {
                            Ok(surface) => surface,
                            Err(err) => {
                                warn!(logger, "Failed to create drm surface: {}", err);
                                continue;
                            }
                        };
                        surface.link(signaler.clone());
                        let renderer_formats = Bind::<Dmabuf>::supported_formats(renderer)
                            .expect("Dmabuf renderer without formats");
                        let surface = match GbmBufferedSurface::new(
                            surface, gbm.clone(), renderer_formats, logger.clone()
                        ) {
                            Ok(renderer) => renderer,
                            Err(err) => {
                                warn!(logger, "Failed to create rendering surface: {}", err);
                                continue;
                            }
                        };
                        let mode = connector.modes()[0];
                        let size = mode.size();
                        let mode = OutputMode {
                            size: (size.0 as i32, size.1 as i32).into(),
                            refresh: (mode.vrefresh() * 1000) as i32,
                        };
                        let other_short_name;
                        let interface_short_name = match connector.interface() {
                            Interface::DVII                => "DVI-I",
                            Interface::DVID                => "DVI-D",
                            Interface::DVIA                => "DVI-A",
                            Interface::SVideo              => "S-VIDEO",
                            Interface::DisplayPort         => "DP",
                            Interface::EmbeddedDisplayPort => "eDP",
                            Interface::HDMIA               => "HDMI-A",
                            Interface::HDMIB               => "HDMI-B",
                            other => {
                                other_short_name = format!("{:?}", other); &other_short_name
                            }
                        };
                        let output_name =
                            format!("{}-{}", interface_short_name, connector.interface_id());
                        let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
                        let output = compositor.add_output(
                            &output_name,
                            PhysicalProperties {
                                size: (phys_w as i32, phys_h as i32).into(),
                                subpixel: wl_output::Subpixel::Unknown,
                                make: "Smithay".into(),
                                model: "Generic DRM".into(),
                            },
                            mode,
                        );
                        output.userdata().insert_if_missing(
                            || UdevOutputId { crtc, device_id: drm.device_id() }
                        );
                        entry.insert(Rc::new(RefCell::new(SurfaceData { surface })));
                        break 'outer;
                    }
                }
            }
        }
        backends
    }

    fn schedule_initial_render<Data: 'static>(
        surface:    Rc<RefCell<SurfaceData>>,
        renderer:   Rc<RefCell<Gles2Renderer>>,
        evt_handle: &LoopHandle<'static, Data>,
        logger:     ::slog::Logger,
    ) {
        let result = {
            let mut surface = surface.borrow_mut();
            let mut renderer = renderer.borrow_mut();
            Self::initial_render(&mut surface.surface, &mut *renderer)
        };
        if let Err(err) = result {
            match err {
                SwapBuffersError::AlreadySwapped
                    => {}
                SwapBuffersError::TemporaryFailure(err)
                    => {
                        // TODO dont reschedule after 3(?) retries
                        warn!(logger, "Failed to submit page_flip: {}", err);
                        let handle = evt_handle.clone();
                        evt_handle.insert_idle(
                            move |_| Self::schedule_initial_render(
                                surface, renderer, &handle, logger
                            )
                        );
                    }
                SwapBuffersError::ContextLost(err)
                    => panic!("Rendering loop lost: {}", err),
            }
        }
    }

    fn initial_render (
        surface:  &mut RenderSurface,
        renderer: &mut Gles2Renderer
    ) -> Result<(), SwapBuffersError> {
        let dmabuf = surface.next_buffer()?;
        renderer.bind(dmabuf)?;
        let clear = |_, frame: &mut Gles2Frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0]).map_err(Into::<SwapBuffersError>::into)
        };
        renderer.render((1, 1).into(), Transform::Normal, clear)
            .map_err(Into::<SwapBuffersError>::into)
            .and_then(|x| x.map_err(Into::<SwapBuffersError>::into))?;
        surface.queue_buffer()?;
        Ok(())
    }
}

struct SurfaceData {
    surface: RenderSurface,
}

pub type RenderSurface = GbmBufferedSurface<SessionFd>;

#[derive(Clone)]
pub struct SessionFd(RawFd);

impl AsRawFd for SessionFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

#[derive(Debug, PartialEq)]
struct UdevOutputId { device_id: dev_t, crtc: CrtcHandle }

impl App<Udev> {    // If crtc is `Some()`, render it, else render all crtcs
    fn render_udev (&mut self, dev_id: u64, crtc: Option<CrtcHandle>) {
        let backend = match self.backend.backends.get_mut(&dev_id) {
            Some(backend) => backend,
            None => {
                error!(self.log, "Trying to render on non-existent backend {}", dev_id);
                return;
            }
        };
        // setup two iterators on the stack, one over all surfaces for this backend, and
        // one containing only the one given as argument.
        // They make a trait-object to dynamically choose between the two
        let surfaces = backend.surfaces.borrow();
        let mut surfaces_iter = surfaces.iter();
        let mut option_iter = crtc
            .iter()
            .flat_map(|crtc| surfaces.get(&crtc).map(|surface| (crtc, surface)));
        let to_render_iter: &mut dyn Iterator<Item = (&CrtcHandle, &Rc<RefCell<SurfaceData>>)> =
            if crtc.is_some() {
                &mut option_iter
            } else {
                &mut surfaces_iter
            };
        for (&crtc, surface) in to_render_iter {
            // TODO get scale from the rendersurface when supporting HiDPI
            let frame = self.backend.pointer_image
                .get_image(1 /*scale*/, self.start_time.elapsed().as_millis() as u32);
            let renderer = &mut *backend.renderer.borrow_mut();
            let pointer_images = &mut backend.pointer_images;
            let pointer_image = pointer_images.iter()
                .find_map(|(image, texture)| if image == &frame { Some(texture) } else { None })
                .cloned().unwrap_or_else(|| {
                    let image =
                        ImageBuffer::from_raw(frame.width, frame.height, &*frame.pixels_rgba).unwrap();
                    let texture = import_bitmap(renderer, &image).expect("Failed to import cursor bitmap");
                    pointer_images.push((frame, texture.clone()));
                    texture
                });

            let result = self.render_surface_udev(
                &mut *surface.borrow_mut(), renderer, backend.dev_id, crtc
            );

            match result {
                Ok(()) => {
                    // TODO: only send drawn windows the frames callback
                    // Send frame events so that client start drawing their next frame
                    self.compositor.borrow()
                        .send_frames(self.start_time.elapsed().as_millis() as u32);
                },
                Err(err) => {
                    warn!(self.log, "Error during rendering: {:?}", err);
                    let reschedule = match err {
                        SwapBuffersError::AlreadySwapped => false,
                        SwapBuffersError::TemporaryFailure(err) => !matches!(
                            err.downcast_ref::<DrmError>(),
                            Some(&DrmError::DeviceInactive)
                                | Some(&DrmError::Access {
                                    source: SystemError::PermissionDenied,
                                    ..
                                })
                        ),
                        SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
                    };
                    if reschedule {
                        debug!(self.log, "Rescheduling");
                        self.backend.timer.add_timeout(
                            Duration::from_millis(1000 /*a seconds*/ / 60 /*refresh rate*/),
                            (backend.dev_id, crtc),
                        );
                    }
                }
            }
        }
    }

    fn render_surface_udev (
        &self,
        surface:   &mut SurfaceData,
        renderer:  &mut Gles2Renderer,
        device_id: dev_t,
        crtc:      CrtcHandle,
    ) -> Result<(), SwapBuffersError> {

        surface.surface.frame_submitted()?;

        let output = self.compositor.borrow()
            .find(|o| o.userdata().get::<UdevOutputId>() == Some(&UdevOutputId { device_id, crtc }))
            .map(|output| (output.geometry(), output.scale(), output.current_mode()));

        if output.is_none() {
            // Somehow we got called with a non existing output
            return Ok(())
        }

        let (geometry, scale, mode) = output.unwrap();

        let dmabuf = surface.surface.next_buffer()?;

        renderer.bind(dmabuf)?;

        let workspace = self.workspace.borrow();

        // and draw to our buffer
        let result = renderer.render(mode.size, Transform::Flipped180, |renderer, frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0])?;
            // draw the surfaces
            self.compositor.borrow().draw(&mut renderer, &mut frame, &workspace)?;
            // set cursor
            let location = self.controller.pointer_location;
            if geometry.to_f64().contains(location) {
                let (x, y) = location.into();
                let location: Point<i32, Logical> = (x as i32, y as i32).into() - geometry.loc;
                self.controller.draw_dnd_icon(renderer, frame, scale, location)?;
                // draw the cursor as relevant
                {
                    // reset the cursor if the surface is no longer alive
                    let mut reset = false;
                    let cursor_status = self.controller.cursor_status.lock()?;
                    if let CursorImageStatus::Image(ref surface) = *cursor_status {
                        reset = !surface.as_ref().is_alive();
                    }
                    if reset {
                        *cursor_status = CursorImageStatus::Default;
                    }

                    if let CursorImageStatus::Image(ref wl_surface) = *cursor_status {
                        self.controller.draw_cursor(renderer, frame, scale, location)?;
                    } else {
                        frame.render_texture_at(
                            self.controller.pointer_image,
                            location.to_f64().to_physical(scale as f64).to_i32_round(),
                            1,
                            scale as f64,
                            Transform::Normal,
                            1.0,
                        )?;
                    }
                }
            }
            Ok(())
        }).map_err(Into::<SwapBuffersError>::into)?;

        surface.surface.queue_buffer().map_err(Into::<SwapBuffersError>::into)

    }
}
