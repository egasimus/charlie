use crate::prelude::*;
use crate::pointer::Pointer;
use crate::xwayland::XWaylandState;
use smithay::backend::input::{InputBackend, InputEvent};

pub struct State {
    logger:       Logger,
    screens:      Vec<Screen>,
    windows:      Vec<Window>,
    pub pointer:  Pointer,
    pub xwayland: XWaylandState
}

impl State {

    pub fn new  (
        logger:   &Logger,
        engine:   &mut impl Engine,
        xwayland: XWaylandState
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:  logger.clone(),
            screens: vec![],
            windows: vec![],
            pointer: Pointer::new(engine)?,
            xwayland
        })
    }

    pub fn render (
        &self, frame: &mut Gles2Frame, size: Size<i32, Physical>, screen: usize
    ) -> Result<(), Box<dyn Error>> {
        let screen = &self.screens[screen];
        self.pointer.render(frame, size, screen)?;
        //for screen in self.screens.iter() {
            //for window in self.windows.iter() {
                //if screen.contains_rect(window) {
                    ////engine.render_window(screen, window)?;
                //}
            //}
            ////if screen.contains_point(self.pointer) {
                ////engine.render_pointer(screen, &self.pointer)?;
            ////}
        //}
        Ok(())
    }

    pub fn on_input <B: InputBackend> (&mut self, event: InputEvent<B>) {
        debug!(self.logger, "Received input event")
    }

    pub fn screen_add (&mut self, screen: Screen) -> usize {
        self.screens.push(screen);
        self.screens.len() - 1
    }

}

pub struct Screen {
    center: Point<f64, Logical>,
    size:   Size<f64, Logical>
}

impl Screen {
    pub fn new (
        center: impl Into<Point<f64, Logical>>,
        size:   impl Into<Size<f64, Logical>>
    ) -> Self {
        Self { center: center.into(), size: size.into() }
    }
    #[inline]
    pub fn center (&self) -> &Point<f64, Logical> {
        &self.center
    }
    fn contains_rect (&self, window: &Window) -> bool {
        false
    }
    fn contains_point (&self, point: Point<f64, Logical>) -> bool {
        false
    }
}

pub struct Window {
    location: Point<f64, Logical>,
    size:     Size<f64, Logical>
}
