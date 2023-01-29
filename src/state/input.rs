use super::prelude::*;

use smithay::{
    backend::input::{
        Event,
        //KeyState,
        KeyboardKeyEvent,
        //AbsolutePositionEvent,
        PointerButtonEvent,
        PointerMotionEvent,
        //PointerAxisEvent
    },
    input::{
        pointer::{
            PointerHandle,
            CursorImageStatus     as Status,
            CursorImageAttributes as Attributes
        },
        keyboard::{
            //keysyms,
            KeyboardHandle,
            FilterResult,
        },
    },
    wayland::input_method::InputMethodSeat
};

smithay::delegate_seat!(@<E: Engine> Charlie<E>);

smithay::delegate_data_device!(@<E: Engine> Charlie<E>);

impl<E: Engine, B: InputBackend> Update<(InputEvent<B>, ScreenId)> for Charlie<E> {
    fn update (&mut self, (event, screen_id): (InputEvent<B>, ScreenId)) -> StdResult<()> {
        handle_input(self, event, screen_id)
    }
}

fn handle_input <E: Engine, B: InputBackend> (
    state: &mut Charlie<E>,
    event: InputEvent<B>,
    screen_id: ScreenId
) -> StdResult<()> {
    Ok(match event {
        InputEvent::PointerMotion { event, .. }
            => Pointer::on_move_relative::<B>(state, 0, event, screen_id),
        InputEvent::PointerMotionAbsolute { event, .. }
            => Pointer::on_move_absolute::<B>(state, 0, event, screen_id),
        InputEvent::PointerButton { event, .. }
            => Pointer::on_button::<B>(state, 0, event, screen_id),
        InputEvent::PointerAxis { event, .. }
            => Pointer::on_axis::<B>(state, 0, event, screen_id),
        InputEvent::Keyboard { event, .. }
            => Keyboard::on_key::<B>(state, 0, event, screen_id),
        _ => {}
    })
}

pub struct Input<E: Engine> {
    logger:      Logger,
    handle:      DisplayHandle,
    seat:        SeatState<Charlie<E>>,
    data_device: DataDeviceState,
    /// State of the mouse pointer(s)
    pub pointers:  Vec<Pointer<E>>,
    /// State of the keyboard(s)
    pub keyboards: Vec<Keyboard<E>>,
}

impl<E: Engine> Input<E> {

    pub fn new (logger: &Logger, handle: &DisplayHandle) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:      logger.clone(),
            handle:      handle.clone(),
            seat:        SeatState::new(),
            data_device: DataDeviceState::new::<Charlie<E>, _>(&handle, logger.clone()),
            pointers:    vec![],
            keyboards:   vec![],
        })
    }

    pub fn seat_add (&mut self, name: impl Into<String>, pointer: Gles2Texture)
        -> Result<Seat<Charlie<E>>, Box<dyn Error>>
    {
        let mut seat = self.seat.new_wl_seat(&self.handle, name.into(), self.logger.clone());
        self.pointers.push(
            Pointer::new(&self.logger, seat.add_pointer(), pointer)?
        );
        self.keyboards.push(
            Keyboard::new(&self.logger, seat.add_keyboard(XkbConfig::default(), 200, 25)?)
        );
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(seat)
    }

}

impl<E: Engine> SeatHandler for Charlie<E> {
    type KeyboardFocus = WlSurface;
    type PointerFocus  = WlSurface;

    fn seat_state (&mut self) -> &mut SeatState<Self> {
        &mut self.input.seat
    }

    fn cursor_image (
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {
    }
}

impl<E: Engine> DataDeviceHandler for Charlie<E> {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.input.data_device
    }
}

impl<E: Engine> ClientDndGrabHandler for Charlie<E> {}

impl<E: Engine> ServerDndGrabHandler for Charlie<E> {}

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

pub struct Keyboard<E: Engine> {
    logger:  Logger,
    handle:  KeyboardHandle<Charlie<E>>,
    hotkeys: Vec<u32>,
}

impl<E: Engine> Keyboard<E> {

    pub fn new (logger: &Logger, handle: KeyboardHandle<Charlie<E>>) -> Self {
        Self {
            logger: logger.clone(),
            handle,
            hotkeys: vec![],
        }
    }

    pub fn on_key <B: InputBackend> (
        state: &mut Charlie<E>,
        index: usize,
        event: B::KeyboardKeyEvent,
        screen_id: usize
    ) {
        let key_code   = event.key_code();
        let key_state  = event.state();
        let serial     = SERIAL_COUNTER.next_serial();
        let logger     = state.logger.clone();
        let time       = Event::time(&event);
        //let hotkeys    = &mut state.keyboards[index].hotkeys;
        let mut action = KeyAction::None;
        debug!(state.logger, "key"; "keycode" => key_code, "state" => format!("{:?}", key_state));
        let keyboard = &mut state.input.keyboards[index];
        keyboard.handle.clone().input::<(), _>(state, key_code, key_state, serial, time, |_,_,_|{
            FilterResult::Forward
        });
        //self.keyboard.input((), keycode, state, serial, time, |state, modifiers, keysym| {
            //debug!(log, "keysym";
                //"state"  => format!("{:?}", state),
                //"mods"   => format!("{:?}", modifiers),
                //"keysym" => ::xkbcommon::xkb::keysym_get_name(keysym)
            //);
            //if let KeyState::Pressed = state {
                //action = if modifiers.ctrl && modifiers.alt && keysym == keysyms::KEY_BackSpace
                    //|| modifiers.logo && keysym == keysyms::KEY_q
                //{
                    //KeyAction::Quit
                //} else if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&keysym) {
                    //// VTSwicth
                    //KeyAction::VtSwitch((keysym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32)
                //} else if modifiers.logo && keysym == keysyms::KEY_Return {
                    //// run terminal
                    //KeyAction::Run("weston-terminal".into())
                //} else if modifiers.logo && keysym >= keysyms::KEY_1 && keysym <= keysyms::KEY_9 {
                    //KeyAction::Screen((keysym - keysyms::KEY_1) as usize)
                //} else if modifiers.logo && modifiers.shift && keysym == keysyms::KEY_M {
                    //KeyAction::ScaleDown
                //} else if modifiers.logo && modifiers.shift && keysym == keysyms::KEY_P {
                    //KeyAction::ScaleUp
                //} else {
                    //KeyAction::Forward
                //};
                //// forward to client only if action == KeyAction::Forward
                //let forward = matches!(action, KeyAction::Forward);
                //if !forward { hotkeys.push(keysym); }
                //forward
            //} else {
                //let suppressed = hotkeys.contains(&keysym);
                //if suppressed { hotkeys.retain(|k| *k != keysym); }
                ////!suppressed
            //}
        //});

        //match action {
            //KeyAction::None | KeyAction::Forward => {}
            //KeyAction::Quit => {}
            //KeyAction::Run(cmd) => {}
            //KeyAction::ScaleUp => {}
            //KeyAction::ScaleDown => {}
            //action => {
                //warn!(self.logger, "Key action {:?} unsupported on winit backend.", action);
            //}
        //};
    }

}

pub struct Pointer<E: Engine> {
    logger:        Logger,
    pub handle:    PointerHandle<Charlie<E>>,
    pub texture:   Gles2Texture,
    status:        Arc<Mutex<Status>>,
    location:      Point<f64, Logical>,
    last_location: Point<f64, Logical>,
    held:          bool,
}

impl<E: Engine> Pointer<E> {

    pub fn new (
        logger:  &Logger,
        handle:  PointerHandle<Charlie<E>>,
        texture: Gles2Texture
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:        logger.clone(),
            status:        Arc::new(Mutex::new(Status::Default)),
            location:      (100.0, 30.0).into(),
            last_location: (100.0, 30.0).into(),
            handle,
            texture,
            held: false
        })
    }

    /// Render this pointer
    pub fn render <'a> (
        &mut self,
        frame:  &mut Gles2Frame<'a>,
        size:   &Size<i32, Physical>,
        screen: &ScreenState
    ) -> StdResult<()> {
        let damage = Rectangle::<i32, Physical>::from_loc_and_size(
            Point::<i32, Physical>::from((0i32, 0i32)),
            *size
        );
        let x = self.location.x;
        let y = self.location.y;
        let location = Point::<f64, Logical>::from((x, y)).to_physical(1.0).to_i32_round();
        //let size = self.texture.size();
        Ok(frame.render_texture_at(
            &self.texture,
            location,
            1,
            1.0,
            Transform::Normal,
            &[damage],
            1.0
        )?)
    }

    fn status (&self) -> (bool, Point<f64, Logical>) {
        let mut reset = false;
        let mut guard = self.status.lock().unwrap();
        if let Status::Surface(ref surface) = *guard {
            reset = !surface.alive();
        }
        if reset {
            *guard = Status::Default;
        }
        let visible = !matches!(*guard, Status::Surface(_));
        let hotspot = if let Status::Surface(ref surface) = *guard {
            with_states(surface, |states| {
                states.data_map.get::<Mutex<Attributes>>().unwrap().lock().unwrap().hotspot
            })
        } else {
            (0, 0).into()
        };
        let location = self.location - hotspot.to_f64();
        (visible, location)
    }

    pub fn on_move_relative<B: InputBackend>(
        state: &mut Charlie<E>,
        index: usize,
        event: B::PointerMotionEvent,
        screen_id: usize
    ) {
        let delta = event.delta();
        panic!("{:?}", delta);
    }

    pub fn on_move_absolute<B: InputBackend>(
        state: &mut Charlie<E>,
        index: usize,
        event: B::PointerMotionAbsoluteEvent,
        screen_id: usize
    ) {
        let pointer = &mut state.input.pointers[index];
        pointer.last_location = pointer.location;
        pointer.location = (event.x(), event.y()).into();
        if pointer.held {
            crit!(state.logger, "CLECK! {screen_id}");
            let dx = pointer.location.x - pointer.last_location.x;
            let dy = pointer.location.y - pointer.last_location.y;
            state.desktop.screens[screen_id].center.x += dx as f64;
            state.desktop.screens[screen_id].center.y += dy as f64;
        } else {
            pointer.handle.clone().motion(state, None, &MotionEvent {
                location: (event.x(), event.y()).into(),
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time()
            })
        }
        //self.pointer.motion(
            //self.location,
            //under,
            //SERIAL_COUNTER.next_serial(),
            //evt.time()
        //);
    }

    pub fn on_button<B: InputBackend>(
        state: &mut Charlie<E>,
        index: usize,
        event: B::PointerButtonEvent,
        screen_id: usize
    ) {
        match event.state() {
            ButtonState::Pressed => {
                crit!(state.logger, "CLICK! {screen_id}");
                state.input.pointers[index].held = true;
            },
            ButtonState::Released => {
                crit!(state.logger, "CLACK! {screen_id}");
                state.input.pointers[index].held = false;
            }
        }
        //self.desktop.borrow_mut();
        //let serial = SCOUNTER.next_serial();
        //let button = match evt.button() {
            //MouseButton::Left => 0x110,
            //MouseButton::Right => 0x111,
            //MouseButton::Middle => 0x112,
            //MouseButton::Other(b) => b as u32,
        //};
        //let state = match evt.state() {
            //ButtonState::Pressed => {
                //// change the keyboard focus unless the pointer is grabbed
                //if !self.pointer.is_grabbed() {
                    //let pos   = self.pointer_location - self.workspace.borrow().offset.to_logical(1.0);
                    //let under = self.compositor.borrow().window_map.borrow().get_surface_under(pos);
                    //if under.is_some() {
                        //let under = self.compositor.borrow().window_map.borrow_mut()
                            //.get_surface_and_bring_to_top(pos);
                        //self.keyboard
                            //.set_focus(under.as_ref().map(|&(ref s, _)| s), serial);
                    //} else {
                        //self.workspace.borrow_mut().dragging = true;
                    //}
                //}
                //wl_pointer::ButtonState::Pressed
            //}
            //ButtonState::Released => {
                //self.workspace.borrow_mut().dragging = false;
                //wl_pointer::ButtonState::Released
            //},
        //};
        //self.pointer.button(button, state, serial, evt.time());
    }

    pub fn on_axis<B: InputBackend>(
        state: &mut Charlie<E>,
        index: usize,
        event: B::PointerAxisEvent,
        screen_id: usize
    ) {
        //let source = match evt.source() {
            //AxisSource::Continuous => wl_pointer::AxisSource::Continuous,
            //AxisSource::Finger => wl_pointer::AxisSource::Finger,
            //AxisSource::Wheel | AxisSource::WheelTilt => wl_pointer::AxisSource::Wheel,
        //};

        //let mut frame = AxisFrame::new(evt.time()).source(source);

        //let horizontal_amount = evt.amount(Axis::Horizontal)
            //.unwrap_or_else(|| evt.amount_discrete(Axis::Horizontal).unwrap() * 3.0);
        //let horizontal_amount_discrete = evt.amount_discrete(Axis::Horizontal);
        //if horizontal_amount != 0.0 {
            //frame = frame.value(wl_pointer::Axis::HorizontalScroll, horizontal_amount);
            //if let Some(discrete) = horizontal_amount_discrete {
                //frame = frame.discrete(wl_pointer::Axis::HorizontalScroll, discrete as i32);
            //}
        //} else if source == wl_pointer::AxisSource::Finger {
            //frame = frame.stop(wl_pointer::Axis::HorizontalScroll);
        //}

        //let vertical_amount = evt.amount(Axis::Vertical)
            //.unwrap_or_else(|| evt.amount_discrete(Axis::Vertical).unwrap() * 3.0);
        //let vertical_amount_discrete = evt.amount_discrete(Axis::Vertical);
        //if vertical_amount != 0.0 {
            //frame = frame.value(wl_pointer::Axis::VerticalScroll, vertical_amount);
            //if let Some(discrete) = vertical_amount_discrete {
                //frame = frame.discrete(wl_pointer::Axis::VerticalScroll, discrete as i32);
            //}
        //} else if source == wl_pointer::AxisSource::Finger {
            //frame = frame.stop(wl_pointer::Axis::VerticalScroll);
        //}

        //self.pointer.axis(frame);
    }

}
