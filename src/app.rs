use crate::prelude::*;
use smithay::{
    wayland::socket::ListeningSocketSource,
    reexports::wayland_server::backend::{ClientId, ClientData, DisconnectReason},
    reexports::calloop::{PostAction, Interest, Mode, generic::Generic}
};

/// Binds an Engine representing the runtime environment
/// to a root Widget representing the application state.
pub struct App<E, S, U, R> where
    E: Engine<'static, U, R, S>,
    S: Widget<'static, U, R> + 'static,
    U: 'static,
    R: 'static
{
    _update: PhantomData<U>,
    _render: PhantomData<R>,
    logger:  Logger,
    display: Display<Shared<Self>>,
    events:  EventLoop<'static, Shared<Self>>,
    engine:  E,
    state:   S,
}

impl<E, S, U, R> App<E, S, U, R> where
    E: Engine<'static, U, R, S>,
    S: Widget<'static, U, R> + 'static,
{

    pub fn new () -> StdResult<Self> {
        // Create the logger
        let (logger, _guard) = init_log();
        // Create the event loop
        let events = EventLoop::<'static, (E, S)>::try_new()?;
        // Create the display
        let display = Display::<(E, S)>::new()?;
        // Create the engine
        let engine = E::new(&logger, &display.handle())?;
        // Create the state
        let state = S::new(&logger, &display.handle(), &events.handle())?;
        Ok(Self {
            _update: PhantomData::default(),
            _render: PhantomData::default(),
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
        let app = Rc::new(RefCell::new(self));
        // Run main loop
        loop {
            // Respond to user input
            if let Err(e) = app.engine.update(app.state) {
                crit!(logger, "Update error: {e}");
                break
            }
            // Render display
            if let Err(e) = app.engine.render(&mut app.state) {
                crit!(logger, "Render error: {e}");
                break
            }
            // Flush display/client messages
            app.display.flush_clients()?;
            // Dispatch state to next event loop tick
            app.events.dispatch(Some(Duration::from_millis(1)), app.clone());
        }
    }

}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {}
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

