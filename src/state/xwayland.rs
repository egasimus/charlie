use super::prelude::*;

use std::{collections::HashMap, convert::TryFrom, os::unix::net::UnixStream, sync::Arc};

use x11rb::protocol::xproto::{ConfigureRequestEvent, ClientMessageEvent};

atom_manager! {
    Atoms: AtomsCookie {
        WM_S0,
        WL_SURFACE_ID,
        _ANVIL_CLOSE_CONNECTION,
    }
}

pub type Unpaired = HashMap<u32, (X11Window, Point<i32, Logical>)>;

pub fn init_xwayland <T> (
    logger:  &Logger,
    events:  &LoopHandle<'static, T>,
    display: &DisplayHandle,
    ready:   Box<dyn Fn(&mut T)->Result<(), Box<dyn Error>>>
) -> Result<(), Box<dyn Error>> {
    let (xwayland, channel) = XWayland::new(logger.clone(), &display);
    let cb_logger  = logger.clone();
    let cb_events  = events.clone();
    let cb_display = display.clone();
    events.insert_source(channel, move |event, _, app| match event {
        XWaylandEvent::Ready { connection, client, .. } => {
            let (x11conn, x11atoms, x11source) = x11_connect(&cb_logger, &cb_display.clone(), connection)
                .unwrap();
            let mut unpaired: Unpaired = Default::default();
            cb_events.clone().insert_source(x11source, move |event, _, state| {
                debug!(cb_logger, "X11: Got event {:?}", event);
                x11_handle(
                    &cb_logger,
                    &cb_display.clone(),
                    &client,
                    &x11conn,
                    x11atoms,
                    event, 
                    &mut unpaired
                ).unwrap();
            });
            debug!(cb_logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
            ready(app).unwrap()
        },
        XWaylandEvent::Exited => {
            crit!(cb_logger, "XWayland exited")
        },
    })?;
    xwayland.start(events.clone())?;
    Ok(())
}

pub fn x11_handle (
    logger:   &Logger,
    display:  &DisplayHandle,
    client:   &Client,
    conn:     &Arc<RustConnection>,
    atoms:    Atoms,
    event:    X11Event,
    unpaired: &mut Unpaired,
) -> Result<(), ReplyOrIdError> {
    debug!(logger, "X11: Got event {:?}", event);
    match event {
        X11Event::ConfigureRequest(r) => { x11_configure(conn, r)?; }
        X11Event::MapRequest(r) => { conn.map_window(r.window)?; }
        X11Event::ClientMessage(msg) => { x11_client_message(logger, display, client, &conn, msg, atoms, unpaired)?; }
        _ => {}
    }
    conn.flush()?;
    Ok(())
}


pub fn x11_connect (
    logger:     &Logger,
    display:    &DisplayHandle,
    connection: UnixStream,
) -> Result<(Arc<RustConnection>, Atoms, X11Source), Box<dyn Error>> {
    debug!(logger, "New X11 connection");
    let screen = 0; // Create an X11 connection. XWaylandState only uses screen 0.
    let stream = DefaultStream::from_unix_stream(connection)?;
    let conn   = RustConnection::connect_to_stream(stream, 0)?;
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
    //let unpaired = Default::default();
    Ok((conn.clone(), atoms, X11Source::new(conn, win, atoms._ANVIL_CLOSE_CONNECTION, logger.clone())))
}

pub fn x11_configure (
    conn: &Arc<RustConnection>,
    r:    ConfigureRequestEvent
) -> Result<(), ReplyOrIdError> {
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
    conn.configure_window(r.window, &aux)?;
    Ok(())
}

pub fn x11_client_message (
    logger:   &Logger,
    display:  &DisplayHandle,
    client:   &Client,
    conn:     &Arc<RustConnection>,
    msg:      ClientMessageEvent,
    atoms:    Atoms,
    unpaired: &mut Unpaired
) -> Result<(), ReplyOrIdError> {
    if msg.type_ == atoms.WL_SURFACE_ID {
        // We get a WL_SURFACE_ID message when Xwayland creates a WlSurface for a
        // window. Both the creation of the surface and this client message happen at
        // roughly the same time and are sent over different sockets (X11 socket and
        // wayland socket). Thus, we could receive these two in any order. Hence, it
        // can happen that we get None below when X11 was faster than Wayland.
        let location = {
            match conn.get_geometry(msg.window)?.reply() {
                Ok(geo) => (geo.x as i32, geo.y as i32).into(),
                Err(err) => {
                    error!(
                        logger,
                        "Failed to get geometry for {:x}, perhaps the window was already destroyed?",
                        msg.window;
                        "err" => format!("{:?}", err),
                    );
                    (0, 0).into()
                }
            }
        };
        let id = msg.data.as_data32()[0];
        let surface = client.object_from_protocol_id(display, id);
        match surface {
            Err(_) => {
                unpaired.insert(id, (msg.window, location));
            }
            Ok(surface) => {
                debug!(
                    logger,
                    "X11 surface {:x?} corresponds to WlSurface {:x} = {:?}",
                    msg.window,
                    id,
                    surface,
                );
                x11_new_window(logger, msg.window, surface, location);
            }
        }
    }
    Ok(())
}

pub fn x11_new_window (
    logger:   &Logger,
    window:   X11Window,
    surface:  WlSurface,
    location: Point<i32, Logical>,
    //space:    &mut Space<Window>,
) {
    debug!(logger, "Matched X11 surface {:x?} to {:x?}", window, surface);
    if give_role(&surface, "x11_surface").is_err() {
        // It makes no sense to post a protocol error here since that would only kill Xwayland
        error!(logger, "Surface {:x?} already has a role?!", surface);
        return;
    }
    let x11surface = X11Surface { surface };
    //space.map_element(Window::new(Kind::X11(x11surface)), location, true);
}

// Called when a WlSurface commits.
//pub fn commit_hook (
    //surface: &WlSurface,
    //dh:      &DisplayHandle,
    //state:   &mut XWaylandConnection,
//) {
    //if let Ok(client) = dh.get_client(surface.id()) {
        //// Is this the Xwayland client?
        //if client == state.client {
            //// Is the surface among the unpaired surfaces (see comment next to WL_SURFACE_ID
            //// handling above)
            //if let Some((window, location)) = state.unpaired.remove(&surface.id().protocol_id()) {
                //state.new_window(window, surface.clone(), location);
            //}
        //}
    //}
//}
