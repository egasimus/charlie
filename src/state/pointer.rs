use super::prelude::*;

use smithay::{
    backend::input::{
        AbsolutePositionEvent,
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
    pointer:       PointerHandle<State>
    pub texture:   Gles2Texture,
    status:        Arc<Mutex<Status>>,
    position:      Point<f64, Logical>,
    last_position: Point<f64, Logical>,
}

impl Pointer {

    pub fn new (
        logger:  &Logger,
        pointer: PointerHandle<State>,
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
        self.last_position = self.position;
        self.position = evt.position_transformed(
            self.compositor.borrow().find_by_name(OUTPUT_NAME)
                .map(|o| o.size()).unwrap());
        self.workspace.borrow_mut()
            .on_move_absolute(self.pointer_location, self.last_pointer_location);
        let pos    = self.pointer_location - self.workspace.borrow().offset.to_logical(1.0);
        let under  = self.compositor.borrow().window_map.borrow().get_surface_under(pos);
        self.pointer.motion(
            self.position,
            under,
            SERIAL_COUNTER.next_serial(),
            evt.time()
        );
    }

    pub fn on_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
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

    pub fn on_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
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

}

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<State>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<State> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<i32, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        //data.space
            //.map_element(self.window.clone(), new_location.to_i32_round(), true);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(data, event.serial, event.time);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn start_data(&self) -> &PointerGrabStartData<State> {
        &self.start_data
    }
}

bitflags::bitflags! {
    pub struct ResizeEdge: u32 {
        const TOP          = 0b0001;
        const BOTTOM       = 0b0010;
        const LEFT         = 0b0100;
        const RIGHT        = 0b1000;

        const TOP_LEFT     = Self::TOP.bits | Self::LEFT.bits;
        const BOTTOM_LEFT  = Self::BOTTOM.bits | Self::LEFT.bits;

        const TOP_RIGHT    = Self::TOP.bits | Self::RIGHT.bits;
        const BOTTOM_RIGHT = Self::BOTTOM.bits | Self::RIGHT.bits;
    }
}

impl From<XdgToplevelResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: XdgToplevelResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

pub struct ResizeSurfaceGrab {
    start_data: PointerGrabStartData<State>,
    window: Window,

    edges: ResizeEdge,

    initial_rect: Rectangle<i32, Logical>,
    last_window_size: Size<i32, Logical>,
}

impl ResizeSurfaceGrab {
    pub fn start(
        start_data: PointerGrabStartData<State>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        let initial_rect = initial_window_rect;

        ResizeSurfaceState::with(window.toplevel().wl_surface(), |state| {
            *state = ResizeSurfaceState::Resizing { edges, initial_rect };
        });

        Self {
            start_data,
            window,
            edges,
            initial_rect,
            last_window_size: initial_rect.size,
        }
    }
}

impl PointerGrab<State> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<i32, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let mut delta = event.location - self.start_data.location;

        let mut new_window_width = self.initial_rect.size.w;
        let mut new_window_height = self.initial_rect.size.h;

        if self.edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                delta.x = -delta.x;
            }

            new_window_width = (self.initial_rect.size.w as f64 + delta.x) as i32;
        }

        if self.edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            if self.edges.intersects(ResizeEdge::TOP) {
                delta.y = -delta.y;
            }

            new_window_height = (self.initial_rect.size.h as f64 + delta.y) as i32;
        }

        let (min_size, max_size) = compositor::with_states(self.window.toplevel().wl_surface(), |states| {
            let data = states.cached_state.current::<SurfaceCachedState>();
            (data.min_size, data.max_size)
        });

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);

        let max_width = (max_size.w == 0).then(i32::max_value).unwrap_or(max_size.w);
        let max_height = (max_size.h == 0).then(i32::max_value).unwrap_or(max_size.h);

        self.last_window_size = Size::from((
            new_window_width.max(min_width).min(max_width),
            new_window_height.max(min_height).min(max_height),
        ));

        if let Kind::Xdg(xdg) = self.window.toplevel() {
            xdg.with_pending_state(|state| {
                state.states.set(XdgToplevelState::Resizing);
                state.size = Some(self.last_window_size);
            });

            xdg.send_configure();
        }
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(data, event.serial, event.time);

            if let Kind::Xdg(xdg) = self.window.toplevel() {
                xdg.with_pending_state(|state| {
                    state.states.unset(XdgToplevelState::Resizing);
                    state.size = Some(self.last_window_size);
                });

                xdg.send_configure();

                ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                    *state = ResizeSurfaceState::WaitingForLastCommit {
                        edges: self.edges,
                        initial_rect: self.initial_rect,
                    };
                });
            }
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn start_data(&self) -> &PointerGrabStartData<State> {
        &self.start_data
    }
}

/// State of the resize operation.
///
/// It is stored inside of WlSurface,
/// and can be accessed using [`ResizeSurfaceState::with`]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ResizeSurfaceState {
    Idle,
    Resizing {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
    /// Resize is done, we are now waiting for last commit, to do the final move
    WaitingForLastCommit {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
}

impl Default for ResizeSurfaceState {
    fn default() -> Self {
        ResizeSurfaceState::Idle
    }
}

impl ResizeSurfaceState {
    pub fn with <T> (surface: &WlSurface, cb: impl FnOnce(&mut Self) -> T) -> T {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            let state = states.data_map.get::<RefCell<Self>>().unwrap();
            cb(&mut state.borrow_mut())
        })
    }

    pub fn commit(&mut self) -> Option<(ResizeEdge, Rectangle<i32, Logical>)> {
        match *self {
            Self::Resizing { edges, initial_rect } => Some((edges, initial_rect)),
            Self::WaitingForLastCommit { edges, initial_rect } => {
                // The resize is done, let's go back to idle
                *self = Self::Idle;

                Some((edges, initial_rect))
            }
            Self::Idle => None,
        }
    }
}

pub(crate) fn check_grab(
    seat: &Seat<State>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<State>> {
    let pointer = seat.get_pointer()?;

    // Check that this surface has a click grab.
    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;

    let (focus, _) = start_data.focus.as_ref()?;
    // If the focus was for a different surface, ignore the request.
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}
