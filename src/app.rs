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
{
    _update: PhantomData<U>,
    _render: PhantomData<R>,
    logger:  Logger,
    display: Display<(E, S)>,
    events:  EventLoop<'static, (E, S)>,
    engine:  E,
    state:   S,
}

impl<E, S, U, R> App<E, S, U, R> where
    E: Engine<'static, U, R, S>,
    S: Widget<'static, U, R> + 'static,
{

    pub fn new () -> StdResult<Self> {
        // Create the environment
        let (logger, events, display) = Self::init()?;
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

    fn init () -> StdResult<(Logger, EventLoop<'static, (E, S)>, Display<(E, S)>)> {
        // Create the logger
        let (logger, _guard) = init_log();
        // Create the event loop
        let events = EventLoop::<'static, (E, S)>::try_new()?;
        // Create the display
        let display = Display::<(E, S)>::new()?;
        Ok((logger, events, display))
    }

    /// Run an instance of an application.
    pub fn run (&mut self) -> StdResult<()> {
        // Listen on socket and expose it as env var
        let socket_name = self.listen()?;
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);
        // Run main loop
        loop {
            // Respond to user input
            if let Err(e) = self.engine.update(self.state) {
                crit!(self.logger, "Update error: {e}");
                break
            }
            // Render display
            if let Err(e) = self.engine.render(&mut self.state) {
                crit!(self.logger, "Render error: {e}");
                break
            }
            // Flush display/client messages
            self.display.flush_clients()?;
            // Dispatch state to next event loop tick
            self.events.dispatch(
                Some(Duration::from_millis(1)),
                &mut (self.engine, self.state)
            );
        }
    }

    fn listen (&self) -> StdResult<std::ffi::OsString> {
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
        let socket = ListeningSocketSource::new_auto(self.logger.clone()).unwrap();
        // Listen for new clients
        let socket_logger  = self.logger.clone();
        let socket_display = self.display.handle();
        self.events.handle().insert_source(socket, move |client, _, _| {
            debug!(socket_logger, "New client {client:?}");
            socket_display.insert_client(
                client.try_clone().expect("Could not clone socket for engine dispatcher"),
                Arc::new(ClientState)
            ).expect("Could not insert client in engine display");
        });
        Ok(socket.socket_name().to_os_string())
    }

}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {
    }
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {
    }
}

