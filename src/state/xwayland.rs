use super::prelude::*;

use std::{collections::HashMap, convert::TryFrom, os::unix::net::UnixStream, sync::Arc};

atom_manager! {
    Atoms: AtomsCookie {
        WM_S0,
        WL_SURFACE_ID,
        _ANVIL_CLOSE_CONNECTION,
    }
}

pub struct XWaylandState {
    logger:    Logger,
    events:    LoopHandle<'static, App>,
    xwayland:  XWayland,
    connected: Option<XWaylandConnection>
}

impl XWaylandState {

    pub fn new (engine: &impl Engine<State=App>) -> Result<Self, Box<dyn Error>> {
        let logger  = engine.logger();
        let events  = engine.event_handle();
        let display = engine.display_handle();
        let (xwayland, channel) = XWayland::new(logger.clone(), &display.clone());
        let display = display.clone();
        events.insert_source(channel, move |event, _, state| match event {
            XWaylandEvent::Ready { connection, client, .. } => {
                state.xwayland.connect(&display, connection, client).unwrap();
                state.ready().unwrap()
            },
            XWaylandEvent::Exited => {
                state.xwayland.exited()
            },
        })?;
        xwayland.start(events.clone())?;
        Ok(Self {
            logger: logger.clone(),
            events,
            xwayland,
            connected: None
        })
    }

    pub fn connect (
        &mut self,
        display:    &DisplayHandle,
        connection: UnixStream,
        client:     Client
    ) -> Result<(), Box<dyn Error>> {
        let logger = &self.logger;
        let events = &self.events;
        let connection = XWaylandConnection::new(&logger, display, &events, connection, client)?;
        self.connected = Some(connection);
        debug!(self.logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
        Ok(())
    }

    pub fn exited (&mut self) {
        error!(self.logger, "XWayland crashed");
    }
}

pub struct XWaylandConnection {
    logger:   Logger,
    conn:     Arc<RustConnection>,
    atoms:    Atoms,
    client:   Client,
    unpaired: HashMap<u32, (X11Window, Point<i32, Logical>)>,
}

impl XWaylandConnection {

    pub fn new (
        logger:     &Logger,
        display:    &DisplayHandle,
        events:     &LoopHandle<'static, App>,
        connection: UnixStream,
        client:     Client
    ) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "New X11 connection");
        let screen = 0; // Create an X11 connection. XWaylandState only uses screen 0.
        let stream = DefaultStream::from_unix_stream(connection)?;
        let conn   = RustConnection::connect_to_stream(stream, screen)?;
        let atoms  = Atoms::new(&conn)?.reply()?;
        let screen = &conn.setup().roots[0];
        // Actually become the WM by redirecting some operations
        conn.change_window_attributes(
            screen.root,
            &ChangeWindowAttributesAux::default().event_mask(EventMask::SUBSTRUCTURE_REDIRECT),
        )?;
        // Tell XWaylandState that we are the WM by acquiring the WM_S0 selection. No X11 clients are accepted before this.
        let win = conn.generate_id()?;
        conn.create_window(
            screen.root_depth, win, screen.root,
            // x, y, width, height, border width
            0, 0, 1, 1, 0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &Default::default(),
        )?;
        conn.set_selection_owner(win, atoms.WM_S0, x11rb::CURRENT_TIME)?;
        // XWaylandState wants us to do this to function properly...?
        conn.composite_redirect_subwindows(screen.root, Redirect::MANUAL)?;
        conn.flush()?;
        let conn = Arc::new(conn);
        let wm = Self {
            logger:   logger.clone(),
            conn:     Arc::clone(&conn),
            unpaired: Default::default(),
            atoms,
            client,
        };
        let source = X11Source::new(conn, win, atoms._ANVIL_CLOSE_CONNECTION, logger.clone());
        let log = logger.clone();
        let display = display.clone();
        events.insert_source(source, move |event, _, state| {
            if let Some(x11) = state.xwayland.connected.as_mut() {
                match x11.handle_event(event, &display) {
                    Ok(()) => {}
                    Err(err) => error!(log, "Error while handling X11 event: {}", err),
                }
            }
        })?;
        info!(logger, "XWayland is ready!");
        Ok(wm)
    }

    fn handle_event (
        &mut self, event: Event, dh: &DisplayHandle//, space: &mut Space<Window>
    ) -> Result<(), ReplyOrIdError> {
        debug!(self.logger, "X11: Got event {:?}", event);
        match event {
            Event::ConfigureRequest(r) => {
                // Just grant the wish
                let mut aux = ConfigureWindowAux::default();
                if r.value_mask & u16::from(ConfigWindow::STACK_MODE) != 0 {
                    aux = aux.stack_mode(r.stack_mode);
                }
                if r.value_mask & u16::from(ConfigWindow::SIBLING) != 0 {
                    aux = aux.sibling(r.sibling);
                }
                if r.value_mask & u16::from(ConfigWindow::X) != 0 {
                    aux = aux.x(i32::try_from(r.x).unwrap());
                }
                if r.value_mask & u16::from(ConfigWindow::Y) != 0 {
                    aux = aux.y(i32::try_from(r.y).unwrap());
                }
                if r.value_mask & u16::from(ConfigWindow::WIDTH) != 0 {
                    aux = aux.width(u32::try_from(r.width).unwrap());
                }
                if r.value_mask & u16::from(ConfigWindow::HEIGHT) != 0 {
                    aux = aux.height(u32::try_from(r.height).unwrap());
                }
                if r.value_mask & u16::from(ConfigWindow::BORDER_WIDTH) != 0 {
                    aux = aux.border_width(u32::try_from(r.border_width).unwrap());
                }
                self.conn.configure_window(r.window, &aux)?;
            }
            Event::MapRequest(r) => {
                // Just grant the wish
                self.conn.map_window(r.window)?;
            }
            Event::ClientMessage(msg) => {
                if msg.type_ == self.atoms.WL_SURFACE_ID {
                    // We get a WL_SURFACE_ID message when Xwayland creates a WlSurface for a
                    // window. Both the creation of the surface and this client message happen at
                    // roughly the same time and are sent over different sockets (X11 socket and
                    // wayland socket). Thus, we could receive these two in any order. Hence, it
                    // can happen that we get None below when X11 was faster than Wayland.

                    let location = {
                        match self.conn.get_geometry(msg.window)?.reply() {
                            Ok(geo) => (geo.x as i32, geo.y as i32).into(),
                            Err(err) => {
                                error!(
                                    self.logger,
                                    "Failed to get geometry for {:x}, perhaps the window was already destroyed?",
                                    msg.window;
                                    "err" => format!("{:?}", err),
                                );
                                (0, 0).into()
                            }
                        }
                    };

                    let id = msg.data.as_data32()[0];
                    let surface = self.client.object_from_protocol_id(dh, id);

                    match surface {
                        Err(_) => {
                            self.unpaired.insert(id, (msg.window, location));
                        }
                        Ok(surface) => {
                            debug!(
                                self.logger,
                                "X11 surface {:x?} corresponds to WlSurface {:x} = {:?}",
                                msg.window,
                                id,
                                surface,
                            );
                            self.new_window(msg.window, surface, location);
                        }
                    }
                }
            }
            _ => {}
        }
        self.conn.flush()?;
        Ok(())
    }

    fn new_window(
        &mut self,
        window:   X11Window,
        surface:  WlSurface,
        location: Point<i32, Logical>,
        //space:    &mut Space<Window>,
    ) {
        debug!(self.logger, "Matched X11 surface {:x?} to {:x?}", window, surface);

        if give_role(&surface, "x11_surface").is_err() {
            // It makes no sense to post a protocol error here since that would only kill Xwayland
            error!(self.logger, "Surface {:x?} already has a role?!", surface);
            return;
        }

        let x11surface = X11Surface { surface };
        //space.map_element(Window::new(Kind::X11(x11surface)), location, true);
    }
}

// Called when a WlSurface commits.
pub fn commit_hook (
    surface: &WlSurface,
    dh:      &DisplayHandle,
    state:   &mut XWaylandConnection,
) {
    if let Ok(client) = dh.get_client(surface.id()) {
        // Is this the Xwayland client?
        if client == state.client {
            // Is the surface among the unpaired surfaces (see comment next to WL_SURFACE_ID
            // handling above)
            if let Some((window, location)) = state.unpaired.remove(&surface.id().protocol_id()) {
                state.new_window(window, surface.clone(), location);
            }
        }
    }
}
