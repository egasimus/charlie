mod prelude;
mod wayland;
mod handle;
mod desktop;
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

pub struct App {
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

impl App {

    pub fn new (engine: &mut impl Engine<State=Self>) -> Result<Self, Box<dyn Error>> {
        let desktop = Rc::new(RefCell::new(Desktop::new(engine.logger())));
        Ok(Self {
            logger:    engine.logger(),
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
            desktop,
            pointers:  vec![],
            keyboards: vec![],
            startup:   vec![],
        })
    }

    pub fn ready (&mut self) -> Result<(), Box<dyn Error>> {
        debug!(self.logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
        debug!(self.logger, "WAYLAND_DISPLAY={:?}", ::std::env::var("WAYLAND_DISPLAY"));
        debug!(self.logger, "{:?}", self.startup);
        for (cmd, args) in self.startup.iter() {
            debug!(self.logger, "Spawning {cmd} {args:?}");
            std::process::Command::new(cmd).args(args).spawn()?;
        }
        Ok(())
    }

    /// Add a command to run on startup.
    /// TODO: Integrate a `systemd --user` session
    pub fn startup_add (&mut self, command: &str, args: &[&str]) -> usize {
        self.startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self.startup.len() - 1
    }

    /// Add a viewport into the workspace.
    pub fn screen_add (&self, screen: ScreenState) -> usize {
        self.desktop.borrow_mut().screen_add(screen)
    }

    /// Add a window to the workspace.
    pub fn window_add (&self, window: Window) -> usize {
        self.desktop.borrow_mut().window_add(window)
    }

    /// Add a control seat over the workspace.
    pub fn seat_add (
        &mut self,
        name: impl Into<String>,
        pointer_image: Gles2Texture,
    ) -> Result<Seat<Self>, Box<dyn Error>> {
        use smithay::input::keyboard::XkbConfig;
        use smithay::wayland::input_method::InputMethodSeat;
        let mut seat = self.delegated.seat_add(name);
        self.pointers.push(Pointer::new(
            &self.logger,
            seat.add_pointer(),
            pointer_image
        )?);
        self.keyboards.push(Keyboard::new(
            &self.logger,
            seat.add_keyboard(XkbConfig::default(), 200, 25)?
        ));
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(seat)
    }

}

type ScreenId = usize;

impl Widget for App {

    type RenderData = ScreenId;

    fn render <'r> (
        &'r self, context: RenderContext<'r, Self::RenderData>
    ) -> Result<(), Box<dyn Error>> {
        let RenderContext { renderer, output, data: screen } = context;
        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );
        let desktop = self.desktop.borrow_mut();
        desktop.import(renderer)?;
        let mut frame = renderer.render(size, transform)?;
        frame.clear([0.2,0.3,0.4,1.0], &[Rectangle::from_loc_and_size((0, 0), size)])?;
        desktop.render(&mut frame, size)?;
        for pointer in self.pointers.iter() {
            pointer.render(&mut frame, size, &desktop.screens[screen])?;
        }
        frame.finish()?;
        desktop.tick(output, self.delegated.clock.now());
        Ok(())
    }

    fn handle <B: InputBackend> (&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::PointerMotion { event, .. }
                => self.pointers[0].on_move_relative::<B>(event),
            InputEvent::PointerMotionAbsolute { event, .. }
                => self.pointers[0].on_move_absolute::<B>(event),
            InputEvent::PointerButton { event, .. }
                => self.pointers[0].on_button::<B>(event),
            InputEvent::PointerAxis { event, .. }
                => self.pointers[0].on_axis::<B>(event),
            InputEvent::Keyboard { event, .. }
                => self.keyboards[0].on_keyboard::<B>(event),
            _ => {}
        }
    }

}

pub struct ScreenState {
    center: Point<f64, Logical>,
    size:   Size<f64, Logical>
}

impl ScreenState {
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
