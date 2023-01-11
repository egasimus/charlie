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
    pointer:       PointerHandle<App>,
    pub texture:   Gles2Texture,
    status:        Arc<Mutex<Status>>,
    position:      Point<f64, Logical>,
    last_position: Point<f64, Logical>,
}

impl Pointer {

    pub fn new (
        logger:  &Logger,
        pointer: PointerHandle<App>,
        texture: Gles2Texture
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:        logger.clone(),
            status:        Arc::new(Mutex::new(Status::Default)),
            position:      (100.0, 30.0).into(),
            last_position: (100.0, 30.0).into(),
            pointer,
            texture,
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
        let position = self.position - hotspot.to_f64();
        (visible, position)
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
        let x = self.position.x + screen.center().x;
        let y = self.position.y + screen.center().y;
        let position = Point::<f64, Logical>::from((x, y)).to_physical(1.0).to_i32_round();
        //let size = self.texture.size();
        Ok(frame.render_texture_at(
            &self.texture,
            position,
            1,
            1.0,
            Transform::Flipped180,
            &[damage],
            1.0
        )?)
    }

    pub fn on_move_relative<B: InputBackend>(&mut self, evt: B::PointerMotionEvent) {
        let delta = evt.delta();
        panic!("{:?}", delta);
    }

    pub fn on_move_absolute<B: InputBackend>(&mut self, evt: B::PointerMotionAbsoluteEvent) {
        //self.pointer.motion(
            //self.position,
            //under,
            //SERIAL_COUNTER.next_serial(),
            //evt.time()
        //);
    }

    pub fn on_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
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

    pub fn on_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
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
