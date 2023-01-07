use crate::prelude::*;
use crate::compositor::{Compositor, WindowMap, SurfaceData, SurfaceKind, draw_surface_tree};
use crate::workspace::Workspace;
use std::cell::Cell;

pub struct Controller {
    pub log:                   Logger,
    pub running:               Arc<AtomicBool>,
    pub compositor:            Rc<RefCell<Compositor>>,
    pub workspace:             Rc<RefCell<Workspace>>,
    pub seat:                  Seat,
    pub pointer:               PointerHandle,
    pub pointer_location:      Point<f64, Logical>,
    pub last_pointer_location: Point<f64, Logical>,
    pub cursor_status:         Arc<Mutex<CursorImageStatus>>,
    pub cursor_visible:        Cell<bool>,
    pub dnd_icon:              Arc<Mutex<Option<WlSurface>>>,
    pub keyboard:              KeyboardHandle,
    pub suppressed_keys:       Vec<u32>,
}

impl Controller {

    pub fn init (
        log:        &Logger,
        running:    &Arc<AtomicBool>,
        display:    &Rc<RefCell<Display>>,
        compositor: &Rc<RefCell<Compositor>>,
        workspace:  &Rc<RefCell<Workspace>>
    ) -> Self {
        let seat_name  = "seat";
        let (mut seat, _) = Seat::new(&mut display.borrow_mut(), seat_name.to_string(), log.clone());
        let cursor_status = Arc::new(Mutex::new(CursorImageStatus::Default));
        let cursor_status2 = cursor_status.clone();
        let pointer = seat.add_pointer(move |new_status| {
            *cursor_status2.lock().unwrap() = new_status
        });
        init_tablet_manager_global(&mut display.borrow_mut());
        let cursor_status3 = cursor_status.clone();
        seat.tablet_seat().on_cursor_surface(move |_tool, new_status| {
            *cursor_status3.lock().unwrap() = new_status
        });
        let keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25, |seat, focus| {
            set_data_device_focus(seat, focus.and_then(|s| s.as_ref().client()))
        }).expect("Failed to initialize the keyboard");
        let dnd_icon = Arc::new(Mutex::new(None));
        Self::init_data_device(&log, &display, &dnd_icon);
        Self {
            log:                   log.clone(),
            running:               running.clone(),
            compositor:            compositor.clone(),
            workspace:             workspace.clone(),
            seat,
            keyboard,
            suppressed_keys:       vec![],
            pointer,
            pointer_location:      (0.0, 0.0).into(),
            last_pointer_location: (0.0, 0.0).into(),
            cursor_status,
            cursor_visible:        Cell::new(true),
            dnd_icon
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

    pub fn draw (
        &self,
        renderer:     &mut Gles2Renderer,
        frame:        &mut Gles2Frame,
        output_scale: f32
    ) -> Result<(), SwapBuffersError> {
        let (x, y) = self.pointer_location.into();
        let location: Point<i32, Logical> = (x as i32, y as i32).into();
        self.draw_dnd_icon(renderer, frame, output_scale, location)?;
        self.draw_cursor(renderer, frame, output_scale, location)?;
        Ok(())
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
            self.cursor_visible.set(false);
            let states = with_states(surface, |states|
                Some(states.data_map.get::<Mutex<CursorImageAttributes>>()
                    .unwrap().lock().unwrap().hotspot));
            let delta = if let Some(h) = states.unwrap_or(None) { h } else {
                warn!(self.log, "Trying to display as a cursor a surface that does not have the CursorImage role.");
                (0, 0).into()
            };
            draw_surface_tree(&self.log, renderer, frame, surface, location - delta, output_scale)?
        } else {
            self.cursor_visible.set(true);
            ()
        })
    }

    pub fn process_input_event<B>(&mut self, event: InputEvent<B>)
    where
        B: InputBackend<SpecialEvent = smithay::backend::winit::WinitEvent>,
    {
        use smithay::backend::winit::WinitEvent;
        match event {
            InputEvent::Keyboard { event, .. }
                => self.on_keyboard::<B>(event),
            InputEvent::PointerMotion { event, .. }
                => self.on_pointer_move_relative::<B>(event),
            InputEvent::PointerMotionAbsolute { event, .. }
                => self.on_pointer_move_absolute::<B>(event),
            InputEvent::PointerButton { event, .. }
                => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. }
                => self.on_pointer_axis::<B>(event),
            InputEvent::Special(WinitEvent::Resized { size, .. })
                => {
                    self.compositor.borrow_mut().update_mode_by_name(
                        OutputMode { size, refresh: 60_000, },
                        OUTPUT_NAME,
                    );
                }
            _ => {
                // other events are not handled in anvil (yet)
            }
        }
    }

    fn on_pointer_move_relative<B: InputBackend>(&mut self, evt: B::PointerMotionEvent) {
        let delta = evt.delta();
        panic!("{:?}", delta);
    }

    fn on_pointer_move_absolute<B: InputBackend>(&mut self, evt: B::PointerMotionAbsoluteEvent) {
        let output_size = self.compositor.borrow().find_by_name(OUTPUT_NAME)
            .map(|o| o.size()).unwrap();
        self.last_pointer_location = self.pointer_location;
        self.pointer_location = evt.position_transformed(output_size);
        self.workspace.borrow_mut()
            .on_pointer_move_absolute(self.pointer_location, self.last_pointer_location);
        let pos    = self.pointer_location - self.workspace.borrow().offset.to_logical(1.0);
        let serial = SCOUNTER.next_serial();
        let under  = self.compositor.borrow().window_map.borrow().get_surface_under(pos);
        self.pointer.motion(pos, under, serial, evt.time());
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
                    let pos   = self.pointer_location - self.workspace.borrow().offset.to_logical(1.0);
                    let under = self.compositor.borrow().window_map.borrow().get_surface_under(pos);
                    if under.is_some() {
                        let under = self.compositor.borrow().window_map.borrow_mut()
                            .get_surface_and_bring_to_top(pos);
                        self.keyboard
                            .set_focus(under.as_ref().map(|&(ref s, _)| s), serial);
                    } else {
                        self.workspace.borrow_mut().dragging = true;
                    }
                }
                wl_pointer::ButtonState::Pressed
            }
            ButtonState::Released => {
                self.workspace.borrow_mut().dragging = false;
                wl_pointer::ButtonState::Released
            },
        };
        self.pointer.button(button, state, serial, evt.time());
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        let source = match evt.source() {
            AxisSource::Continuous => wl_pointer::AxisSource::Continuous,
            AxisSource::Finger => wl_pointer::AxisSource::Finger,
            AxisSource::Wheel | AxisSource::WheelTilt => wl_pointer::AxisSource::Wheel,
        };

        let mut frame = AxisFrame::new(evt.time()).source(source);

        let horizontal_amount = evt.amount(Axis::Horizontal)
            .unwrap_or_else(|| evt.amount_discrete(Axis::Horizontal).unwrap() * 3.0);
        let horizontal_amount_discrete = evt.amount_discrete(Axis::Horizontal);
        if horizontal_amount != 0.0 {
            frame = frame.value(wl_pointer::Axis::HorizontalScroll, horizontal_amount);
            if let Some(discrete) = horizontal_amount_discrete {
                frame = frame.discrete(wl_pointer::Axis::HorizontalScroll, discrete as i32);
            }
        } else if source == wl_pointer::AxisSource::Finger {
            frame = frame.stop(wl_pointer::Axis::HorizontalScroll);
        }

        let vertical_amount = evt.amount(Axis::Vertical)
            .unwrap_or_else(|| evt.amount_discrete(Axis::Vertical).unwrap() * 3.0);
        let vertical_amount_discrete = evt.amount_discrete(Axis::Vertical);
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

    fn on_keyboard<B: InputBackend> (&mut self, event: B::KeyboardKeyEvent) {
        let keycode = event.key_code();
        let state = event.state();
        debug!(self.log, "key"; "keycode" => keycode, "state" => format!("{:?}", state));
        let serial = SCOUNTER.next_serial();
        let log = &self.log;
        let time = Event::time(&event);
        let mut action = KeyAction::None;
        let suppressed_keys = &mut self.suppressed_keys;
        self.keyboard.input(keycode, state, serial, time, |modifiers, keysym| {
            debug!(log, "keysym";
                "state"  => format!("{:?}", state),
                "mods"   => format!("{:?}", modifiers),
                "keysym" => ::xkbcommon::xkb::keysym_get_name(keysym)
            );
            // If the key is pressed and triggered a action
            // we will not forward the key to the client.
            // Additionally add the key to the suppressed keys
            // so that we can decide on a release if the key
            // should be forwarded to the client or not.
            if let KeyState::Pressed = state {
                action = if modifiers.ctrl && modifiers.alt && keysym == xkb::KEY_BackSpace
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
                };
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
        match action {
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
                    self.compositor.borrow().find_by_name(OUTPUT_NAME)
                        .map(|o| o.scale()).unwrap_or(1.0)
                };
                self.compositor.borrow_mut()
                    .update_scale_by_name(current_scale + 0.05f32, OUTPUT_NAME);
            }
            KeyAction::ScaleDown => {
                let current_scale = {
                    self.compositor.borrow().find_by_name(OUTPUT_NAME)
                        .map(|o| o.scale()).unwrap_or(1.0)
                };
                self.compositor.borrow_mut().update_scale_by_name(
                    f32::max(0.05f32, current_scale - 0.05f32),
                    OUTPUT_NAME,
                );
            }
            action => {
                warn!(self.log, "Key action {:?} unsupported on winit backend.", action);
            }
        };
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

bitflags::bitflags! {
    pub struct ResizeEdge: u32 {
        const NONE = 0;
        const TOP = 1;
        const BOTTOM = 2;
        const LEFT = 4;
        const TOP_LEFT = 5;
        const BOTTOM_LEFT = 6;
        const RIGHT = 8;
        const TOP_RIGHT = 9;
        const BOTTOM_RIGHT = 10;
    }
}

impl From<wl_shell_surface::Resize> for ResizeEdge {
    #[inline]
    fn from(x: wl_shell_surface::Resize) -> Self {
        Self::from_bits(x.bits()).unwrap()
    }
}

impl From<ResizeEdge> for wl_shell_surface::Resize {
    #[inline]
    fn from(x: ResizeEdge) -> Self {
        Self::from_bits(x.bits()).unwrap()
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x.to_raw()).unwrap()
    }
}

impl From<ResizeEdge> for xdg_toplevel::ResizeEdge {
    #[inline]
    fn from(x: ResizeEdge) -> Self {
        Self::from_raw(x.bits()).unwrap()
    }
}

/// Information about the resize operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ResizeData {
    /// The edges the surface is being resized with.
    pub edges: ResizeEdge,
    /// The initial window location.
    pub initial_window_location: Point<i32, Logical>,
    /// The initial window size (geometry width and height).
    pub initial_window_size: Size<i32, Logical>,
}

/// State of the resize operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ResizeState {
    /// The surface is not being resized.
    NotResizing,
    /// The surface is currently being resized.
    Resizing(ResizeData),
    /// The resize has finished, and the surface needs to ack the final configure.
    WaitingForFinalAck(ResizeData, Serial),
    /// The resize has finished, and the surface needs to commit its final state.
    WaitingForCommit(ResizeData),
}

impl Default for ResizeState {
    fn default() -> Self {
        ResizeState::NotResizing
    }
}

pub struct ResizeSurfaceGrab {
    pub start_data: GrabStartData,
    pub toplevel: SurfaceKind,
    pub edges: ResizeEdge,
    pub initial_window_size: Size<i32, Logical>,
    pub last_window_size: Size<i32, Logical>,
}

impl PointerGrab for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        handle: &mut PointerInnerHandle<'_>,
        location: Point<f64, Logical>,
        _focus: Option<(wl_surface::WlSurface, Point<i32, Logical>)>,
        serial: Serial,
        time: u32,
    ) {
        // It is impossible to get `min_size` and `max_size` of dead toplevel, so we return early.
        if !self.toplevel.alive() | self.toplevel.get_surface().is_none() {
            handle.unset_grab(serial, time);
            return;
        }

        let (mut dx, mut dy) = (location - self.start_data.location).into();

        let mut new_window_width = self.initial_window_size.w;
        let mut new_window_height = self.initial_window_size.h;

        let left_right = ResizeEdge::LEFT | ResizeEdge::RIGHT;
        let top_bottom = ResizeEdge::TOP | ResizeEdge::BOTTOM;

        if self.edges.intersects(left_right) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                dx = -dx;
            }

            new_window_width = (self.initial_window_size.w as f64 + dx) as i32;
        }

        if self.edges.intersects(top_bottom) {
            if self.edges.intersects(ResizeEdge::TOP) {
                dy = -dy;
            }

            new_window_height = (self.initial_window_size.h as f64 + dy) as i32;
        }

        let (min_size, max_size) = with_states(self.toplevel.get_surface().unwrap(), |states| {
            let data = states.cached_state.current::<SurfaceCachedState>();
            (data.min_size, data.max_size)
        })
        .unwrap();

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);
        let max_width = if max_size.w == 0 {
            i32::max_value()
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::max_value()
        } else {
            max_size.h
        };

        new_window_width = new_window_width.max(min_width).min(max_width);
        new_window_height = new_window_height.max(min_height).min(max_height);

        self.last_window_size = (new_window_width, new_window_height).into();

        match &self.toplevel {
            SurfaceKind::Xdg(xdg) => {
                let ret = xdg.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Resizing);
                    state.size = Some(self.last_window_size);
                });
                if ret.is_ok() {
                    xdg.send_configure();
                }
            }
            SurfaceKind::Wl(wl) => wl.send_configure(self.last_window_size, self.edges.into()),
            SurfaceKind::X11(_) => {
                // TODO: What to do here? Send the update via X11?
            }
        }
    }

    fn button(
        &mut self,
        handle: &mut PointerInnerHandle<'_>,
        button: u32,
        state: WlButtonState,
        serial: Serial,
        time: u32,
    ) {
        handle.button(button, state, serial, time);
        if handle.current_pressed().is_empty() {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(serial, time);

            // If toplevel is dead, we can't resize it, so we return early.
            if !self.toplevel.alive() | self.toplevel.get_surface().is_none() {
                return;
            }

            if let SurfaceKind::Xdg(xdg) = &self.toplevel {
                let ret = xdg.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                    state.size = Some(self.last_window_size);
                });
                if ret.is_ok() {
                    xdg.send_configure();
                }

                with_states(self.toplevel.get_surface().unwrap(), |states| {
                    let mut data = states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut();
                    if let ResizeState::Resizing(resize_data) = data.resize_state {
                        data.resize_state = ResizeState::WaitingForFinalAck(resize_data, serial);
                    } else {
                        panic!("invalid resize state: {:?}", data.resize_state);
                    }
                })
                .unwrap();
            } else {
                with_states(self.toplevel.get_surface().unwrap(), |states| {
                    let mut data = states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut();
                    if let ResizeState::Resizing(resize_data) = data.resize_state {
                        data.resize_state = ResizeState::WaitingForCommit(resize_data);
                    } else {
                        panic!("invalid resize state: {:?}", data.resize_state);
                    }
                })
                .unwrap();
            }
        }
    }

    fn axis(&mut self, handle: &mut PointerInnerHandle<'_>, details: AxisFrame) {
        handle.axis(details)
    }

    fn start_data(&self) -> &GrabStartData {
        &self.start_data
    }
}

#[derive(Clone)]
pub struct ShellHandles;

pub struct MoveSurfaceGrab {
    pub start_data: GrabStartData,
    pub window_map: Rc<RefCell<WindowMap>>,
    pub toplevel: SurfaceKind,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab for MoveSurfaceGrab {
    fn motion(
        &mut self,
        _handle: &mut PointerInnerHandle<'_>,
        location: Point<f64, Logical>,
        _focus: Option<(wl_surface::WlSurface, Point<i32, Logical>)>,
        _serial: Serial,
        _time: u32,
    ) {
        let delta = location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;

        self.window_map.borrow_mut().set_location(
            &self.toplevel,
            (new_location.x as i32, new_location.y as i32).into(),
        );
    }

    fn button(
        &mut self,
        handle: &mut PointerInnerHandle<'_>,
        button: u32,
        state: WlButtonState,
        serial: Serial,
        time: u32,
    ) {
        handle.button(button, state, serial, time);
        if handle.current_pressed().is_empty() {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(serial, time);
        }
    }

    fn axis(&mut self, handle: &mut PointerInnerHandle<'_>, details: AxisFrame) {
        handle.axis(details)
    }

    fn start_data(&self) -> &GrabStartData {
        &self.start_data
    }
}
