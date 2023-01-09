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
    pub delegated: DelegatedState,
    /// Commands to run after successful initialization
    startup: Vec<(String, Vec<String>)>,
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
            startup:   vec![],
        })
    }

    pub fn screen_add (&mut self, screen: Screen) -> usize {
        self.screens.push(screen);
        self.screens.len() - 1
    }

    pub fn startup_add (&mut self, command: &str, args: &[&str]) -> usize {
        self.startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self.startup.len() - 1
    }

    pub fn seat_add (&mut self, name: impl Into<String>) -> Result<Seat<Self>, Box<dyn Error>> {
        use smithay::input::keyboard::XkbConfig;
        use smithay::wayland::input_method::InputMethodSeat;
        let mut seat = self.delegated.seat_add(name);
        seat.add_pointer();
        seat.add_keyboard(XkbConfig::default(), 200, 25)?;
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(seat)
    }

}

type ScreenId = usize;

impl<'a> Widget for State {

    type RenderData = ScreenId;

    fn prepare (&mut self) -> Result<(), Box<dyn Error>> {
        println!("DISPLAY={:?}", ::std::env::var("DISPLAY"));
        println!("WAYLAND_DISPLAY={:?}", ::std::env::var("WAYLAND_DISPLAY"));
        println!("{:?}", self.startup);
        for (cmd, args) in self.startup.iter() {
            debug!(self.logger, "Spawning {cmd} {args:?}");
            std::process::Command::new(cmd).args(args).spawn()?;
        }
        Ok(())
    }

    fn render <'r> (&'r self, context: RenderContext<'r, ScreenId>) -> Result<(), Box<dyn Error>> {
        let RenderContext { renderer, output, data: screen } = context;
        let size      = output.current_mode().unwrap().size;
        let transform = output.current_transform();
        let scale     = output.current_scale();
        let rect: Rectangle<i32, Physical> = Rectangle::from_loc_and_size((0, 0), size);
        use smithay::desktop::space::space_render_elements;
        let screen = &self.screens[screen];
        let elements = space_render_elements(renderer, [&self.space], output)?;
        let mut frame = renderer.render(size, transform)?;
        frame.clear([0.2,0.3,0.4,1.0], &[rect]);
        for (mut z_index, element) in elements.iter().rev().enumerate() {
            // This is necessary because we reversed the render elements to draw
            // them back to front, but z-index including opaque regions is defined
            // front to back
            z_index = elements.len() - 1 - z_index;
            let element_geometry = element.geometry(scale.fractional_scale().into());
            element.draw(&mut frame, element.src(), element_geometry, &[element_geometry], &self.logger)?;
        }
        frame.finish();
        Ok(())
    }

    fn handle <B: InputBackend> (&mut self, event: InputEvent<B>) {
        debug!(self.logger, "Received input event")
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
