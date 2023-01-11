use super::prelude::*;

use smithay::{
    backend::input::{
        Event,
        //AbsolutePositionEvent,
        PointerButtonEvent,
        PointerMotionEvent,
        PointerAxisEvent
    },
    input::{
        pointer::{
            PointerHandle,
            CursorImageStatus     as Status,
            CursorImageAttributes as Attributes
        }
    }
};

pub struct Pointer {
    logger:        Logger,
    pub pointer:   PointerHandle<AppState>,
    pub texture:   Gles2Texture,
    status:        Arc<Mutex<Status>>,
    location:      Point<f64, Logical>,
    last_location: Point<f64, Logical>,
    held:          bool,
}

impl Pointer {

    pub fn new (
        logger:  &Logger,
        pointer: PointerHandle<AppState>,
        texture: Gles2Texture
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:        logger.clone(),
            status:        Arc::new(Mutex::new(Status::Default)),
            location:      (100.0, 30.0).into(),
            last_location: (100.0, 30.0).into(),
            pointer,
            texture,
            held: false
        })
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

    pub fn render (
        &self,
        frame:  &mut Gles2Frame,
        size:   Size<i32, Physical>,
        screen: &ScreenState
    ) -> Result<(), Box<dyn Error>> {
        let damage = Rectangle::<i32, Physical>::from_loc_and_size(
            Point::<i32, Physical>::from((0i32, 0i32)),
            size
        );
        let x = self.location.x + screen.center().x;
        let y = self.location.y + screen.center().y;
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

    pub fn on_move_relative<B: InputBackend>(
        state: &mut AppState,
        index: usize,
        event: B::PointerMotionEvent,
        screen_id: usize
    ) {
        let delta = event.delta();
        panic!("{:?}", delta);
    }

    pub fn on_move_absolute<B: InputBackend>(
        state: &mut AppState,
        index: usize,
        event: B::PointerMotionAbsoluteEvent,
        screen_id: usize
    ) {
        let pointer = &mut state.pointers[index];
        pointer.last_location = pointer.location;
        pointer.location = (event.x(), event.y()).into();
        if pointer.held {
            crit!(state.logger, "CLECK! {screen_id}");
            let dx = pointer.location.x - pointer.last_location.x;
            let dy = pointer.location.y - pointer.last_location.y;
            state.desktop.screens[screen_id].center.x += dx as f64;
            state.desktop.screens[screen_id].center.y += dy as f64;
        } else {
            pointer.pointer.clone().motion(state, None, &MotionEvent {
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
        state: &mut AppState,
        index: usize,
        event: B::PointerButtonEvent,
        screen_id: usize
    ) {
        match event.state() {
            ButtonState::Pressed => {
                crit!(state.logger, "CLICK! {screen_id}");
                state.pointers[index].held = true;
            },
            ButtonState::Released => {
                crit!(state.logger, "CLACK! {screen_id}");
                state.pointers[index].held = false;
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
        state: &mut AppState,
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
