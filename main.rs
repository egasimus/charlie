mod prelude;
use prelude::*;

mod output;
mod grab;
mod surface;
use surface::draw_surface_tree;
mod popup;
mod window;
mod compositor;
use compositor::Compositor;

fn main () -> Result<(), Box<dyn Error>> {
    let (log, _guard) = App::init_log();
    let display = Rc::new(RefCell::new(Display::new()));
    let (renderer, input) = App::init_io(&log, &display)?;
    let event_loop = EventLoop::try_new().unwrap();
    let mut charlie = App::init(log, &display, &renderer, &event_loop)?;
    charlie.add_output(OUTPUT_NAME);
    std::process::Command::new("kitty").spawn()?;
    Ok(charlie.run(&display, input, event_loop))
}

pub struct App {
    pub log:              Logger,
    pub socket_name:      Option<String>,
    pub running:          Arc<AtomicBool>,
    pub renderer:         Rc<RefCell<WinitGraphicsBackend>>,
    pub dnd_icon:         Arc<Mutex<Option<WlSurface>>>,
    pub pointer:          PointerHandle,
    pub keyboard:         KeyboardHandle,
    pub suppressed_keys:  Vec<u32>,
    pub pointer_location: Point<f64, Logical>,
    pub seat:             Seat,
    pub cursor_status:    Arc<Mutex<CursorImageStatus>>,
    pub compositor:       Rc<Compositor>,
}

impl App {

    pub fn init (
        log:        Logger,
        display:    &Rc<RefCell<Display>>,
        renderer:   &Rc<RefCell<WinitGraphicsBackend>>,
        event_loop: &EventLoop<'static, Self>
    ) -> Result<Self, Box<dyn Error>> {
        init_xdg_output_manager(&mut *display.borrow_mut(), log.clone());
        init_shm_global(&mut *display.borrow_mut(), vec![], log.clone());
        Self::init_loop(&log, &display, event_loop.handle());
        let socket_name = Self::init_socket(&log, &display, true);
        let dnd_icon = Arc::new(Mutex::new(None));
        Self::init_data_device(&log, &display, &dnd_icon);
        let (seat, pointer, cursor_status, keyboard) = Self::init_seat(&log, &display, "seat");
        let compositor = Compositor::init(&log, display);
        Ok(Self {
            cursor_status,
            dnd_icon,
            keyboard,
            log,
            pointer,
            pointer_location: (0.0, 0.0).into(),
            renderer: renderer.clone(),
            running: Arc::new(AtomicBool::new(true)),
            seat,
            socket_name,
            suppressed_keys: Vec::new(),
            compositor
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
            Ok((renderer, input)) => {
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
            move |_, _, state: &mut App| {
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

    pub fn add_output (&self, name: &str) -> &Self {
        let size = self.renderer.borrow().window_size().physical_size;
        self.compositor.output_map.borrow_mut().add(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode { size, refresh: 60_000 }
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
        let (output_geometry, output_scale) = self.compositor.output_map.borrow()
            .find_by_name(OUTPUT_NAME)
            .map(|output| (output.geometry(), output.scale()))
            .unwrap();
        // This is safe to do as with winit we are guaranteed to have exactly one output
        let result = self.renderer.borrow_mut().render(|renderer, frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0])?;
            let windows = self.compositor.window_map.borrow();
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
        self.compositor.window_map.borrow().send_frames(frames);
    }

    pub fn clear (&self) {
        self.compositor.window_map.borrow_mut().clear()
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
        self.compositor.window_map.borrow_mut().refresh();
        self.compositor.output_map.borrow_mut().refresh();
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
                if !forward { suppressed_keys.push(keysym); }
                forward
            } else {
                let suppressed = suppressed_keys.contains(&keysym);
                if suppressed { suppressed_keys.retain(|k| *k != keysym); }
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
                    let under = self.compositor.window_map.borrow_mut()
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
                        self.compositor.output_map.borrow().find_by_name(OUTPUT_NAME)
                            .map(|o| o.scale()).unwrap_or(1.0)
                    };
                    self.compositor.output_map.borrow_mut()
                        .update_scale_by_name(current_scale + 0.25f32, OUTPUT_NAME);
                }
                KeyAction::ScaleDown => {
                    let current_scale = {
                        self.compositor.output_map.borrow().find_by_name(OUTPUT_NAME)
                            .map(|o| o.scale()).unwrap_or(1.0)
                    };
                    self.compositor.output_map.borrow_mut().update_scale_by_name(
                        f32::max(1.0f32, current_scale - 0.25f32),
                        OUTPUT_NAME,
                    );
                }
                action => {
                    warn!(self.log, "Key action {:?} unsupported on winit backend.", action);
                }
            },
            InputEvent::PointerMotionAbsolute { event, .. }
                => self.on_pointer_move_absolute::<B>(event),
            InputEvent::PointerButton { event, .. }
                => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. }
                => self.on_pointer_axis::<B>(event),
            InputEvent::Special(WinitEvent::Resized { size, .. })
                => {
                    self.compositor.output_map.borrow_mut().update_mode_by_name(
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
        let output_size = self.compositor.output_map.borrow().find_by_name(OUTPUT_NAME)
            .map(|o| o.size()).unwrap();
        let pos = evt.position_transformed(output_size);
        self.pointer_location = pos;
        let serial = SCOUNTER.next_serial();
        let under = self.compositor.window_map.borrow().get_surface_under(pos);
        self.pointer.motion(pos, under, serial, evt.time());
    }

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
