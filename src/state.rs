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

pub struct App<E: Engine<State=S>, S> {
    logger: Logger,
    engine: E,
    state:  Rc<RefCell<S>>,
}

impl<E: Engine<State=AppState>> App<E, AppState> {

    /// Create a new application instance.
    pub fn new (mut engine: E) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger: engine.logger(),
            state:  Rc::new(RefCell::new(AppState::new(&mut engine)?)),
            engine
        })
    }

    /// Add a command to run on startup.
    /// TODO: Integrate a `systemd --user` session
    pub fn startup (&mut self, command: &str, args: &[&str]) -> &mut Self {
        self.state.borrow_mut().startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self
    }

    /// Add a viewport into the workspace.
    pub fn output (&mut self, name: &str, w: i32, h: i32, x: f64, y: f64) -> Result<&mut Self, Box<dyn Error>> {
        let screen = self.state.borrow_mut().desktop.screen_add(ScreenState::new((x, y), (w as f64, h as f64)));
        self.engine.output_add(name, screen)?;
        Ok(self)
    }

    /// Add a control seat over the workspace.
    pub fn input (&mut self, name: &str, cursor: &str) -> Result<&mut Self, Box<dyn Error>> {
        {
            let mut state = self.state.borrow_mut();
            let mut seat = state.delegated.seat.new_wl_seat(
                &self.engine.display_handle(),
                String::from(name),
                self.logger.clone()
            );
            state.pointers.push(Pointer::new(
                &self.logger,
                &self.state,
                seat.add_pointer(),
                import_bitmap(self.engine.renderer(), cursor)?
            )?);
            state.keyboards.push(Keyboard::new(
                &self.logger,
                seat.add_keyboard(XkbConfig::default(), 200, 25)?
            ));
            seat.add_input_method(XkbConfig::default(), 200, 25);
        }
        Ok(self)
    }

    pub fn screen_add (&self, screen: ScreenState) -> usize {
        let AppState { desktop, .. } = &mut *self.state.borrow_mut();
        desktop.screen_add(screen)
    }

    /// Add a window to the workspace.
    pub fn window_add (&self, window: Window) -> usize {
        let AppState { desktop, .. } = &mut *self.state.borrow_mut();
        desktop.window_add(window)
    }

    pub fn seat_add (
        &self,
        name: impl Into<String>,
        pointer_image: Gles2Texture,
    ) -> Result<Seat<AppState>, Box<dyn Error>> {
        let AppState { pointers, keyboards, delegated, .. } = &mut *self.state.borrow_mut();
        let mut seat = delegated.seat.new_wl_seat(
            &delegated.display_handle,
            name.into(),
            self.logger.clone()
        );
        pointers.push(Pointer::new(
            &self.logger,
            &self.state,
            seat.add_pointer(),
            pointer_image
        )?);
        keyboards.push(Keyboard::new(
            &self.logger,
            seat.add_keyboard(XkbConfig::default(), 200, 25)?
        ));
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(seat)
    }

    pub fn start (&mut self) -> Result<(), Box<dyn Error>> {
        self.engine.start(&mut *self.state.borrow_mut())
    }
}

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
    desktop:       Desktop,
    /// State of the mouse pointer(s)
    pointers:      Vec<Pointer>,
    /// State of the keyboard(s)
    keyboards:     Vec<Keyboard>,
}

impl AppState {

    pub fn new (engine: &mut impl Engine<State=Self>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:    engine.logger(),
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
            desktop:   Desktop::new(engine),
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
        let AppState { desktop, pointers, delegated, .. } = &*self;
        let RenderContext { renderer, output, data: (screen, screen_size) } = context;
        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );
        desktop.import(renderer)?;
        let mut frame = renderer.render(size, transform)?;
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
        let AppState { pointers, keyboards, .. } = &mut *self;
        match event {
            InputEvent::PointerMotion { event, .. }
                => pointers[0].on_move_relative::<B>(event),
            InputEvent::PointerButton { event, .. }
                => pointers[0].on_button::<B>(event),
            InputEvent::PointerAxis { event, .. }
                => pointers[0].on_axis::<B>(event),
            InputEvent::Keyboard { event, .. }
                => keyboards[0].on_keyboard::<B>(event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                debug!(self.logger, "{} {}", event.x(), event.y());
                //let event = MotionEvent { location: 
                //self.pointers[0].on_move_absolute::<B>(event)
            },
            _ => {}
        }
    }

}
