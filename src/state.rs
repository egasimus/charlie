mod prelude;
mod wayland;
mod handle;
pub mod desktop;
mod pointer;
mod keyboard;
mod xwayland;

use self::prelude::*;
use self::wayland::WaylandListener;
use self::handle::DelegatedState;
use self::desktop::Desktop;
use self::pointer::Pointer;
use self::keyboard::Keyboard;
use self::xwayland::XWaylandState;

/// Couples an engine to a state struct.
pub struct App<S> {
    logger: Logger,
    engine: Box<dyn Engine<State=S>>,
    state:  S,
}

/// Contains the compositor state.
pub struct AppState {
    logger:        Logger,
    /// A wayland socket listener
    wayland:       WaylandListener,
    /// State of the X11 integration.
    pub xwayland:  XWaylandState,
    /// States of smithay-provided implementations of compositor features
    pub delegated: DelegatedState,
    /// Commands to run after successful initialization
    startup:       Vec<(String, Vec<String>)>,
    /// The collection of windows and their layouts
    desktop:       Rc<RefCell<Desktop>>,
    /// State of the mouse pointer(s)
    pointers:      Vec<Pointer>,
    /// State of the keyboard(s)
    keyboards:     Vec<Keyboard>,
}

impl App<AppState> {

    /// Create a new application instance.
    pub fn new (mut engine: impl Engine<State=AppState>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger: engine.logger(),
            state:  AppState::new(&mut engine)?,
            engine: Box::new(engine)
        })
    }

    /// Add a command to run on startup.
    /// TODO: Integrate a `systemd --user` session
    pub fn startup (&mut self, command: &str, args: &[&str]) -> &mut Self {
        self.state.startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self
    }

    /// Add a viewport into the workspace.
    pub fn output (&mut self, name: &str, w: i32, h: i32, x: f64, y: f64) -> Result<&mut Self, Box<dyn Error>> {
        let screen = self.state.desktop.borrow_mut().screen_add(ScreenState::new((x, y), (w as f64, h as f64)));
        self.engine.output_add(name, screen, w, h)?;
        Ok(self)
    }

    /// Add a control seat over the workspace.
    pub fn input (&mut self, name: &str, cursor: &str) -> Result<&mut Self, Box<dyn Error>> {
        let mut seat = self.state.delegated.seat.new_wl_seat(
            &self.engine.display_handle(),
            String::from(name),
            self.logger.clone()
        );
        self.state.pointers.push(Pointer::new(
            &self.logger,
            seat.add_pointer(),
            import_bitmap(self.engine.renderer(), cursor)?
        )?);
        self.state.keyboards.push(Keyboard::new(
            &self.logger,
            seat.add_keyboard(XkbConfig::default(), 200, 25)?
        ));
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(self)
    }

    pub fn start (&mut self) -> Result<(), Box<dyn Error>> {
        self.engine.start(&mut self.state)
    }

}

impl AppState {

    pub fn new (engine: &mut impl Engine<State=Self>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:    engine.logger(),
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
            desktop:   Rc::new(RefCell::new(Desktop::new(engine))),
            pointers:  vec![],
            keyboards: vec![],
            startup:   vec![],
        })
    }

    /// When the app is ready to run, this spawns the startup processes.
    pub fn ready (&self) -> Result<(), Box<dyn Error>> {
        debug!(self.logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
        debug!(self.logger, "WAYLAND_DISPLAY={:?}", ::std::env::var("WAYLAND_DISPLAY"));
        debug!(self.logger, "{:?}", self.startup);
        for (cmd, args) in self.startup.iter() {
            debug!(self.logger, "Spawning {cmd} {args:?}");
            std::process::Command::new(cmd).args(args).spawn()?;
        }
        Ok(())
    }

}

type ScreenId = usize;

impl Widget for AppState {

    type RenderData = (ScreenId, Size<i32, Physical>);

    fn render <'r> (
        &'r self, context: RenderContext<'r, Self::RenderData>
    ) -> Result<(), Box<dyn Error>> {
        let AppState { desktop, pointers, delegated, .. } = &self;
        let RenderContext { renderer, output, data: (screen, screen_size) } = context;
        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );
        let desktop = desktop.borrow_mut();
        desktop.import(renderer)?;
        let mut frame = renderer.render(size, Transform::Flipped180)?;
        frame.clear([0.2,0.3,0.4,1.0], &[Rectangle::from_loc_and_size((0, 0), size)])?;
        desktop.render(&mut frame, size)?;
        for pointer in pointers.iter() {
            pointer.render(&mut frame, size, &desktop.screens[screen])?;
        }
        frame.finish()?;
        desktop.tick(output, delegated.clock.now());
        Ok(())
    }

    fn handle <B: InputBackend> (&mut self, event: InputEvent<B>) {
        //let state    = &mut self.state;
        //let pointer  = &self.state.pointers[0];
        //let keyboard = &self.state.keyboards[0];
        match event {
            InputEvent::PointerMotion { event, .. }
                => Pointer::on_move_relative::<B>(self, 0, event),
            InputEvent::PointerMotionAbsolute { event, .. }
                => Pointer::on_move_absolute::<B>(self, 0, event),
            InputEvent::PointerButton { event, .. }
                => Pointer::on_button::<B>(self, 0, event),
            InputEvent::PointerAxis { event, .. }
                => Pointer::on_axis::<B>(self, 0, event),
            InputEvent::Keyboard { event, .. }
                => Keyboard::on_key::<B>(self, 0, event),
            _ => {}
        }
    }

}
