mod prelude;
pub mod desktop;
mod input;
pub mod xwayland;

use self::prelude::*;
use self::desktop::Desktop;
use self::input::Input;

use smithay::{
    wayland::socket::ListeningSocketSource,
    reexports::wayland_server::backend::{ClientId, ClientData, DisconnectReason},
    reexports::calloop::{PostAction, Interest, Mode, generic::Generic}
};

/// Contains the compositor state.
pub struct App<E: Engine> {
    pub logger:  Logger,
    pub display: Rc<RefCell<Display<Self>>>,
    pub events:  Rc<RefCell<EventLoop<'static, Self>>>,
    /// Commands to run after successful initialization
    pub startup: Vec<(String, Vec<String>)>,
    /// The collection of windows and their layouts
    pub desktop: Desktop,
    /// The collection of input devices
    pub input:   Input,
    /// Engine-specific state
    pub engine:  E,
}

impl<E: Engine> App<E> {

    pub fn new () -> StdResult<Self> {

        // Create the logger
        let (logger, _guard) = init_log();

        // Create the event loop
        let events = EventLoop::try_new()?;

        // Create the display
        let display = Display::new()?;

        // Create the engine
        let engine = E::new(&logger, &display.handle())?;

        Ok(Self {
            logger,
            engine,
            display: Rc::new(RefCell::new(display)),
            events:  Rc::new(RefCell::new(events)),
            startup: vec![],
            desktop: Desktop::new::<Self>(logger, display)?,
            input:   Input::new(logger, display)?,
        })
    }

    /// Perform a procedure with this app instance as part of a method call chain.
    pub fn with (self, cb: impl Fn(Self)->StdResult<Self>) -> StdResult<Self> {
        cb(self)
    }

    /// Run an instance of an application.
    pub fn run (mut self) -> StdResult<()> {

        // Listen for events
        let display = self.display.clone();
        let fd = display.borrow_mut().backend().poll_fd().as_raw_fd();
        self.events.borrow().handle().insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            move |_, _, state| {
                display.borrow_mut().dispatch_clients(state)?;
                Ok(PostAction::Continue)
            }
        );

        // Create a socket
        let socket = ListeningSocketSource::new_auto(self.logger.clone()).unwrap();
        let socket_name = socket.socket_name().to_os_string();

        // Listen for new clients
        let socket_logger  = self.logger.clone();
        let mut socket_display = self.display.borrow().handle();
        self.events.borrow().handle().insert_source(socket, move |client, _, _| {
            debug!(socket_logger, "New client {client:?}");
            socket_display.insert_client(
                client.try_clone().expect("Could not clone socket for engine dispatcher"),
                Arc::new(ClientState)
            ).expect("Could not insert client in engine display");
        });
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);

        // Run main loop
        let display = self.display.clone();
        let events  = self.events.clone();

        loop {

            // Respond to user input
            if let Err(e) = self.engine.update(&mut self) {
                crit!(self.logger, "Update error: {e}");
                break
            }

            // Render display
            if let Err(e) = self.engine.render(&mut self) {
                crit!(self.logger, "Render error: {e}");
                break
            }

            // Flush display/client messages
            display.borrow_mut().flush_clients()?;

            // Dispatch state to next event loop tick
            events.borrow_mut().dispatch(Some(Duration::from_millis(1)), &mut self);
        }

        Ok(())
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

    fn new <E: Engine> (
        logger:  &Logger,
        display: &DisplayHandle,
        events:  &LoopHandle<'static, App<E>>
    )
        -> Result<Self, Box<dyn Error>>
    {
        // Init xwayland
        crate::state::xwayland::init_xwayland(
            logger, events, display,
            Box::new(|x|Ok(()))//x.1.ready())
        )?;
        Ok(Self {
            logger:  logger.clone(),
            desktop: Desktop::new::<E>(logger, display)?,
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

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {}
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
