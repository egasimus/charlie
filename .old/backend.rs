use crate::prelude::*;
use crate::app::App;
use crate::compositor::Compositor;
use crate::controller::{Controller, Cursor};

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        drm::{DrmDevice, GbmBufferedSurface, DrmEvent, DrmError},
        egl::{EGLContext, EGLDisplay},
        winit::{self, WinitGraphicsBackend, WinitInputBackend},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        session::{Session, Signal as SessionSignal, auto::{AutoSession, AutoSessionNotifier}},
        udev::{UdevBackend, UdevEvent, primary_gpu},
        renderer::{Bind}
    },
    reexports::{
        calloop::{Dispatcher, RegistrationToken, timer::{Timer, TimerHandle}},
        drm::{
            SystemError,
            control::{
                Device,
                crtc::Handle as CrtcHandle,
                connector::{Info as ConnectorInfo, State as ConnectorState, Interface},
                encoder::{Info as EncoderInfo}
            }
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

pub trait Engine: Sized {
    type Renderer;
    type DrawState;
    type Input;

    fn display (&self) -> &Rc<RefCell<Display>>;

    fn log (&self) -> &Logger;

    fn logger (&self) -> Logger {
        self.log().clone()
    }

    fn event_loop (&self) -> &Rc<RefCell<EventLoop<'static, App<Self>>>>;

    fn event_handle (&self) -> LoopHandle<'static, App<Self>> {
        self.event_loop().borrow().handle()
    }

    fn event_dispatch (
        &self, duration: Option<Duration>, data: &mut App<Self>
    ) -> Result<(), IOError> {
        self.event_loop().borrow_mut().dispatch(duration, data)
    }

    fn init (log: &Logger) -> Result<Self, Box<dyn Error>>;

    fn run (mut self, configure: impl Fn(App<Self>)->App<Self>)
        -> Result<(), Box<dyn Error>>
    {
        let app = App::init(&self)?;
        let mut app = configure(app);
        let socket_name = self.display().borrow_mut()
            .add_socket_auto().unwrap()
            .into_string().unwrap();
        info!(self.log(), "Listening on wayland socket"; "name" => socket_name.clone());
        ::std::env::set_var("WAYLAND_DISPLAY", &socket_name);
        app.x11_start();
        app.start_time = Instant::now();
        info!(self.log(), "Initialization completed, starting the main loop.");
        while app.running() {
            if self.dispatch_input(&mut app.controller).is_err() {
                app.stop()
            } else {
                self.tick(&mut app)?;
                self.flush(&mut app);
                app.refresh();
            }
        }
        app.compositor.borrow().clear();
        self.clear();
        Ok(())
    }

    fn draw_or_stop (&self, app: &mut App<Self>, state: Self::DrawState) {
        if let Err(SwapBuffersError::ContextLost(err)) = self.draw(app, state) {
            error!(self.log(), "Critical Rendering Error: {}", err);
            app.stop();
        } else {
            app.send_elapsed();
        }
    }

    fn draw (&self, app: &mut App<Self>, state: Self::DrawState) -> Result<(), SwapBuffersError>;

    fn flush (&self, app: &mut App<Self>) {
        self.display().borrow_mut().flush_clients(app);
    }

    fn clear (&mut self) {
        unimplemented!();
    }

    fn renderer (&self) -> &mut Gles2Renderer {
        unimplemented!()
    }

    fn tick (&self, app: &mut App<Self>) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn load_texture (&self, path: impl AsRef<Path>) -> Result<Gles2Texture, Box<dyn Error>> {
        import_bitmap(self.renderer(), &image::io::Reader::open(path)?
            .with_guessed_format().unwrap()
            .decode().unwrap()
            .to_rgba8()
        ).map_err(Into::<Box<dyn Error>>::into)
    }

    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }

    fn add_output (&self, app: &mut App<Self>, name: impl AsRef<str>) {
        unimplemented!()
    }
}

pub struct Winit {
    log:        Logger,
    display:    Rc<RefCell<Display>>,
    event_loop: Rc<RefCell<EventLoop<'static, App<Self>>>>,

    renderer:   Rc<RefCell<<Self as Engine>::Renderer>>,
    input:      <Self as Engine>::Input,
}

impl Engine for Winit {
    type Renderer  = WinitGraphicsBackend;
    type DrawState = ();
    type Input     = WinitInputBackend;

    fn log (&self) -> &Logger {
        &self.log
    }

    fn display (&self) -> &Rc<RefCell<Display>> {
        &self.display
    }

    fn event_loop (&self) -> &Rc<RefCell<EventLoop<'static, App<Self>>>> {
        &self.event_loop
    }

    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        let (renderer, input) = winit::init(log.clone())?;
        let mut backend = Self {
            log:        log.clone(),
            display:    Rc::new(RefCell::new(Display::new())),
            event_loop: Rc::new(RefCell::new(EventLoop::try_new()?)),
            renderer:   Rc::new(RefCell::new(renderer)),
            input,
        };
        let log = backend.logger();
        backend.event_handle().insert_source(Generic::from_fd(
            backend.display().clone().borrow().get_poll_fd(),
            Interest::READ,
            CalloopMode::Level
        ), {
            let display = backend.display().clone();
            move |_, _, state: &mut App<Self>| {
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
            }
        })?;
        let display = backend.display().clone();
        backend.renderer().bind_wl_display(&display.clone().borrow())?;
        let renderer = backend.renderer.clone();
        init_dmabuf_global(
            &mut *display.borrow_mut(),
            backend.renderer().dmabuf_formats().cloned().collect::<Vec<_>>(),
            move |buffer, _| renderer.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
            backend.logger()
        );
        Ok(backend)
    }

    fn renderer (&self) -> &mut Gles2Renderer {
        self.renderer.borrow().renderer()
    }
    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        self.input
            .dispatch_new_events(|event| controller.process_input_event(event))
            .map_err(Into::<Box<dyn Error>>::into)
            //.map_err(|e|Box::new(e) as Box<dyn Error>)
    }
    fn tick (&self, app: &mut App<Self>) -> Result<(), Box<dyn Error>> {
        self.draw_or_stop(app, ());
        self.flush(app);
        self.event_dispatch(Some(Duration::from_millis(16)), app)?;
        Ok(())
    }
    fn draw (&self, app: &mut App<Self>, _: Self::DrawState) -> Result<(), SwapBuffersError> {
        let workspace  = app.workspace.borrow();
        let compositor = app.compositor.borrow();
        let controller = &app.controller;
        let result = self.renderer.borrow().render(|mut renderer, mut frame| {
            // This is safe to do as with winit we are guaranteed to have exactly one output
            frame.clear([0.8, 0.8, 0.8, 1.0])?;
            compositor.draw(&mut renderer, &mut frame, &workspace)?;
            controller.draw(&mut renderer, &mut frame, 1.0)?;
            Ok(())
        })?;
        self.renderer.borrow().window().set_cursor_visible(controller.cursor_visible.get());
        result
    }
    fn add_output (&self, app: &mut App<Self>, name: impl AsRef<str>) {
        app.compositor.borrow_mut().add_output(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode {
                size:    self.renderer.borrow().window_size().physical_size,
                refresh: 60_000
            }
        );
    }
}

pub struct Udev {
    log:           Logger,
    display:       Rc<RefCell<Display>>,
    event_loop:    Rc<RefCell<EventLoop<'static, App<Self>>>>,
    render_timer:  TimerHandle<(u64, CrtcHandle)>,
    session:       Rc<RefCell<AutoSession>>,
    signaler:      Signaler<SessionSignal>,
    primary_gpu:   Option<PathBuf>,
    instances:     Rc<RefCell<UdevInstances>>,
    pointer_image: Cursor,
    render_event_source:  Option<RegistrationToken>,
    session_event_source: Option<RegistrationToken>,
    input_event_source:   Option<RegistrationToken>,
    udev_event_source:    Option<RegistrationToken>,
}

type UdevInstances = HashMap<dev_t, UdevInstance>;

pub struct UdevInstance {
    //_restart_token:       SignalToken,
    surfaces:             Rc<RefCell<UdevInstanceSurfaces>>,
    pointer_images:       Vec<(xcursor::parser::Image, Gles2Texture)>,
    renderer:             Rc<RefCell<Gles2Renderer>>,
    gbm:                  GbmDevice<SessionFd>,
    drm_dispatcher_token: RegistrationToken,
    event_dispatcher:     Dispatcher<'static, DrmDevice<SessionFd>, App<Udev>>,
    dev_id: u64,
}

type UdevInstanceSurfaces = HashMap<CrtcHandle, Rc<RefCell<SurfaceData>>>;

impl Engine for Udev {
    type Renderer  = UdevBackend;
    type DrawState = (u64, Option<CrtcHandle>);
    type Input     = LibinputInputBackend;
    fn log (&self) -> &Logger {
        &self.log
    }
    fn display (&self) -> &Rc<RefCell<Display>> {
        &self.display
    }
    fn event_loop (&self) -> &Rc<RefCell<EventLoop<'static, App<Self>>>> {
        &self.event_loop
    }

    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        // Init timer
        let timer = Timer::new().unwrap();
        let render_timer = timer.handle();
        // Init session
        let (session, notifier) = AutoSession::new(log.clone())
            .expect("Could not initialize session.");
        let session_signal = notifier.signaler();
        let signaler = session_signal.clone();
        // Init input
        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<AutoSession>>(
            session.clone().into()
        );
        let seat_name = "udev-seat";
        libinput.udev_assign_seat(seat_name).unwrap();
        let mut input = LibinputInputBackend::new(libinput, log.clone());
        input.link(session_signal);
        // Init render
        let primary_gpu = primary_gpu(&session.seat()).unwrap_or_default();
        let event_loop  = EventLoop::try_new()?;

        let mut backend = Self {
            log:        log.clone(),
            display:    Rc::new(RefCell::new(Display::new())),
            event_loop: Rc::new(RefCell::new(event_loop)),

            session:              Rc::new(RefCell::new(session)),
            render_timer,
            signaler,
            primary_gpu,
            instances:            Rc::new(RefCell::new(HashMap::new())),
            pointer_image:        Cursor::load(&log),
            render_event_source:  None,
            input_event_source:   None,
            session_event_source: None,
            udev_event_source:    None,
        };

        backend.render_event_source = Some(backend.event_handle().insert_source(
            timer, |(dev_id, crtc), _, app: &mut App<Self>| {
                backend.draw_or_stop(app, (dev_id, Some(crtc)))
            }
        )?);

        backend.input_event_source = Some(backend.event_handle().insert_source(
            input, move |event, _, app| { app.controller.process_input_event(event) }
        )?);

        backend.session_event_source = Some(backend.event_handle().insert_source(
            notifier, move |(), &mut (), _app| { /*do nothing club*/ }
        )?);

        let udev = UdevBackend::new(seat_name, log.clone())?;

        for (dev, path) in udev.device_list() {
            backend.device_added(dev, path.into())?;
        }

        backend.udev_event_source = Some(backend.event_handle().insert_source(
            udev, move |event, _, app| {
                match event {
                    UdevEvent::Added { device_id, path } => backend.device_added(device_id, path),
                    _ => Ok(())
                    //UdevEvent::Changed { device_id } => state.device_changed(device_id),
                    //UdevEvent::Removed { device_id } => state.device_removed(device_id),
                }.unwrap();
            }
        )?);

        Ok(backend)
    }

    fn draw (
        &self,
        app: &mut App<Self>,
        (dev_id, crtc): Self::DrawState
    ) -> Result<(), SwapBuffersError> {
        let instance = match self.instances.borrow_mut().get_mut(&dev_id) {
            Some(backend) => backend,
            None => {
                error!(app.log, "Trying to render on non-existent backend {}", dev_id);
                return Ok(());
            }
        };
        // setup two iterators on the stack, one over all surfaces for this backend, and
        // one containing only the one given as argument.
        // They make a trait-object to dynamically choose between the two
        let to_render = if crtc.is_some() {
            &mut crtc.iter().flat_map(|crtc| {
                instance.surfaces.borrow().get(&crtc).map(|surface| (crtc, surface))
            }) as &mut dyn Iterator<Item=(&CrtcHandle, &Rc<RefCell<SurfaceData>>)>
        } else {
            &mut instance.surfaces.borrow().iter()
        };
        for (&crtc, surface) in to_render {

            let result = self.render_surface_udev(
                &mut app,
                &mut *surface.borrow_mut(),
                &mut *instance.renderer.borrow_mut(),
                instance,
                crtc
            );

            if let Err(err) = result {
                warn!(app.log, "Error during rendering: {:?}", err);
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
                    debug!(app.log, "Rescheduling");
                    self.render_timer.add_timeout(
                        Duration::from_millis(
                            1000 /*a second*/ / 60 /*refresh rate*/
                        ),
                        (instance.dev_id, crtc),
                    );
                }
            }

        }
        Ok(())
    }

    fn clear (&mut self) {
        self.event_handle().remove(self.session_event_source.take().unwrap());
        self.event_handle().remove(self.input_event_source.take().unwrap());
        self.event_handle().remove(self.udev_event_source.take().unwrap());
    }
}

impl Udev {

    fn device_added (
        &self,
        device_id: dev_t,
        path:      PathBuf
    ) -> Result<(), Box<dyn Error>> {
        let log = self.logger();
        // Try to open the device
        let fd = SessionFd(self.session.borrow_mut().open(
            &path, OFlag::O_RDWR | OFlag::O_CLOEXEC | OFlag::O_NOCTTY | OFlag::O_NONBLOCK
        )?);
        let mut drm = DrmDevice::new(fd.clone(), true, self.logger())?;
        let gbm = GbmDevice::new(fd)?;
        let egl = EGLDisplay::new(&gbm, self.logger())?;
        let context = EGLContext::new(&egl, self.logger())?;
        let renderer = unsafe { Gles2Renderer::new(context, self.logger()).unwrap() };
        let renderer = Rc::new(RefCell::new(renderer));
        if path.canonicalize().ok() == self.primary_gpu {
            info!(log, "Initializing EGL Hardware Acceleration via {:?}", path);
            if renderer.borrow_mut().bind_wl_display(&*self.display.borrow()).is_ok() {
                info!(log, "EGL hardware-acceleration enabled");
            }
        }
        let surfaces = self.scan(&mut drm, &gbm, &mut *renderer.borrow_mut(), None);
        let surfaces = Rc::new(RefCell::new(surfaces));
        let dev_id   = drm.device_id();
        let handle   = self.event_handle();
        let restart_token = self.signaler.clone().register(move |signal| match signal {
            SessionSignal::ActivateSession | SessionSignal::ActivateDevice { .. } => {
                handle.insert_idle(move |app| self.draw_or_stop(app, (dev_id, None)));
            }
            _ => {}
        });
        drm.link(self.signaler.clone());
        let log = self.logger();
        let event_dispatcher = Dispatcher::new(drm, move |event, _, app: &mut App<_>| match event {
            DrmEvent::VBlank(crtc) => { self.draw_or_stop(app, (dev_id, Some(crtc))); },
            DrmEvent::Error(error) => { error!(&log, "{:?}", error) }
        });
        let log = self.logger();
        let drm_dispatcher_token = self.event_handle()
            .register_dispatcher(event_dispatcher.clone()).unwrap();
        trace!(&log, "Backends: {:?}", surfaces.borrow().keys());
        for backend in surfaces.borrow_mut().values() {
            // render first frame
            trace!(&log, "Scheduling frame");
            Self::schedule_initial_render(
                backend.clone(), renderer.clone(), &self.event_handle(), self.logger()
            );
        }
        self.instances.borrow_mut().insert(dev_id, UdevInstance {
            dev_id,
            event_dispatcher,
            gbm,
            drm_dispatcher_token,
            renderer,
            pointer_images: Vec::new(),
            //_restart_token: restart_token,
            surfaces,
        });
        Ok(())
    }

    fn scan (
        &self,
        drm:        &mut DrmDevice<SessionFd>,
        gbm:        &GbmDevice<SessionFd>,
        renderer:   &mut Gles2Renderer,
        compositor: Option<Rc<RefCell<Compositor<Self>>>>
    ) -> HashMap<CrtcHandle, Rc<RefCell<SurfaceData>>> {
        let logger      = self.logger();
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

                        let mut surface = match surface {
                            Ok(surface) => surface,
                            Err(err) => {
                                warn!(logger, "Failed to create drm surface: {}", err);
                                continue;
                            }
                        };

                        surface.link(self.signaler.clone());

                        let surface = match GbmBufferedSurface::new(
                            surface,
                            gbm.clone(),
                            Bind::<Dmabuf>::supported_formats(renderer)
                                .expect("Dmabuf renderer without formats"),
                            logger.clone()
                        ) {
                            Ok(renderer) => renderer,
                            Err(err) => {
                                warn!(logger, "Failed to create rendering surface: {}", err);
                                continue;
                            }
                        };

                        let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
                        let mode = connector.modes()[0];
                        let size = mode.size();

                        //if let Some(compositor) = compositor {
                            //let output = compositor.borrow_mut().add_output(
                                //&Self::output_name(&connector),
                                //PhysicalProperties {
                                    //size: (phys_w as i32, phys_h as i32).into(),
                                    //subpixel: wl_output::Subpixel::Unknown,
                                    //make: "Smithay".into(),
                                    //model: "Generic DRM".into(),
                                //},
                                //OutputMode {
                                    //size: (size.0 as i32, size.1 as i32).into(),
                                    //refresh: (mode.vrefresh() * 1000) as i32,
                                //},
                            //);

                            //output.userdata().insert_if_missing(
                                //|| UdevOutputId { crtc, device_id: drm.device_id() }
                            //);
                        //}

                        entry.insert(Rc::new(RefCell::new(SurfaceData { surface })));

                        break 'outer;
                    }
                }
            }
        }
        backends
    }

    fn output_name (connector: &ConnectorInfo) -> String {
        let id = connector.interface_id();
        let name = match connector.interface() {
            Interface::DVII                => "DVI-I",
            Interface::DVID                => "DVI-D",
            Interface::DVIA                => "DVI-A",
            Interface::SVideo              => "S-VIDEO",
            Interface::DisplayPort         => "DP",
            Interface::EmbeddedDisplayPort => "eDP",
            Interface::HDMIA               => "HDMI-A",
            Interface::HDMIB               => "HDMI-B",
            other => format!("{other:?}").as_str()
        };
        format!("{name}-{id}")
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
        renderer.render((1, 1).into(), Transform::Normal, |_, frame: &mut Gles2Frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0]).map_err(Into::<SwapBuffersError>::into)
        })
            .map_err(Into::<SwapBuffersError>::into)
            .and_then(|x| x.map_err(Into::<SwapBuffersError>::into))?;
        surface.queue_buffer()?;
        Ok(())
    }

    fn render_surface_udev (
        &self,
        app:      &mut App<Self>,
        surface:  &mut SurfaceData,
        renderer: &mut Gles2Renderer,
        instance: &mut UdevInstance,
        crtc:     CrtcHandle,
    ) -> Result<(), SwapBuffersError> {
        surface.surface.frame_submitted()?;
        let device_id = instance.dev_id;
        if let Some((geometry, scale, mode)) = app.compositor.clone().borrow()
            .find(|o| o.userdata().get::<UdevOutputId>() == Some(&UdevOutputId { device_id, crtc }))
            .map(|output| (output.geometry(), output.scale(), output.current_mode()))
        {
            renderer.bind(surface.surface.next_buffer()?)?;
            renderer.render(mode.size, Transform::Flipped180, |mut renderer, mut frame| -> Result<(), Box<dyn Error>> {
                frame.clear([0.8, 0.8, 0.9, 1.0])?;
                // draw the surfaces
                app.compositor.borrow().draw(&mut renderer, &mut frame, &app.workspace.borrow())?;
                // set cursor
                let location = app.controller.pointer_location;
                if geometry.to_f64().contains(location) {
                    let (x, y) = location.into();
                    let location: Point<i32, Logical> = (x as i32, y as i32).into();
                    let location = location - geometry.loc;
                    app.controller.draw_dnd_icon(renderer, frame, scale, location)?;
                    // draw the cursor as relevant
                    {
                        // reset the cursor if the surface is no longer alive
                        let mut reset = false;
                        let mut cursor_status = app.controller.cursor_status.lock()?;
                        if let CursorImageStatus::Image(ref surface) = *cursor_status {
                            reset = !surface.as_ref().is_alive();
                        }
                        if reset {
                            *cursor_status = CursorImageStatus::Default;
                        }
                        // draw the cursor
                        if let CursorImageStatus::Image(ref wl_surface) = *cursor_status {
                            app.controller.draw_cursor(renderer, frame, scale, location)?;
                        } else {
                            let elapsed = app.start_time.elapsed().as_millis() as u32;
                            self.draw_cursor_fallback(
                                elapsed, renderer, frame, instance, scale, location
                            )?;
                        }
                    }
                }
                Ok(())
            })?;
            surface.surface.queue_buffer().map_err(Into::<SwapBuffersError>::into)
        } else {
            // Somehow we got called with a non existing output
            return Ok(())
        }
    }

    fn draw_cursor_fallback (
        &self,
        elapsed:  u32,
        renderer: &mut Gles2Renderer,
        frame:    &mut Gles2Frame,
        instance: &mut UdevInstance,
        scale:    f32,
        location: Point<i32, Logical>,
    ) -> Result<(), Box<dyn Error>> {
        // what the hell is all this
        // TODO get scale from the rendersurface when supporting HiDPI
        let pointer_0 = self.pointer_image.get_image(1, elapsed);
        let pointer_images = &mut instance.pointer_images;
        let pointer_image = pointer_images.iter().find_map(|(image, texture)| {
            if image == &pointer_0 {
                Some(texture)
            } else {
                None
            }
        }).cloned().unwrap_or_else(|| {
            let texture = import_bitmap(renderer,
                &ImageBuffer::from_raw(
                    pointer_0.width, pointer_0.height, &*pointer_0.pixels_rgba
                ).unwrap()
            ).expect("Failed to import cursor bitmap");
            pointer_images.push((pointer_0, texture.clone()));
            texture
        });
        frame.render_texture_at(
            &pointer_image,
            location.to_f64().to_physical(scale as f64).to_i32_round(),
            1,
            scale as f64,
            Transform::Normal,
            1.0,
        )?;
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
}
