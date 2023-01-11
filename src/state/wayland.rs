use super::prelude::*;

use std::ffi::OsString;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::reexports::calloop::{PostAction, Interest, Mode, generic::Generic};
use smithay::reexports::wayland_server::backend::{ClientId, ClientData, DisconnectReason};

pub struct WaylandListener(OsString);

impl WaylandListener {
    pub fn new (engine: &impl Engine) -> Result<Self, Box<dyn Error>> {
        let socket = ListeningSocketSource::new_auto(engine.logger()).unwrap();
        let name = socket.socket_name().to_os_string();
        let handle = engine.event_handle();
        let mut display = engine.display_handle();
        let logger = engine.logger();
        handle.insert_source(socket, move |client_stream, _, _state| {
            debug!(logger, "New client");
            display.insert_client(client_stream, Arc::new(ClientState)).unwrap();
        })?;
        let dispatch = engine.display_dispatcher();
        let logger = engine.logger();
        handle.insert_source(
            Generic::new(engine.display_fd(), Interest::READ, Mode::Level),
            move |x, y, mut state| {
                dispatch(&mut state).unwrap();
                Ok(PostAction::Continue)
            }
        )?;
        ::std::env::set_var("WAYLAND_DISPLAY", &name);
        Ok(Self(name))
    }
}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {
    }
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {
    }
}
