use crate::prelude::*;
use smithay::{
    wayland::socket::ListeningSocketSource,
    reexports::wayland_server::backend::{ClientId, ClientData, DisconnectReason},
    reexports::calloop::{PostAction, Interest, Mode, generic::Generic}
};

/// Binds an Engine representing the runtime environment
/// to a root Widget representing the application state.
pub struct App<E: Engine, W: Widget + 'static> where {
    logger:  Logger,
    display: Display<Self>,
    events:  EventLoop<'static, Self>,
    pub engine: E,
    pub state:  W,
}

impl<E: Engine, W: Widget + 'static> App<E, W> {

    pub fn new () -> StdResult<Self> {
        // Create the logger
        let (logger, _guard) = init_log();
        // Create the event loop
        let events = EventLoop::try_new()?;
        // Create the display
        let display = Display::new()?;
        // Create the engine
        let engine = E::new::<W>(&logger, &display.handle())?;
        // Create the state
        let state = W::new(&logger, &display.handle(), &events.handle())?;
        Ok(Self {
            logger,
            display,
            events,
            engine,
            state,
        })
    }

    /// Perform a procedure with this app instance as part of a method call chain.
    pub fn with (&mut self, cb: impl Fn(&mut Self)->StdResult<&mut Self>) -> StdResult<&mut Self> {
        cb(self)
    }

    /// Run an instance of an application.
    pub fn run (self: Self) -> StdResult<()> {
        let logger = self.logger.clone();
        // Listen for events
        let fd = self.display.backend().poll_fd().as_raw_fd();
        self.events.handle().insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            move |_, _, mut state| {
                self.display.dispatch_clients(state)?;
                Ok(PostAction::Continue)
            }
        );
        // Create a socket
        let socket = ListeningSocketSource::new_auto(logger.clone()).unwrap();
        // Listen for new clients
        let socket_logger  = logger.clone();
        let socket_display = self.display.handle();
        self.events.handle().insert_source(socket, move |client, _, _| {
            debug!(socket_logger, "New client {client:?}");
            socket_display.insert_client(
                client.try_clone().expect("Could not clone socket for engine dispatcher"),
                Arc::new(ClientState)
            ).expect("Could not insert client in engine display");
        });
        let socket_name = socket.socket_name().to_os_string();
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);
        // Put the whole struct in a smart pointer
        // Run main loop
        loop {
            // Respond to user input
            if let Err(e) = self.engine.update(self.state) {
                crit!(logger, "Update error: {e}");
                break
            }
            // Render display
            if let Err(e) = self.engine.render(&mut self.state) {
                crit!(logger, "Render error: {e}");
                break
            }
            // Flush display/client messages
            self.display.flush_clients()?;
            // Dispatch state to next event loop tick
            self.events.dispatch(Some(Duration::from_millis(1)), &mut self);
        }
        Ok(())
    }

}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {}
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

