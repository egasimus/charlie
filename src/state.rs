mod prelude;
pub mod desktop;
mod input;
pub mod xwayland;

use self::prelude::*;
use self::desktop::Desktop;
use self::input::Input;

/// Contains the compositor state.
pub struct AppState {
    logger:      Logger,
    /// Commands to run after successful initialization
    startup:     Vec<(String, Vec<String>)>,
    /// The collection of windows and their layouts
    pub desktop: Desktop,
    /// The collection of input devices
    pub input:   Input
}

impl AppState {

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

    pub fn startup (&mut self, cmd: impl AsRef<str>, args: &[&str]) -> StdResult<&mut Self> {
        Ok(self)
    }

    pub fn output (&mut self, cmd: impl AsRef<str>, w: i32, h: i32, x: f64, y: f64) -> StdResult<&mut Self> {
        Ok(self)
    }

    pub fn input (&mut self, cmd: impl AsRef<str>, cursor: impl AsRef<str>) -> StdResult<&mut Self> {
        Ok(self)
    }

}

impl Widget for AppState {
    fn new <T: 'static> (
        logger:  &Logger,
        display: &DisplayHandle,
        events:  &LoopHandle<'static, T>
    ) -> Result<Self, Box<dyn Error>> {
        // Init xwayland
        crate::state::xwayland::init_xwayland(
            logger, events, display,
            Box::new(|x|Ok(()))//x.1.ready())
        )?;
        Ok(Self {
            logger:  logger.clone(),
            desktop: Desktop::new(logger, display)?,
            input:   Input::new(logger, display)?,
            startup: vec![],
        })
    }

    /// Render the desktop and pointer for this output
    fn render (
        &mut self,
        renderer: &mut Gles2Renderer,
        output:   &Output,
        size:     &Size<i32, Physical>,
        screen:   ScreenId
    ) -> StdResult<()> {

        // Get the render parameters
        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );

        // Import window surfaces
        self.desktop.import(renderer)?;

        // Begin frame
        let mut frame = renderer.render(size, Transform::Flipped180)?;

        // Clear frame
        frame.clear([0.2, 0.3, 0.4, 1.0], &[Rectangle::from_loc_and_size((0, 0), size)])?;

        // Render window surfaces
        self.desktop.render(&mut frame, screen, size)?;

        // Render pointers
        for pointer in self.input.pointers.iter_mut() {
            pointer.render(&mut frame, &size, &self.desktop.screens[screen])?;
        }

        // End frame
        frame.finish()?;

        // Advance time
        self.desktop.send_frames(output);

        Ok(())

    }
}
