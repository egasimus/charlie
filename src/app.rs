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
    display: Rc<RefCell<Display<Self>>>,
    events:  Rc<RefCell<EventLoop<'static, Self>>>,
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
            engine,
            state,
            display: Rc::new(RefCell::new(display)),
            events:  Rc::new(RefCell::new(events)),
        })
    }

    /// Perform a procedure with this app instance as part of a method call chain.
    pub fn with (self, cb: impl Fn(Self)->StdResult<Self>) -> StdResult<Self> {
        cb(self)
    }

    /// Run an instance of an application.
    pub fn run (mut self) -> StdResult<()> {
        let logger = self.logger.clone();
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
        let socket = ListeningSocketSource::new_auto(logger.clone()).unwrap();
        let socket_name = socket.socket_name().to_os_string();
        // Listen for new clients
        let socket_logger  = logger.clone();
        let mut socket_display = self.display.borrow().handle();
        self.events.borrow().handle().insert_source(socket, move |client, _, _| {
            debug!(socket_logger, "New client {client:?}");
            socket_display.insert_client(
                client.try_clone().expect("Could not clone socket for engine dispatcher"),
                Arc::new(ClientState)
            ).expect("Could not insert client in engine display");
        });
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);
        // Put the whole struct in a smart pointer
        // Run main loop
        let display = self.display.clone();
        let events  = self.events.clone();
        loop {
            // Respond to user input
            if let Err(e) = self.engine.update(&mut self.state) {
                crit!(logger, "Update error: {e}");
                break
            }
            // Render display
            if let Err(e) = self.engine.render(&mut self.state) {
                crit!(logger, "Render error: {e}");
                break
            }
            // Flush display/client messages
            display.borrow_mut().flush_clients()?;
            // Dispatch state to next event loop tick
            events.borrow_mut().dispatch(Some(Duration::from_millis(1)), &mut self);
        }
        Ok(())
    }

}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {}
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

