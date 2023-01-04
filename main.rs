use prelude::*;

mod output;
use output::OutputMap;
mod prelude;
mod grab;
use grab::{MoveSurfaceGrab, ResizeSurfaceGrab, ResizeState, ResizeData};
mod surface;
use surface::{draw_surface_tree, SurfaceData, SurfaceKind};
mod popup;
use popup::PopupKind;
mod window;
use window::WindowMap;

fn main () -> Result<(), Box<dyn Error>> {
    let (log, _guard) = Charlie::init_log();
    let display = Rc::new(RefCell::new(Display::new()));
    let (renderer, input) = Charlie::init_io(&log, &display)?;
    let event_loop = EventLoop::try_new().unwrap();
    let mut charlie = Charlie::init(log, &display, &renderer, &event_loop)?;
    charlie.add_output(OUTPUT_NAME);
    Ok(charlie.run(&display, input, event_loop))
}

pub struct Charlie {
    pub log:              Logger,
    pub socket_name:      Option<String>,
    pub running:          Arc<AtomicBool>,
    pub renderer:         Rc<RefCell<WinitGraphicsBackend>>,
    pub window_map:       Rc<RefCell<WindowMap>>,
    pub output_map:       Rc<RefCell<OutputMap>>,
    pub dnd_icon:         Arc<Mutex<Option<WlSurface>>>,
    pub pointer:          PointerHandle,
    pub keyboard:         KeyboardHandle,
    pub suppressed_keys:  Vec<u32>,
    pub pointer_location: Point<f64, Logical>,
    pub seat:             Seat,
    pub cursor_status:    Arc<Mutex<CursorImageStatus>>,
}

impl Charlie {

    pub fn init (
        log: Logger,
        display:    &Rc<RefCell<Display>>,
        renderer:   &Rc<RefCell<WinitGraphicsBackend>>,
        event_loop: &EventLoop<'static, Self>
    ) -> Result<Charlie, Box<dyn Error>> {
        init_xdg_output_manager(&mut *display.borrow_mut(), log.clone());
        init_shm_global(&mut *display.borrow_mut(), vec![], log.clone());
        Self::init_loop(&log, &display, event_loop.handle());
        let socket_name = Self::init_socket(&log, &display, true);
        let dnd_icon = Arc::new(Mutex::new(None));
        Self::init_data_device(&log, &display, &dnd_icon);
        let (seat, pointer, cursor_status, keyboard) = Self::init_seat(&log, &display, "seat");
        let window_map = Rc::new(RefCell::new(WindowMap::default()));
        let output_map = Rc::new(RefCell::new(OutputMap::new(
            display.clone(), window_map.clone(), log.clone())));
        Self::init_compositor(&log, &display);
        Self::init_xdg_shell(&log, &display, &window_map, &output_map);
        Self::init_wl_shell(&log, &display, &window_map, &output_map);
        Ok(Charlie {
            cursor_status,
            dnd_icon,
            keyboard,
            log,
            output_map,
            pointer,
            pointer_location: (0.0, 0.0).into(),
            renderer: renderer.clone(),
            running: Arc::new(AtomicBool::new(true)),
            seat,
            socket_name,
            suppressed_keys: Vec::new(),
            window_map,
        })
    }

    fn init_log () -> (slog::Logger, GlobalLoggerGuard) {
        let fuse = slog_async::Async::default(slog_term::term_full().fuse()).fuse();
        let log = slog::Logger::root(fuse, o!());
        let guard = slog_scope::set_global_logger(log.clone());
        slog_stdlog::init().expect("Could not setup log backend");
        (log, guard)
    }

    fn init_io (log: &Logger, display: &Rc<RefCell<Display>>)
        -> Result<(Rc<RefCell<WinitGraphicsBackend>>, WinitInputBackend), winit::Error>
    {
        match winit::init(log.clone()) {
            Ok((mut renderer, mut input)) => {
                let renderer = Rc::new(RefCell::new(renderer));
                if renderer.borrow_mut().renderer().bind_wl_display(&display.borrow()).is_ok() {
                    info!(log, "EGL hardware-acceleration enabled");
                    let dmabuf_formats = renderer.borrow_mut().renderer().dmabuf_formats().cloned()
                        .collect::<Vec<_>>();
                    let renderer = renderer.clone();
                    init_dmabuf_global(
                        &mut *display.borrow_mut(),
                        dmabuf_formats,
                        move |buffer, _| renderer.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
                        log.clone()
                    );
                };
                Ok((renderer, input))
            },
            Err(err) => {
                slog::crit!(log, "Failed to initialize Winit backend: {}", err);
                Err(err)
            }
        }
    }

    fn init_loop (
        log:        &Logger,
        display:    &Rc<RefCell<Display>>,
        event_loop: LoopHandle<'static, Self>,
    ) {
        let log = log.clone();
        let display = display.clone();
        let same_display = display.clone();
        event_loop.insert_source( // init the wayland connection
            Generic::from_fd(display.borrow().get_poll_fd(), Interest::READ, CalloopMode::Level),
            move |_, _, state: &mut Charlie| {
                let mut display = same_display.borrow_mut();
                match display.dispatch(std::time::Duration::from_millis(0), state) {
                    Ok(_) => Ok(PostAction::Continue),
                    Err(e) => {
                        error!(log, "I/O error on the Wayland display: {}", e);
                        state.running.store(false, Ordering::SeqCst);
                        Err(e)
                    }
                }
            },
        ).expect("Failed to init the wayland event source.");
    }

    fn init_socket (
        log: &Logger, display: &Rc<RefCell<Display>>, listen_on_socket: bool
    ) -> Option<String> {
        if listen_on_socket {
            let socket_name =
                display.borrow_mut().add_socket_auto().unwrap().into_string().unwrap();
            info!(log, "Listening on wayland socket"; "name" => socket_name.clone());
            ::std::env::set_var("WAYLAND_DISPLAY", &socket_name);
            Some(socket_name)
        } else {
            None
        }
    }

    pub fn init_data_device (
        log: &Logger, display: &Rc<RefCell<Display>>, dnd_icon: &Arc<Mutex<Option<WlSurface>>>
    ) {
        let dnd_icon = dnd_icon.clone();
        init_data_device(
            &mut display.borrow_mut(),
            move |event| match event {
                DataDeviceEvent::DnDStarted { icon, .. } => {*dnd_icon.lock().unwrap() = icon;}
                DataDeviceEvent::DnDDropped => {*dnd_icon.lock().unwrap() = None;}
                _ => {}
            },
            default_action_chooser,
            log.clone(),
        );
    }

    pub fn init_seat (
        log: &Logger, display: &Rc<RefCell<Display>>, seat_name: &str
    ) -> (Seat, PointerHandle, Arc<Mutex<CursorImageStatus>>, KeyboardHandle) {
        let (mut seat, _) = Seat::new(&mut display.borrow_mut(), seat_name.to_string(), log.clone());
        let cursor_status = Arc::new(Mutex::new(CursorImageStatus::Default));
        let cursor_status2 = cursor_status.clone();
        let handler = move |new_status| { *cursor_status2.lock().unwrap() = new_status };
        let pointer = seat.add_pointer(handler);
        init_tablet_manager_global(&mut display.borrow_mut());
        let cursor_status3 = cursor_status.clone();
        seat.tablet_seat().on_cursor_surface(
            move |_tool, new_status|{*cursor_status3.lock().unwrap() = new_status}
        );
        let keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25, |seat, focus| {
            set_data_device_focus(seat, focus.and_then(|s| s.as_ref().client()))
        }).expect("Failed to initialize the keyboard");
        (seat, pointer, cursor_status, keyboard)
    }

    pub fn init_compositor (log: &Logger, display: &Rc<RefCell<Display>>) {
        compositor_init(
            &mut *display.borrow_mut(),
            |surface, mut data|data.get::<Charlie>().unwrap().window_map.as_ref().borrow_mut()
                .commit(&surface),
            log.clone()
        );
    }

    pub fn init_xdg_shell (
        log:        &Logger,
        display:    &Rc<RefCell<Display>>,
        window_map: &Rc<RefCell<WindowMap>>,
        output_map: &Rc<RefCell<OutputMap>>
    ) -> Arc<Mutex<XdgShellState>> {
        let window_map = window_map.clone();
        let output_map = output_map.clone();
        let (state, _, _) = xdg_shell_init(
            &mut *display.borrow_mut(),
            move |shell_event, _dispatch_data| match shell_event {
                XdgRequest::NewToplevel { surface }
                    => Self::xdg_new_toplevel(surface, &output_map, &window_map),
                XdgRequest::NewPopup { surface }
                    => Self::xdg_new_popup(surface, &window_map),
                XdgRequest::Move { surface, seat, serial, }
                    => Self::xdg_move(&surface, &output_map, &window_map, seat, serial),
                XdgRequest::Resize { surface, seat, serial, edges }
                    => Self::xdg_resize(&surface, &output_map, &window_map, seat, serial, edges),
                XdgRequest::AckConfigure { surface, configure: Configure::Toplevel(configure), .. }
                    => Self::xdg_ack_configure(&surface, &output_map, &window_map, configure),
                XdgRequest::Fullscreen { surface, output, .. }
                    => Self::xdg_fullscreen(&surface, &output_map, &window_map, output),
                XdgRequest::UnFullscreen { surface }
                    => Self::xdg_unfullscreen(&surface),
                XdgRequest::Maximize { surface }
                    => Self::xdg_maximize(&surface, &output_map, &window_map),
                XdgRequest::UnMaximize { surface }
                    => Self::xdg_unmaximize(&surface),
                _ => (),
            },
            log.clone());
        state
    }

    pub fn xdg_new_toplevel (
        surface:    ToplevelSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>
    ) {
        // place the window at a random location on the primary output
        // or if there is not output in a [0;800]x[0;800] square
        use rand::distributions::{Distribution, Uniform};
        let output_geometry = output_map
            .borrow().with_primary().map(|o| o.geometry())
            .unwrap_or_else(|| Rectangle::from_loc_and_size((0, 0), (800, 800)));
        let max_x = output_geometry.loc.x + (((output_geometry.size.w as f32) / 3.0) * 2.0) as i32;
        let max_y = output_geometry.loc.y + (((output_geometry.size.h as f32) / 3.0) * 2.0) as i32;
        let x_range = Uniform::new(output_geometry.loc.x, max_x);
        let y_range = Uniform::new(output_geometry.loc.y, max_y);
        let mut rng = rand::thread_rng();
        let x = x_range.sample(&mut rng);
        let y = y_range.sample(&mut rng);
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        window_map.borrow_mut().insert(SurfaceKind::Xdg(surface), (x, y).into());
    }

    pub fn xdg_new_popup (
        surface:    PopupSurface,
        window_map: &Rc<RefCell<WindowMap>>
    ) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        window_map.borrow_mut().insert_popup(PopupKind::Xdg(surface));
    }

    pub fn xdg_move (
        surface:    &ToplevelSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>,
        seat:       WlSeat,
        serial:     Serial
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch move.
        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .as_ref()
                .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            return;
        }

        let toplevel = SurfaceKind::Xdg(surface.clone());
        let mut initial_window_location = window_map.borrow().location(&toplevel).unwrap();

        // If surface is maximized then unmaximize it
        if let Some(current_state) = surface.current_state() {
            if current_state.states.contains(xdg_toplevel::State::Maximized) {
                let fs_changed = surface.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Maximized);
                    state.size = None;
                });

                if fs_changed.is_ok() {
                    surface.send_configure();

                    // NOTE: In real compositor mouse location should be mapped to a new window size
                    // For example, you could:
                    // 1) transform mouse pointer position from compositor space to window space (location relative)
                    // 2) divide the x coordinate by width of the window to get the percentage
                    //   - 0.0 would be on the far left of the window
                    //   - 0.5 would be in middle of the window
                    //   - 1.0 would be on the far right of the window
                    // 3) multiply the percentage by new window width
                    // 4) by doing that, drag will look a lot more natural
                    //
                    // but for anvil needs setting location to pointer location is fine
                    let pos = pointer.current_location();
                    initial_window_location = (pos.x as i32, pos.y as i32).into();
                }
            }
        }

        let grab = MoveSurfaceGrab {
            start_data,
            window_map: window_map.clone(),
            toplevel,
            initial_window_location,
        };

        pointer.set_grab(grab, serial);
    }

    pub fn xdg_resize (
        surface:    &ToplevelSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>,
        seat:       WlSeat,
        serial:     Serial,
        edges:      ResizeEdge
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch resize.
        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .as_ref()
                .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            return;
        }
        let toplevel = SurfaceKind::Xdg(surface.clone());
        let initial_window_location = window_map.borrow().location(&toplevel).unwrap();
        let geometry = window_map.borrow().geometry(&toplevel).unwrap();
        let initial_window_size = geometry.size;
        with_states(surface.get_surface().unwrap(), move |states| {
            states.data_map.get::<RefCell<SurfaceData>>().unwrap().borrow_mut().resize_state =
                ResizeState::Resizing(ResizeData {
                    edges: edges.into(), initial_window_location, initial_window_size,
                });
        }).unwrap();

        pointer.set_grab(ResizeSurfaceGrab {
            start_data,
            toplevel,
            edges: edges.into(),
            initial_window_size,
            last_window_size: initial_window_size,
        }, serial);
    }

    pub fn xdg_ack_configure (
        surface:    &WlSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>,
        configure:  ToplevelConfigure
    ) {
        let waiting_for_serial = with_states(&surface, |states| {
            if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                if let ResizeState::WaitingForFinalAck(_, serial) = data.borrow().resize_state {
                    return Some(serial);
                }
            }
            None
        }).unwrap();
        if let Some(serial) = waiting_for_serial {
            // When the resize grab is released the surface
            // resize state will be set to WaitingForFinalAck
            // and the client will receive a configure request
            // without the resize state to inform the client
            // resizing has finished. Here we will wait for
            // the client to acknowledge the end of the
            // resizing. To check if the surface was resizing
            // before sending the configure we need to use
            // the current state as the received acknowledge
            // will no longer have the resize state set
            if configure.serial >= serial && with_states(&surface, |states| states.data_map
                .get::<Mutex<XdgToplevelSurfaceRoleAttributes>>().unwrap()
                .lock().unwrap()
                .current.states.contains(xdg_toplevel::State::Resizing)).unwrap()
            {
                with_states(&surface, |states| {
                    let mut data = states.data_map.get::<RefCell<SurfaceData>>().unwrap()
                        .borrow_mut();
                    if let ResizeState::WaitingForFinalAck(resize_data, _) = data.resize_state {
                        data.resize_state = ResizeState::WaitingForCommit(resize_data);
                    } else {
                        unreachable!()
                    }
                })
                .unwrap();
            }
        }
    }

    pub fn xdg_fullscreen (
        surface:    &ToplevelSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>,
        output:     Option<WlOutput>
    ) {
        // NOTE: This is only one part of the solution. We can set the
        // location and configure size here, but the surface should be rendered fullscreen
        // independently from its buffer size
        let wl_surface = if let Some(surface) = surface.get_surface() {
            surface
        } else {
            // If there is no underlying surface just ignore the request
            return;
        };

        let output_geometry = fullscreen_output_geometry(
            wl_surface,
            output.as_ref(),
            &window_map.borrow(),
            &output_map.borrow(),
        );

        if let Some(geometry) = output_geometry {
            if let Some(surface) = surface.get_surface() {
                let mut window_map = window_map.borrow_mut();
                if let Some(kind) = window_map.find(surface) {
                    window_map.set_location(&kind, geometry.loc);
                }
            }

            let ret = surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Fullscreen);
                state.size = Some(geometry.size);
                state.fullscreen_output = output;
            });
            if ret.is_ok() {
                surface.send_configure();
            }
        }
    }

    pub fn xdg_unfullscreen (surface: &ToplevelSurface) {
        let ret = surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.size = None;
            state.fullscreen_output = None;
        });
        if ret.is_ok() {
            surface.send_configure();
        }
    }

    pub fn xdg_maximize (
        surface:    &ToplevelSurface,
        output_map: &Rc<RefCell<OutputMap>>,
        window_map: &Rc<RefCell<WindowMap>>,
    ) {
        // NOTE: This should use layer-shell when it is implemented to
        // get the correct maximum size
        let output_geometry = {
            let window_map = window_map.borrow();
            surface
                .get_surface()
                .and_then(|s| window_map.find(s))
                .and_then(|k| window_map.location(&k))
                .and_then(|position| {
                    output_map
                        .borrow()
                        .find_by_position(position)
                        .map(|o| o.geometry())
                })
        };

        if let Some(geometry) = output_geometry {
            if let Some(surface) = surface.get_surface() {
                let mut window_map = window_map.borrow_mut();
                if let Some(kind) = window_map.find(surface) {
                    window_map.set_location(&kind, geometry.loc);
                }
            }
            let ret = surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Maximized);
                state.size = Some(geometry.size);
            });
            if ret.is_ok() {
                surface.send_configure();
            }
        }
    }

    pub fn xdg_unmaximize (surface: &ToplevelSurface) {
        let ret = surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.size = None;
        });
        if ret.is_ok() {
            surface.send_configure();
        }
    }

    pub fn init_wl_shell (
        log:        &Logger,
        display:    &Rc<RefCell<Display>>,
        window_map: &Rc<RefCell<WindowMap>>,
        output_map: &Rc<RefCell<OutputMap>>
    ) -> Arc<Mutex<WlShellState>> {
        let window_map = window_map.clone();
        let output_map = output_map.clone();
        let (state, _) = wl_shell_init(&mut *display.borrow_mut(), move |req: ShellRequest, _dispatch_data| {
            match req {
                ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Toplevel, }
                    => Self::set_toplevel(surface, &window_map, &output_map),
                ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Fullscreen { output, .. } }
                    => Self::set_fullscreen(surface, &window_map, &output_map, output),
                ShellRequest::Move { surface, seat, serial }
                    => Self::shell_move(surface, &window_map, seat, serial),
                ShellRequest::Resize { surface, seat, serial, edges, }
                    => Self::shell_resize(surface, &window_map, seat, serial, edges),
                    _ => (),
            }
        }, log.clone());
        state
    }

    /// place the window at a random location on the primary output
    /// or if there is not output in a [0;800]x[0;800] square
    pub fn set_toplevel (
        surface:    ShellSurface,
        window_map: &Rc<RefCell<WindowMap>>,
        output_map: &Rc<RefCell<OutputMap>>,
    ) {
        use rand::distributions::{Distribution, Uniform};
        let output_geometry = output_map
            .borrow()
            .with_primary()
            .map(|o| o.geometry())
            .unwrap_or_else(|| Rectangle::from_loc_and_size((0, 0), (800, 800)));
        let max_x =
            output_geometry.loc.x + (((output_geometry.size.w as f32) / 3.0) * 2.0) as i32;
        let max_y =
            output_geometry.loc.y + (((output_geometry.size.h as f32) / 3.0) * 2.0) as i32;
        let x_range = Uniform::new(output_geometry.loc.x, max_x);
        let y_range = Uniform::new(output_geometry.loc.y, max_y);
        let mut rng = rand::thread_rng();
        let x = x_range.sample(&mut rng);
        let y = y_range.sample(&mut rng);
        window_map.borrow_mut().insert(SurfaceKind::Wl(surface), (x, y).into());
    }

    pub fn set_fullscreen (
        surface:    ShellSurface,
        window_map: &Rc<RefCell<WindowMap>>,
        output_map: &Rc<RefCell<OutputMap>>,
        output:     Option<WlOutput>
    ) {
        // NOTE: This is only one part of the solution. We can set the
        // location and configure size here, but the surface should be rendered fullscreen
        // independently from its buffer size
        if let Some(wl_surface) = surface.get_surface() {
            let output_geometry = fullscreen_output_geometry(
                wl_surface, output.as_ref(), &window_map.borrow(), &output_map.borrow(),
            );
            if let Some(geometry) = output_geometry {
                window_map.borrow_mut().insert(SurfaceKind::Wl(surface), geometry.loc);
            }
        } else {
            // If there is no underlying surface just ignore the request
            return;
        };
    }

    pub fn shell_move (
        surface:    ShellSurface,
        window_map: &Rc<RefCell<WindowMap>>,
        seat:       WlSeat,
        serial:     Serial
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        let pointer = seat.get_pointer().unwrap();
        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) { return; }
        let start_data = pointer.grab_start_data();
        // If the focus was for a different surface, ignore the request.
        if let Some(start_data) = start_data && start_data.focus.as_ref().unwrap().0.as_ref()
            .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            let toplevel = SurfaceKind::Wl(surface);
            let initial_window_location = window_map.borrow().location(&toplevel).unwrap();
            pointer.set_grab(MoveSurfaceGrab {
                start_data, window_map: window_map.clone(), toplevel, initial_window_location,
            }, serial);
        }
    }
    
    pub fn shell_resize (
        surface:    ShellSurface,
        window_map: &Rc<RefCell<WindowMap>>,
        seat:       WlSeat,
        serial:     Serial,
        edges:      Resize,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch resize.
        let pointer = seat.get_pointer().unwrap();
        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) { return; }
        let start_data = pointer.grab_start_data();
        // If the focus was for a different surface, ignore the request.
        if let Some(start_data) = start_data && start_data.focus.as_ref().unwrap().0.as_ref()
            .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            let toplevel = SurfaceKind::Wl(surface.clone());
            let initial_window_location = window_map.borrow().location(&toplevel).unwrap();
            let geometry = window_map.borrow().geometry(&toplevel).unwrap();
            let initial_window_size = geometry.size;
            with_states(surface.get_surface().unwrap(), move |states| {
                states.data_map.get::<RefCell<SurfaceData>>().unwrap().borrow_mut().resize_state =
                    ResizeState::Resizing(ResizeData {
                        edges: edges.into(),
                        initial_window_location,
                        initial_window_size,
                    }); }).unwrap();
            let grab = ResizeSurfaceGrab {
                start_data,
                toplevel,
                edges: edges.into(),
                initial_window_size,
                last_window_size: initial_window_size,
            };
            pointer.set_grab(grab, serial);
        }
    }

    pub fn add_output (&self, name: &str) -> &Self {
        self.output_map.borrow_mut().add(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode {
                size: self.renderer.borrow().window_size().physical_size, refresh: 60_000
            }
        );
        self
    }

    pub fn run (
        &mut self,
        display:        &Rc<RefCell<Display>>,
        mut input:      WinitInputBackend,
        mut event_loop: EventLoop<'static, Self>,
    ) {
        let start_time = std::time::Instant::now();
        let mut cursor_visible = true;
        info!(self.log, "Initialization completed, starting the main loop.");
        while self.running() {
            let handle = |event| self.process_input_event(event);
            if input.dispatch_new_events(handle).is_err() {
                self.running.store(false, Ordering::SeqCst);
                break;
            }
            self.draw(&mut cursor_visible);
            self.send_frames(start_time.elapsed().as_millis() as u32);
            self.flush(display);
            if event_loop.dispatch(Some(Duration::from_millis(16)), self).is_err() {
                self.stop();
            } else {
                self.flush(display);
                self.refresh();
            }
        }
        self.clear();
    }

    pub fn draw (&self, cursor_visible: &mut bool) {
        let (output_geometry, output_scale) = self.output_map.borrow()
            .find_by_name(OUTPUT_NAME)
            .map(|output| (output.geometry(), output.scale()))
            .unwrap();
        // This is safe to do as with winit we are guaranteed to have exactly one output
        let result = self.renderer.borrow_mut().render(|renderer, frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0])?;
            let windows = self.window_map.borrow();
            windows.draw_windows(&self.log, renderer, frame, output_geometry, output_scale)?;
            let (x, y) = self.pointer_location.into();
            let location: Point<i32, Logical> = (x as i32, y as i32).into();
            self.draw_dnd_icon(renderer, frame, output_scale, location)?;
            self.draw_cursor(renderer, frame, output_scale, cursor_visible, location)?;
            Ok(())
        }).map_err(Into::<SwapBuffersError>::into).and_then(|x| x);
        self.renderer.borrow().window().set_cursor_visible(*cursor_visible);
        if let Err(SwapBuffersError::ContextLost(err)) = result {
            error!(self.log, "Critical Rendering Error: {}", err);
            self.stop();
        }
    }

    pub fn draw_dnd_icon<R, F, E, T>(
        &self,
        renderer:     &mut R,
        frame:        &mut F,
        output_scale: f32,
        location:     Point<i32, Logical>,
    )
        -> Result<(), SwapBuffersError>
    where
        T: Texture + 'static,
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: Error + Into<SwapBuffersError>
    {
        let guard = self.dnd_icon.lock().unwrap();
        Ok(if let Some(ref surface) = *guard && surface.as_ref().is_alive() {
            if get_role(surface) != Some("dnd_icon") {
                warn!(self.log, "Trying to display as a dnd icon a surface that does not have the DndIcon role.");
            }
            draw_surface_tree(&self.log, renderer, frame, surface, location, output_scale)?
        } else {
            ()
        })
    }

    pub fn draw_cursor<R, F, E, T>(
        &self,
        renderer:       &mut R,
        frame:          &mut F,
        output_scale:   f32,
        cursor_visible: &mut bool,
        location:       Point<i32, Logical>,
    )
        -> Result<(), SwapBuffersError>
    where
        T: Texture + 'static,
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: Error + Into<SwapBuffersError>,
    {
        let mut guard = self.cursor_status.lock().unwrap();
        let mut reset = false; // reset the cursor if the surface is no longer alive
        if let CursorImageStatus::Image(ref surface) = *guard {
            reset = !surface.as_ref().is_alive();
        }
        if reset {
            *guard = CursorImageStatus::Default;
        }
        Ok(if let CursorImageStatus::Image(ref surface) = *guard {
            *cursor_visible = false;
            let states = with_states(surface, |states| Some(states.data_map.get::<Mutex<CursorImageAttributes>>()
                .unwrap().lock().unwrap().hotspot));
            let delta = if let Some(h) = states.unwrap_or(None) { h } else {
                warn!(self.log, "Trying to display as a cursor a surface that does not have the CursorImage role.");
                (0, 0).into()
            };
            draw_surface_tree(&self.log, renderer, frame, surface, location - delta, output_scale)?
        } else {
            *cursor_visible = true;
            ()
        })
    }

    /// Send frame events so that client start drawing their next frame
    pub fn send_frames (&self, frames: u32) {
        self.window_map.borrow().send_frames(frames);
    }

    pub fn clear (&self) {
        self.window_map.borrow_mut().clear()
    }

    pub fn running (&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop (&self) {
        self.running.store(false, Ordering::SeqCst)
    }

    pub fn flush (&mut self, display: &Rc<RefCell<Display>>) {
        display.borrow_mut().flush_clients(self);
    }

    pub fn refresh (&mut self) {
        self.window_map.borrow_mut().refresh();
        self.output_map.borrow_mut().refresh();
    }

    fn keyboard_key_to_action<B: InputBackend>(&mut self, evt: B::KeyboardKeyEvent) -> KeyAction {
        let keycode = evt.key_code();
        let state = evt.state();
        debug!(self.log, "key"; "keycode" => keycode, "state" => format!("{:?}", state));
        let serial = SCOUNTER.next_serial();
        let log = &self.log;
        let time = Event::time(&evt);
        let mut action = KeyAction::None;
        let suppressed_keys = &mut self.suppressed_keys;
        self.keyboard.input(keycode, state, serial, time, |modifiers, keysym| {
            debug!(log, "keysym";
                "state" => format!("{:?}", state),
                "mods" => format!("{:?}", modifiers),
                "keysym" => ::xkbcommon::xkb::keysym_get_name(keysym)
            );

            // If the key is pressed and triggered a action
            // we will not forward the key to the client.
            // Additionally add the key to the suppressed keys
            // so that we can decide on a release if the key
            // should be forwarded to the client or not.
            if let KeyState::Pressed = state {
                action = process_keyboard_shortcut(*modifiers, keysym);

                // forward to client only if action == KeyAction::Forward
                let forward = matches!(action, KeyAction::Forward);

                if !forward {
                    suppressed_keys.push(keysym);
                }

                forward
            } else {
                let suppressed = suppressed_keys.contains(&keysym);

                if suppressed {
                    suppressed_keys.retain(|k| *k != keysym);
                }

                !suppressed
            }
        });
        action
    }

    fn on_pointer_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
        let serial = SCOUNTER.next_serial();
        let button = match evt.button() {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
            MouseButton::Other(b) => b as u32,
        };
        let state = match evt.state() {
            ButtonState::Pressed => {
                // change the keyboard focus unless the pointer is grabbed
                if !self.pointer.is_grabbed() {
                    let under = self
                        .window_map
                        .borrow_mut()
                        .get_surface_and_bring_to_top(self.pointer_location);
                    self.keyboard
                        .set_focus(under.as_ref().map(|&(ref s, _)| s), serial);
                }
                wl_pointer::ButtonState::Pressed
            }
            ButtonState::Released => wl_pointer::ButtonState::Released,
        };
        self.pointer.button(button, state, serial, evt.time());
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        let source = match evt.source() {
            AxisSource::Continuous => wl_pointer::AxisSource::Continuous,
            AxisSource::Finger => wl_pointer::AxisSource::Finger,
            AxisSource::Wheel | AxisSource::WheelTilt => wl_pointer::AxisSource::Wheel,
        };
        let horizontal_amount = evt
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| evt.amount_discrete(Axis::Horizontal).unwrap() * 3.0);
        let vertical_amount = evt
            .amount(Axis::Vertical)
            .unwrap_or_else(|| evt.amount_discrete(Axis::Vertical).unwrap() * 3.0);
        let horizontal_amount_discrete = evt.amount_discrete(Axis::Horizontal);
        let vertical_amount_discrete = evt.amount_discrete(Axis::Vertical);

        {
            let mut frame = AxisFrame::new(evt.time()).source(source);
            if horizontal_amount != 0.0 {
                frame = frame.value(wl_pointer::Axis::HorizontalScroll, horizontal_amount);
                if let Some(discrete) = horizontal_amount_discrete {
                    frame = frame.discrete(wl_pointer::Axis::HorizontalScroll, discrete as i32);
                }
            } else if source == wl_pointer::AxisSource::Finger {
                frame = frame.stop(wl_pointer::Axis::HorizontalScroll);
            }
            if vertical_amount != 0.0 {
                frame = frame.value(wl_pointer::Axis::VerticalScroll, vertical_amount);
                if let Some(discrete) = vertical_amount_discrete {
                    frame = frame.discrete(wl_pointer::Axis::VerticalScroll, discrete as i32);
                }
            } else if source == wl_pointer::AxisSource::Finger {
                frame = frame.stop(wl_pointer::Axis::VerticalScroll);
            }
            self.pointer.axis(frame);
        }
    }

    pub fn process_input_event<B>(&mut self, event: InputEvent<B>)
    where
        B: InputBackend<SpecialEvent = smithay::backend::winit::WinitEvent>,
    {
        use smithay::backend::winit::WinitEvent;

        match event {
            InputEvent::Keyboard { event, .. } => match self.keyboard_key_to_action::<B>(event) {
                KeyAction::None | KeyAction::Forward => {}
                KeyAction::Quit => {
                    info!(self.log, "Quitting.");
                    self.running.store(false, Ordering::SeqCst);
                }
                KeyAction::Run(cmd) => {
                    info!(self.log, "Starting program"; "cmd" => cmd.clone());
                    if let Err(e) = std::process::Command::new(&cmd).spawn() {
                        error!(self.log,
                            "Failed to start program";
                            "cmd" => cmd,
                            "err" => format!("{:?}", e)
                        );
                    }
                }
                KeyAction::ScaleUp => {
                    let current_scale = {
                        self.output_map
                            .borrow()
                            .find_by_name(OUTPUT_NAME)
                            .map(|o| o.scale())
                            .unwrap_or(1.0)
                    };
                    self.output_map
                        .borrow_mut()
                        .update_scale_by_name(current_scale + 0.25f32, OUTPUT_NAME);
                }
                KeyAction::ScaleDown => {
                    let current_scale = {
                        self.output_map
                            .borrow()
                            .find_by_name(OUTPUT_NAME)
                            .map(|o| o.scale())
                            .unwrap_or(1.0)
                    };

                    self.output_map.borrow_mut().update_scale_by_name(
                        f32::max(1.0f32, current_scale - 0.25f32),
                        OUTPUT_NAME,
                    );
                }
                action => {
                    warn!(self.log, "Key action {:?} unsupported on winit backend.", action);
                }
            },
            InputEvent::PointerMotionAbsolute { event, .. } => self.on_pointer_move_absolute::<B>(event),
            InputEvent::PointerButton { event, .. } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. } => self.on_pointer_axis::<B>(event),
            InputEvent::Special(WinitEvent::Resized { size, .. }) => {
                self.output_map.borrow_mut().update_mode_by_name(
                    OutputMode { size, refresh: 60_000, },
                    OUTPUT_NAME,
                );
            }
            _ => {
                // other events are not handled in anvil (yet)
            }
        }
    }

    fn on_pointer_move_absolute<B: InputBackend>(&mut self, evt: B::PointerMotionAbsoluteEvent) {
        let output_size = self.output_map.borrow().find_by_name(OUTPUT_NAME).map(|o| o.size())
            .unwrap();
        let pos = evt.position_transformed(output_size);
        self.pointer_location = pos;
        let serial = SCOUNTER.next_serial();
        let under = self.window_map.borrow().get_surface_under(pos);
        self.pointer.motion(pos, under, serial, evt.time());
    }

}

fn fullscreen_output_geometry(
    wl_surface: &wl_surface::WlSurface,
    wl_output: Option<&wl_output::WlOutput>,
    window_map: &WindowMap,
    output_map: &OutputMap,
) -> Option<Rectangle<i32, Logical>> {
    // First test if a specific output has been requested
    // if the requested output is not found ignore the request
    if let Some(wl_output) = wl_output {
        return output_map.find_by_output(&wl_output).map(|o| o.geometry());
    }

    // There is no output preference, try to find the output
    // where the window is currently active
    let window_location = window_map
        .find(wl_surface)
        .and_then(|kind| window_map.location(&kind));

    if let Some(location) = window_location {
        let window_output = output_map.find_by_position(location).map(|o| o.geometry());

        if let Some(result) = window_output {
            return Some(result);
        }
    }

    // Fallback to primary output
    output_map.with_primary().map(|o| o.geometry())
}

/// Possible results of a keyboard action
#[derive(Debug)]
enum KeyAction {
    /// Quit the compositor
    Quit,
    /// Trigger a vt-switch
    VtSwitch(i32),
    /// run a command
    Run(String),
    /// Switch the current screen
    Screen(usize),
    ScaleUp,
    ScaleDown,
    /// Forward the key to the client
    Forward,
    /// Do nothing more
    None,
}

fn process_keyboard_shortcut(modifiers: ModifiersState, keysym: Keysym) -> KeyAction {
    if modifiers.ctrl && modifiers.alt && keysym == xkb::KEY_BackSpace
        || modifiers.logo && keysym == xkb::KEY_q
    {
        // ctrl+alt+backspace = quit
        // logo + q = quit
        KeyAction::Quit
    } else if (xkb::KEY_XF86Switch_VT_1..=xkb::KEY_XF86Switch_VT_12).contains(&keysym) {
        // VTSwicth
        KeyAction::VtSwitch((keysym - xkb::KEY_XF86Switch_VT_1 + 1) as i32)
    } else if modifiers.logo && keysym == xkb::KEY_Return {
        // run terminal
        KeyAction::Run("weston-terminal".into())
    } else if modifiers.logo && keysym >= xkb::KEY_1 && keysym <= xkb::KEY_9 {
        KeyAction::Screen((keysym - xkb::KEY_1) as usize)
    } else if modifiers.logo && modifiers.shift && keysym == xkb::KEY_M {
        KeyAction::ScaleDown
    } else if modifiers.logo && modifiers.shift && keysym == xkb::KEY_P {
        KeyAction::ScaleUp
    } else {
        KeyAction::Forward
    }
}
