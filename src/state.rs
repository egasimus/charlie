mod prelude;
mod wayland;
mod handle;
mod pointer;
mod xwayland;

use self::prelude::*;
use self::wayland::WaylandListener;
use self::handle::DelegatedState;
use self::pointer::Pointer;
use self::xwayland::XWaylandState;

pub struct State {
    logger:  Logger,
    /// A wayland socket listener
    wayland: WaylandListener,
    /// A collection of views into the workspace, bound to engine outputs
    screens: Vec<Screen>,
    /// State of the workspace containing the windows
    pub space:     Space<Window>,
    /// State of the mouse pointer
    pub pointer:   Pointer,
    /// State of the X11 integration.
    pub xwayland:  XWaylandState,
    /// States of smithay-provided implementations of compositor features
    pub delegated: DelegatedState
}

impl State {

    pub fn new (engine: &mut impl Engine<Self>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:    engine.logger(),
            screens:   vec![],
            space:     Space::new(engine.logger()),
            pointer:   Pointer::new(engine)?,
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
        })
    }

    pub fn render (
        &self,
        renderer: &mut Gles2Renderer,
        damage:   &mut DamageTrackedRenderer,
        size:     Size<i32, Physical>,
        output:   &Output,
        screen:   usize
    ) -> Result<(), Box<dyn Error>> {
        let rect: Rectangle<i32, Physical> = Rectangle::from_loc_and_size((0, 0), size);
        use smithay::desktop::space::space_render_elements;
        let screen = &self.screens[screen];
        let clear_color = [0.2,0.3,0.4,1.0];
        let elements = space_render_elements(renderer, [&self.space], output)?;
        damage.render_output(renderer, 0, &elements, clear_color, self.logger.clone())?;
        //let mut frame = renderer.render(size, Transform::Normal)?;
        //frame.clear(clear_color, &[rect])?;
        //self.pointer.render(&mut frame, size, screen)?;
        //frame.finish()?;
        //self
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
}
