use crate::prelude::*;
use crate::app::{App, Backend};
use std::collections::HashMap;
use std::io::{Read, Error as IoError};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::drm::{DrmDevice, DrmEvent, DrmError, GbmBufferedSurface};
use smithay::backend::session::{Session, Signal as SessionSignal, auto::AutoSession};
use smithay::backend::udev::{UdevBackend, UdevEvent, primary_gpu};
use smithay::backend::renderer::Bind;
use smithay::reexports::calloop::{Dispatcher, RegistrationToken, timer::{Timer, TimerHandle}};
use smithay::reexports::drm::control::{crtc, Device};
use smithay::reexports::gbm::Device as GbmDevice;
use smithay::reexports::nix::{fcntl::OFlag, sys::stat::dev_t};
use smithay::reexports::input::Libinput;
use smithay::utils::signaling::{Linkable, Signaler, SignalToken};
use xcursor::{CursorTheme, parser::{parse_xcursor, Image}};

pub struct Udev<'a> {
    pub session:   AutoSession,
    events:        &'a EventLoop<'static, App<Self>>,
    primary_gpu:   Option<PathBuf>,
    backends:      HashMap<dev_t, UdevData<'a>>,
    signaler:      Signaler<SessionSignal>,
    pointer_image: Cursor,
    render_timer:  TimerHandle<(u64, crtc::Handle)>,
    input_backend: LibinputInputBackend
}

impl<'a> Backend for Udev<'a> {

    type Render = UdevBackend;

    type Input  = LibinputInputBackend;

    fn init (
        log:     &Logger,
        display: &Rc<RefCell<Display>>,
        events:  &EventLoop<'static, App<Self>>
    ) -> Result<Self, Box<dyn Error>> where Self: Sized {
        let seat_name = String::from("seat");
        let name = display.borrow_mut().add_socket_auto().unwrap().into_string().unwrap();
        info!(log, "Listening on wayland socket"; "name" => name.clone());
        ::std::env::set_var("WAYLAND_DISPLAY", name);
        let (session, notifier) = AutoSession::new(log.clone()).unwrap();
        let session_signal = notifier.signaler();
        let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<AutoSession>>(
            session.clone().into(),
        );
        libinput_context.udev_assign_seat(&seat_name).unwrap();
        let mut libinput_backend = LibinputInputBackend::new(libinput_context, log.clone());
        libinput_backend.link(session_signal);
        let timer = Timer::new().unwrap();
        let primary_gpu = primary_gpu(&session.seat()).unwrap_or_default();
        let backend = Self {
            events,
            session,
            primary_gpu,
            backends:      HashMap::new(),
            signaler:      session_signal.clone(),
            pointer_image: Cursor::load(&log),
            render_timer:  timer.handle(),
            input_backend: libinput_backend
        };
        let libinput_event_source = events.handle().insert_source(
            libinput_backend,
            move |event, _, app| app.controller.process_input_event(event)
        ).unwrap();
        let session_event_source = events.handle().insert_source(
            notifier,
            |(), &mut (), _| {}
        ).unwrap();
        let mut formats = Vec::new();
        for backend in backend.backends.values() {
            formats.extend(backend.renderer.borrow().dmabuf_formats().cloned());
        }
        init_dmabuf_global(
            &mut *display.borrow_mut(),
            formats,
            |buffer, mut ddata| {
                for backend in ddata.get::<App<Self>>().unwrap().backend.backends.values() {
                    if backend.renderer.borrow_mut().import_dmabuf(buffer).is_ok() {
                        return true;
                    }
                }
                false
            },
            log.clone(),
        );
        let udev_backend = UdevBackend::new(seat_name, log.clone())?;
        let udev_event_source = events.handle()
            .insert_source(udev_backend, move |event, _, app| match event {
                UdevEvent::Added { device_id, path } => app.device_added(device_id, path),
                UdevEvent::Changed { device_id } => app.device_changed(device_id),
                UdevEvent::Removed { device_id } => app.device_removed(device_id),
            })
            .map_err(|e| -> IoError { e.into() })
            .unwrap();
        Ok(backend)
    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        unimplemented!();
    }

    fn input (&self) -> Self::Input {
        self.input_backend
    }

    fn input_dispatched (&self, state: &mut App<Self>) -> bool {
        self.events
            .dispatch(Some(Duration::from_millis(16)), state)
            .is_ok()
    }

    fn draw (&self, app: &App<Self>, elapsed: u32) {}

}

#[derive(Clone)]
pub struct SessionFd(RawFd);

impl AsRawFd for SessionFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

pub type RenderSurface = GbmBufferedSurface<SessionFd>;

struct SurfaceData {
    surface: RenderSurface,
    #[cfg(feature = "debug")]
    fps: fps_ticker::Fps,
}

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("data/cursor.rgba");

pub struct Cursor {
    icons: Vec<Image>,
    size: u32,
}

impl Cursor {
    pub fn load(log: &::slog::Logger) -> Cursor {
        let name = std::env::var("XCURSOR_THEME")
            .ok()
            .unwrap_or_else(|| "default".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);

        let theme = CursorTheme::load(&name);
        let icons = load_icon(&theme)
            .map_err(|err| slog::warn!(log, "Unable to load xcursor: {}, using fallback cursor", err))
            .unwrap_or_else(|_| {
                vec![Image {
                    size: 32,
                    width: 64,
                    height: 64,
                    xhot: 1,
                    yhot: 1,
                    delay: 1,
                    pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
                    pixels_argb: vec![], //unused
                }]
            });

        Cursor { icons, size }
    }

    pub fn get_image(&self, scale: u32, millis: u32) -> Image {
        let size = self.size * scale;
        frame(millis, size, &self.icons)
    }
}

fn nearest_images(size: u32, images: &[Image]) -> impl Iterator<Item = &Image> {
    // Follow the nominal size of the cursor to choose the nearest
    let nearest_image = images
        .iter()
        .min_by_key(|image| (size as i32 - image.size as i32).abs())
        .unwrap();

    images
        .iter()
        .filter(move |image| image.width == nearest_image.width && image.height == nearest_image.height)
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> Image {
    let total = nearest_images(size, images).fold(0, |acc, image| acc + image.delay);
    millis %= total;

    for img in nearest_images(size, images) {
        if millis < img.delay {
            return img.clone();
        }
        millis -= img.delay;
    }

    unreachable!()
}

#[derive(thiserror::Error, Debug)]
enum CursorError {
    #[error("Theme has no default cursor")]
    NoDefaultCursor,
    #[error("Error opening xcursor file: {0}")]
    File(#[from] std::io::Error),
    #[error("Failed to parse XCursor file")]
    Parse,
}

fn load_icon(theme: &CursorTheme) -> Result<Vec<Image>, CursorError> {
    let icon_path = theme.load_icon("default").ok_or(CursorError::NoDefaultCursor)?;
    let mut cursor_file = std::fs::File::open(&icon_path)?;
    let mut cursor_data = Vec::new();
    cursor_file.read_to_end(&mut cursor_data)?;
    parse_xcursor(&cursor_data).ok_or(CursorError::Parse)
}

impl<'a> App<Udev<'a>> {

    fn device_added(&mut self, device_id: dev_t, path: PathBuf) {
        // Try to open the device
        if let Some((mut device, gbm)) = self.backend.session
            .open(&path, OFlag::O_RDWR | OFlag::O_CLOEXEC | OFlag::O_NOCTTY | OFlag::O_NONBLOCK)
            .ok().and_then(|fd| {
                match {
                    let fd = SessionFd(fd);
                    (
                        DrmDevice::new(fd.clone(), true, self.log.clone()),
                        GbmDevice::new(fd),
                    )
                } {
                    (Ok(drm), Ok(gbm)) => Some((drm, gbm)),
                    (Err(err), _) => {
                        warn!(
                            self.log,
                            "Skipping device {:?}, because of drm error: {}", device_id, err
                        );
                        None
                    }
                    (_, Err(err)) => {
                        // TODO try DumbBuffer allocator in this case
                        warn!(
                            self.log,
                            "Skipping device {:?}, because of gbm error: {}", device_id, err
                        );
                        None
                    }
                }
            })
        {
            let egl = match EGLDisplay::new(&gbm, self.log.clone()) {
                Ok(display) => display,
                Err(err) => {
                    warn!(
                        self.log,
                        "Skipping device {:?}, because of egl display error: {}", device_id, err
                    );
                    return;
                }
            };

            let context = match EGLContext::new(&egl, self.log.clone()) {
                Ok(context) => context,
                Err(err) => {
                    warn!(
                        self.log,
                        "Skipping device {:?}, because of egl context error: {}", device_id, err
                    );
                    return;
                }
            };

            let renderer = Rc::new(RefCell::new(unsafe {
                Gles2Renderer::new(context, self.log.clone()).unwrap()
            }));

            if path.canonicalize().ok() == self.backend.primary_gpu {
                info!(self.log, "Initializing EGL Hardware Acceleration via {:?}", path);
                if renderer.borrow_mut().bind_wl_display(&*self.display.borrow()).is_ok() {
                    info!(self.log, "EGL hardware-acceleration enabled");
                }
            }
            let backends = Rc::new(RefCell::new(scan_connectors(
                &mut device,
                &gbm,
                &mut *renderer.borrow_mut(),
                &mut *self.compositor.output_map.borrow_mut(),
                &self.backend.signaler,
                &self.log,
            )));
            let dev_id = device.device_id();
            let handle = self.handle.clone();
            let restart_token = self.backend.signaler.register(move |signal| match signal {
                SessionSignal::ActivateSession | SessionSignal::ActivateDevice { .. } => {
                    handle.insert_idle(move |anvil_state| anvil_state.render(dev_id, None));
                }
                _ => {}
            });
            device.link(self.backend.signaler.clone());
            let event_dispatcher = Dispatcher::new(
                device,
                move |event, _, anvil_state: &mut AnvilState<_>| match event {
                    DrmEvent::VBlank(crtc) => anvil_state.render(dev_id, Some(crtc)),
                    DrmEvent::Error(error) => {
                        error!(anvil_state.log, "{:?}", error);
                    }
                },
            );
            let registration_token = self.handle.register_dispatcher(event_dispatcher.clone()).unwrap();
            trace!(self.log, "Backends: {:?}", backends.borrow().keys());
            for backend in backends.borrow_mut().values() {
                // render first frame
                trace!(self.log, "Scheduling frame");
                schedule_initial_render(backend.clone(), renderer.clone(), &self.handle, self.log.clone());
            }
            self.backend.backends.insert(dev_id, UdevData {
                _restart_token: restart_token,
                dev_id,
                event_dispatcher,
                gbm,
                pointer_images: Vec::new(),
                registration_token,
                renderer,
                surfaces: backends,
            });
        }
    }

    fn device_changed(&mut self, device: dev_t) {
        //quick and dirty, just re-init all backends
        if let Some(ref mut backend) = self.backend.backends.get_mut(&device) {
            let logger = self.log.clone();
            let loop_handle = self.handle.clone();
            let signaler = self.backend.signaler.clone();

            self.compositor.output_map.borrow_mut().retain(|output| {
                output
                    .userdata()
                    .get::<UdevOutputId>()
                    .map(|id| id.device_id != device)
                    .unwrap_or(true)
            });

            let mut source = backend.event_dispatcher.as_source_mut();
            let mut backends = backend.surfaces.borrow_mut();
            *backends = scan_connectors(
                &mut *source,
                &backend.gbm,
                &mut *backend.renderer.borrow_mut(),
                &mut *self.compositor.output_map.borrow_mut(),
                &signaler,
                &logger,
            );

            for renderer in backends.values() {
                let logger = logger.clone();
                // render first frame
                schedule_initial_render(
                    renderer.clone(),
                    backend.renderer.clone(),
                    &loop_handle,
                    logger,
                );
            }
        }
    }

    fn device_removed(&mut self, device: dev_t) {
        // drop the backends on this side
        if let Some(backend) = self.backend.backends.remove(&device) {
            // drop surfaces
            backend.surfaces.borrow_mut().clear();
            debug!(self.log, "Surfaces dropped");
            self.compositor.output_map.borrow_mut().retain(|output| {
                output.userdata().get::<UdevOutputId>()
                    .map(|id| id.device_id != device)
                    .unwrap_or(true)
            });
            let _device = self.handle.remove(backend.registration_token);
            let _device = backend.event_dispatcher.into_source_inner();
            // don't use hardware acceleration anymore, if this was the primary gpu
            #[cfg(feature = "egl")]
            if _device.dev_path().and_then(|path| path.canonicalize().ok()) == self.backend.primary_gpu {
                backend.renderer.borrow_mut().unbind_wl_display();
            }
            debug!(self.log, "Dropping device");
        }
    }

    // If crtc is `Some()`, render it, else render all crtcs
    fn render (
        &mut self,
        dev_id: u64,
        crtc:   Option<crtc::Handle>
    ) -> Result<(), Box<dyn Error>> {
        let device_backend = match self.backend.backends.get_mut(&dev_id) {
            Some(backend) => backend,
            None => {
                error!(self.log, "Trying to render on non-existent backend {}", dev_id);
                return Ok(());
            }
        };
        // setup two iterators on the stack, one over all surfaces for this backend, and
        // one containing only the one given as argument.
        // They make a trait-object to dynamically choose between the two
        let surfaces = device_backend.surfaces.borrow();
        let mut surfaces_iter = surfaces.iter();
        let mut option_iter = crtc.iter()
            .flat_map(|crtc| surfaces.get(&crtc).map(|surface| (crtc, surface)));
        let to_render_iter: &mut dyn Iterator<Item = (&crtc::Handle, &Rc<RefCell<SurfaceData>>)> =
            if crtc.is_some() { &mut option_iter } else { &mut surfaces_iter };
        for (&crtc, surface) in to_render_iter {
            // TODO get scale from the rendersurface when supporting HiDPI
            let frame = self.backend.pointer_image.get_image(1 /*scale*/, self.elapsed());
            let renderer = &mut *device_backend.renderer.borrow_mut();
            let pointer_images = &mut device_backend.pointer_images;
            let pointer_image = pointer_images.iter()
                .find_map(|(image, texture)| if image == &frame { Some(texture) } else { None })
                .cloned().unwrap_or_else(|| {
                    let image = ImageBuffer::from_raw(
                        frame.width, frame.height, &*frame.pixels_rgba
                    ).unwrap();
                    let texture = import_bitmap(
                        renderer, &image
                    ).expect("Failed to import cursor bitmap");
                    pointer_images.push((frame, texture.clone()));
                    texture
                });
                let result = {
                    surface.borrow().surface.frame_submitted()?;
                    let output = self.compositor.output_map.borrow()
                        .find(|o| o.userdata().get::<UdevOutputId>() == Some(&UdevOutputId {
                            device_id: dev_id,
                            crtc
                        }))
                        .map(|output| (output.geometry(), output.scale(), output.current_mode()));
                    let (output_geometry, output_scale, mode) = if let Some((geometry, scale, mode)) = output {
                        (geometry, scale, mode)
                    } else {
                        // Somehow we got called with a non existing output
                        return Ok(());
                    };
                    let dmabuf = surface.borrow().surface.next_buffer()?;
                    renderer.bind(dmabuf)?;
                    // and draw to our buffer
                    match renderer.render(mode.size, Transform::Flipped180, |renderer, frame| {
                        frame.clear([0.8, 0.8, 0.9, 1.0])?;
                        // draw the surfaces
                        self.compositor.draw(renderer, frame, &self.workspace)?;
                        // set cursor
                        if output_geometry.to_f64().contains(pointer_location) {
                            let (ptr_x, ptr_y) = pointer_location.into();
                            let relative_ptr_location =
                                Point::<i32, Logical>::from((ptr_x as i32, ptr_y as i32)) - output_geometry.loc;
                            // draw the dnd icon if applicable
                            {
                                if let Some(ref wl_surface) = dnd_icon.as_ref() {
                                    if wl_surface.as_ref().is_alive() {
                                        draw_dnd_icon(
                                            renderer,
                                            frame,
                                            wl_surface,
                                            relative_ptr_location,
                                            output_scale,
                                            logger,
                                        )?;
                                    }
                                }
                            }
                            // draw the cursor as relevant
                            {
                                // reset the cursor if the surface is no longer alive
                                let mut reset = false;
                                if let CursorImageStatus::Image(ref surface) = *cursor_status {
                                    reset = !surface.as_ref().is_alive();
                                }
                                if reset {
                                    *cursor_status = CursorImageStatus::Default;
                                }

                                if let CursorImageStatus::Image(ref wl_surface) = *cursor_status {
                                    draw_cursor(
                                        renderer,
                                        frame,
                                        wl_surface,
                                        relative_ptr_location,
                                        output_scale,
                                        logger,
                                    )?;
                                } else {
                                }
                            }
                        }
                        Ok(())
                    })
                        .map_err(Into::<SwapBuffersError>::into)
                        .and_then(|x| x)
                        .map_err(Into::<SwapBuffersError>::into)
                    {
                        Ok(()) => surface.borrow().surface.queue_buffer().map_err(Into::<SwapBuffersError>::into),
                        Err(err) => Err(err),
                    }
                };

            if let Err(err) = result {
                warn!(self.log, "Error during rendering: {:?}", err);
                let reschedule = match err {
                    SwapBuffersError::AlreadySwapped => false,
                    SwapBuffersError::TemporaryFailure(err) => !matches!(
                        err.downcast_ref::<DrmError>(),
                        Some(&DrmError::DeviceInactive)
                            | Some(&DrmError::Access {
                                source: drm::SystemError::PermissionDenied,
                                ..
                            })
                    ),
                    SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
                };

                if reschedule {
                    debug!(self.log, "Rescheduling");
                    self.backend.render_timer.add_timeout(
                        Duration::from_millis(1000 /*a seconds*/ / 60 /*refresh rate*/),
                        (device_backend.dev_id, crtc),
                    );
                }
            } else {
                // TODO: only send drawn windows the frames callback
                // Send frame events so that client start drawing their next frame
                self.compositor.window_map.borrow().send_frames(self.elapsed());
            }
        }
        Ok(())
    }
}

pub struct UdevData<'a> {
    _restart_token:     SignalToken,
    dev_id:             u64,
    event_dispatcher:   Dispatcher<'static, DrmDevice<SessionFd>, App<Udev<'a>>>,
    gbm:                GbmDevice<SessionFd>,
    pointer_images:     Vec<(xcursor::parser::Image, Gles2Texture)>,
    registration_token: RegistrationToken,
    renderer:           Rc<RefCell<Gles2Renderer>>,
    surfaces:           Rc<RefCell<HashMap<crtc::Handle, Rc<RefCell<SurfaceData>>>>>,
}

#[derive(Debug, PartialEq)]
struct UdevOutputId {
    device_id: dev_t,
    crtc: crtc::Handle,
}

fn scan_connectors(
    device: &mut DrmDevice<SessionFd>,
    gbm: &GbmDevice<SessionFd>,
    renderer: &mut Gles2Renderer,
    output_map: &mut crate::output_map::OutputMap,
    signaler: &Signaler<SessionSignal>,
    logger: &::slog::Logger,
) -> HashMap<crtc::Handle, Rc<RefCell<SurfaceData>>> {
    // Get a set of all modesetting resource handles (excluding planes):
    let res_handles = device.resource_handles().unwrap();

    // Use first connected connector
    let connector_infos: Vec<ConnectorInfo> = res_handles
        .connectors()
        .iter()
        .map(|conn| device.get_connector(*conn).unwrap())
        .filter(|conn| conn.state() == ConnectorState::Connected)
        .inspect(|conn| info!(logger, "Connected: {:?}", conn.interface()))
        .collect();

    let mut backends = HashMap::new();

    // very naive way of finding good crtc/encoder/connector combinations. This problem is np-complete
    for connector_info in connector_infos {
        let encoder_infos = connector_info
            .encoders()
            .iter()
            .filter_map(|e| *e)
            .flat_map(|encoder_handle| device.get_encoder(encoder_handle))
            .collect::<Vec<EncoderInfo>>();
        'outer: for encoder_info in encoder_infos {
            for crtc in res_handles.filter_crtcs(encoder_info.possible_crtcs()) {
                if let Entry::Vacant(entry) = backends.entry(crtc) {
                    info!(
                        logger,
                        "Trying to setup connector {:?}-{} with crtc {:?}",
                        connector_info.interface(),
                        connector_info.interface_id(),
                        crtc,
                    );
                    let mut surface = match device.create_surface(
                        crtc,
                        connector_info.modes()[0],
                        &[connector_info.handle()],
                    ) {
                        Ok(surface) => surface,
                        Err(err) => {
                            warn!(logger, "Failed to create drm surface: {}", err);
                            continue;
                        }
                    };
                    surface.link(signaler.clone());

                    let renderer_formats =
                        Bind::<Dmabuf>::supported_formats(renderer).expect("Dmabuf renderer without formats");

                    let gbm_surface =
                        match GbmBufferedSurface::new(surface, gbm.clone(), renderer_formats, logger.clone())
                        {
                            Ok(renderer) => renderer,
                            Err(err) => {
                                warn!(logger, "Failed to create rendering surface: {}", err);
                                continue;
                            }
                        };

                    let mode = connector_info.modes()[0];
                    let size = mode.size();
                    let mode = Mode {
                        size: (size.0 as i32, size.1 as i32).into(),
                        refresh: (mode.vrefresh() * 1000) as i32,
                    };

                    let other_short_name;
                    let interface_short_name = match connector_info.interface() {
                        drm::control::connector::Interface::DVII => "DVI-I",
                        drm::control::connector::Interface::DVID => "DVI-D",
                        drm::control::connector::Interface::DVIA => "DVI-A",
                        drm::control::connector::Interface::SVideo => "S-VIDEO",
                        drm::control::connector::Interface::DisplayPort => "DP",
                        drm::control::connector::Interface::HDMIA => "HDMI-A",
                        drm::control::connector::Interface::HDMIB => "HDMI-B",
                        drm::control::connector::Interface::EmbeddedDisplayPort => "eDP",
                        other => {
                            other_short_name = format!("{:?}", other);
                            &other_short_name
                        }
                    };

                    let output_name = format!("{}-{}", interface_short_name, connector_info.interface_id());

                    let (phys_w, phys_h) = connector_info.size().unwrap_or((0, 0));
                    let output = output_map.add(
                        &output_name,
                        PhysicalProperties {
                            size: (phys_w as i32, phys_h as i32).into(),
                            subpixel: wl_output::Subpixel::Unknown,
                            make: "Smithay".into(),
                            model: "Generic DRM".into(),
                        },
                        mode,
                    );

                    output.userdata().insert_if_missing(|| UdevOutputId {
                        crtc,
                        device_id: device.device_id(),
                    });

                    entry.insert(Rc::new(RefCell::new(SurfaceData {
                        surface: gbm_surface,
                        #[cfg(feature = "debug")]
                        fps: fps_ticker::Fps::default(),
                    })));
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
        initial_render(&mut surface.surface, &mut *renderer)
    };
    if let Err(err) = result {
        match err {
            SwapBuffersError::AlreadySwapped => {}
            SwapBuffersError::TemporaryFailure(err) => {
                // TODO dont reschedule after 3(?) retries
                warn!(logger, "Failed to submit page_flip: {}", err);
                let handle = evt_handle.clone();
                evt_handle.insert_idle(move |_| schedule_initial_render(surface, renderer, &handle, logger));
            }
            SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
        }
    }
}

fn initial_render (
    surface:  &mut RenderSurface,
    renderer: &mut Gles2Renderer
) -> Result<(), SwapBuffersError> {
    let dmabuf = surface.next_buffer()?;
    renderer.bind(dmabuf)?;
    // Does not matter if we render an empty frame
    renderer.render((1, 1).into(), Transform::Normal, |_, frame| {
        frame.clear([0.8, 0.8, 0.9, 1.0]).map_err(Into::<SwapBuffersError>::into)
    })
        .map_err(Into::<SwapBuffersError>::into)
        .and_then(|x| x.map_err(Into::<SwapBuffersError>::into))?;
    surface.queue_buffer()?;
    Ok(())
}
