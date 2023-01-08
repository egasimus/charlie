use crate::prelude::*;

use smithay::backend::input::{InputBackend, InputEvent};

pub struct State {
    logger:       Logger,
    screens:      Vec<Screen>,
    windows:      Vec<Window>,
    pointer:      Point<f64, Logical>,
    pointer_last: Point<f64, Logical>
}

impl State {

    pub fn new (logger: &Logger) -> Self {
        Self {
            logger:  logger.clone(),
            screens: vec![],
            windows: vec![],
            pointer: (0.0, 0.0).into(),
            pointer_last: (0.0, 0.0).into(),
        }
    }

    fn render (&self, engine: &mut impl Engine) -> Result<(), Box<dyn Error>> {
        for screen in self.screens.iter() {
            for window in self.windows.iter() {
                if screen.contains_rect(window) {
                    engine.render_window(screen, window)?;
                }
            }
            if screen.contains_point(self.pointer) {
                engine.render_pointer(screen, &self.pointer)?;
            }
        }
        Ok(())
    }

    pub fn on_input <B: InputBackend> (&mut self, event: InputEvent<B>) {
        debug!(self.logger, "Received input event")
    }

}


pub struct Screen {
    location: Point<f64, Logical>,
    size:     Size<f64, Logical>
}

impl Screen {
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
