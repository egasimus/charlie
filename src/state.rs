mod prelude;
mod wayland;
mod handle;
mod window;
mod pointer;
mod keyboard;
mod xwayland;

use self::prelude::*;
use self::wayland::WaylandListener;
use self::handle::DelegatedState;
use self::window::WindowState;
use self::pointer::Pointer;
use self::keyboard::Keyboard;
use self::xwayland::XWaylandState;

pub struct State {
    logger: Logger,
    /// A collection of windows that are mapped across the screens
    windows: Vec<WindowState>,
    /// A collection of views into the workspace, bound to engine outputs
    screens: Vec<ScreenState>,
    /// State of the mouse pointer(s)
    pointers: Vec<Pointer>,
    /// State of the keyboard(s)
    keyboards: Vec<Keyboard>,
    /// A wayland socket listener
    wayland: WaylandListener,
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
            windows:   vec![],
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
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
    pub fn screen_add (&mut self, screen: ScreenState) -> usize {
        self.screens.push(screen);
        self.screens.len() - 1
    }

    /// Add an control seat over the workspace.
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

    /// Add a window to the workspace.
    pub fn window_add (&mut self, window: Window) -> usize {
        self.windows.push(WindowState::new(window));
        self.windows.len() - 1
    }

    /// Find a window by its top level surface.
    pub fn window_find (&self, surface: &WlSurface) -> Option<&Window> {
        self.windows.iter()
            .find(|w| w.window.toplevel().wl_surface() == surface)
            .map(|w|&w.window)
    }

}

type ScreenId = usize;

impl Widget for State {

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

        for window in self.windows.iter() {
            window.import(&self.logger, renderer)?;
        }

        let mut frame = renderer.render(size, transform)?;

        frame.clear([0.2,0.3,0.4,1.0], &[Rectangle::from_loc_and_size((0, 0), size)])?;

        for window in self.windows.iter() {
            window.render(&self.logger, &mut frame, size)?;
        }

        for pointer in self.pointers.iter() {
            pointer.render(&mut frame, size, &self.screens[screen])?;
        }

        frame.finish()?;

        for window in self.windows.iter() {
            window.window.send_frame(
                output,
                Duration::from(self.delegated.clock.now()),
                Some(Duration::from_secs(1)),
                smithay::desktop::utils::surface_primary_scanout_output
            );
        }

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
