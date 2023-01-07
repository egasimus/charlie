use crate::App;
use crate::prelude::*;
use crate::controller::{MoveSurfaceGrab, ResizeSurfaceGrab, ResizeState, ResizeData, ResizeEdge};
use crate::workspace::Workspace;

pub struct Compositor {
    pub log:        Logger,
    pub display:    Rc<RefCell<Display>>,
    pub outputs:    Vec<Output>,
    pub window_map: Rc<RefCell<WindowMap>>,
    pub xwayland:   XWayland<App>,
    pub x11state:   Option<X11State>,
}

impl Compositor {

    pub fn init (
        log:        &Logger,
        display:    &Rc<RefCell<Display>>,
        event_loop: &EventLoop<'static, App>,
    ) -> Result<Self, Box<dyn Error>> {

        compositor_init(&mut *display.borrow_mut(), move |surface, mut data| {
            data.get::<App>().unwrap().compositor.borrow_mut().commit(&surface)
        }, log.clone());

        let (xwayland, channel) = XWayland::new(event_loop.handle(), display.clone(), log.clone());

        let compositor = Self {
            log:        log.clone(),
            display:    Rc::clone(display),
            outputs:    vec![],
            window_map: Rc::new(RefCell::new(WindowMap::init(&log))),
            x11state:   None,
            xwayland,
        };

        let handle1 = event_loop.handle();
        let handle2 = event_loop.handle();
        handle1.insert_source(channel, move |event, _, state| {
            match event {
                XWaylandEvent::Ready { connection, client }
                    => state.x11_ready(connection, client, &handle2).unwrap(),
                XWaylandEvent::Exited
                    => state.compositor.borrow_mut().x11_exited().unwrap(),
            };
        })?;

        xdg_shell_init(&mut *display.borrow_mut(), move |event, mut state| {
            let compositor = state.get::<App>().unwrap().compositor.borrow_mut();
            match event {
                XdgRequest::NewToplevel { surface }
                    => compositor.xdg_new_toplevel(surface),
                XdgRequest::NewPopup { surface }
                    => compositor.xdg_new_popup(surface),
                XdgRequest::Move { surface, seat, serial, }
                    => compositor.xdg_move(&surface, seat, serial),
                XdgRequest::Resize { surface, seat, serial, edges }
                    => compositor.xdg_resize(&surface, seat, serial, edges),
                XdgRequest::AckConfigure { surface, configure: Configure::Toplevel(configure), .. }
                    => compositor.xdg_ack_configure(&surface, configure),
                XdgRequest::Fullscreen { surface, output, .. }
                    => compositor.xdg_fullscreen(&surface, output),
                XdgRequest::UnFullscreen { surface }
                    => compositor.xdg_unfullscreen(&surface),
                XdgRequest::Maximize { surface }
                    => compositor.xdg_maximize(&surface),
                XdgRequest::UnMaximize { surface }
                    => compositor.xdg_unmaximize(&surface),
                _ => (),
            };
        }, compositor.log.clone());

        let log = compositor.log.clone();
        wl_shell_init(&mut *display.borrow_mut(), move |req: ShellRequest, mut state| {
            let compositor = state.get::<App>().unwrap().compositor.borrow_mut();
            match req {
                ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Toplevel, }
                    => compositor.set_toplevel(surface),
                ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Fullscreen { output, .. } }
                    => compositor.set_fullscreen(surface, output),
                ShellRequest::Move { surface, seat, serial }
                    => compositor.shell_move(surface, seat, serial),
                ShellRequest::Resize { surface, seat, serial, edges, }
                    => compositor.shell_resize(surface, seat, serial, edges),
                    _ => (),
            }
        }, compositor.log.clone());

        Ok(compositor)
    }

    pub fn add_output (
        &mut self,
        name:     impl AsRef<str>,
        physical: PhysicalProperties,
        mode:     OutputMode
    ) -> &Output {
        // Append the output to the end of the existing
        // outputs by placing it after the current overall
        // width
        let location = (self.width(), 0);
        let output = Output::new(
            name,
            location.into(),
            &mut *self.display.borrow_mut(),
            physical,
            mode,
            self.log.clone(),
        );
        self.outputs.push(output);
        // We call arrange here albeit the output is only appended and
        // this would not affect windows, but arrange could re-organize
        // outputs from a configuration.
        self.arrange();
        self.outputs.last().unwrap()
    }

    pub fn width(&self) -> i32 {
        // This is a simplification, we only arrange the outputs on the y axis side-by-side
        // so that the total width is simply the sum of all output widths.
        self.outputs.iter().fold(0, |acc, output| acc + output.size().w)
    }

    pub fn arrange(&mut self) {
        // First recalculate the outputs location
        let mut output_x = 0;
        for output in self.outputs.iter_mut() {
            let output_x_shift = output_x - output.location.x;
            // If the scale changed we shift all windows on that output
            // so that the location of the window will stay the same on screen
            if output_x_shift != 0 {
                let mut window_map = self.window_map.borrow_mut();
                for surface in output.surfaces.iter() {
                    let toplevel = window_map.find(surface);
                    if let Some(toplevel) = toplevel {
                        let current_location = window_map.location(&toplevel);
                        if let Some(mut location) = current_location {
                            if output.geometry().contains(location) {
                                location.x += output_x_shift;
                                window_map.set_location(&toplevel, location);
                            }
                        }
                    }
                }
            }
            output.location.x = output_x;
            output.location.y = 0;
            output.output.change_current_state(None, None, None, Some(output.location));
            output_x += output.size().w;
        }
        // Check if any windows are now out of outputs range
        // and move them to the primary output
        let primary_output_location = self.with_primary().map(|o| o.location()).unwrap_or_default();
        let mut window_map = self.window_map.borrow_mut();
        // TODO: This is a bit unfortunate, we save the windows in a temp vector
        // cause we can not call window_map.set_location within the closure.
        let mut windows_to_move = Vec::new();
        window_map.with_windows_from_bottom_to_top(|kind, _, &bbox| {
            let within_outputs = self.outputs.iter().any(|o| o.geometry().overlaps(bbox));
            if !within_outputs {
                windows_to_move.push((kind.to_owned(), primary_output_location));
            }
        });
        for (window, location) in windows_to_move.drain(..) {
            window_map.set_location(&window, location);
        }
        // Update the size and location for maximized and fullscreen windows
        window_map.with_windows_from_bottom_to_top(|kind, location, _| {
            if let SurfaceKind::Xdg(xdg) = kind {
                if let Some(state) = xdg.current_state() {
                    if state.states.contains(xdg_toplevel::State::Maximized)
                        || state.states.contains(xdg_toplevel::State::Fullscreen)
                    {
                        let output_geometry = if let Some(output) = state.fullscreen_output.as_ref() {
                            self.find_by_output(output).map(|o| o.geometry())
                        } else {
                            self.find_by_position(location).map(|o| o.geometry())
                        };
                        if let Some(geometry) = output_geometry {
                            if location != geometry.loc {
                                windows_to_move.push((kind.to_owned(), geometry.loc));
                            }
                            let res = xdg.with_pending_state(|pending_state| {
                                pending_state.size = Some(geometry.size);
                            });
                            if res.is_ok() {
                                xdg.send_configure();
                            }
                        }
                    }
                }
            }
        });
        for (window, location) in windows_to_move.drain(..) {
            window_map.set_location(&window, location);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    pub fn find<F: FnMut(&&Output) -> bool>(&self, f: F) -> Option<&Output> {
        self.outputs.iter().find(f)
    }

    pub fn with_primary(&self) -> Option<&Output> {
        self.outputs.get(0)
    }

    pub fn find_by_output(&self, output: &wl_output::WlOutput) -> Option<&Output> {
        self.find(|o| o.output.owns(output))
    }

    pub fn find_by_name<N: AsRef<str>>(&self, name: N) -> Option<&Output> {
        self.find(|o| o.name == name.as_ref())
    }

    pub fn find_by_position(&self, position: Point<i32, Logical>) -> Option<&Output> {
        self.find(|o| o.geometry().contains(position))
    }

    pub fn find_by_index(&self, index: usize) -> Option<&Output> {
        self.outputs.get(index)
    }

    pub fn draw (
        &self, renderer: &mut Gles2Renderer, frame: &mut Gles2Frame, workspace: &Workspace
    )
        -> Result<(), SwapBuffersError>
    {
        for output in self.outputs.iter() {
            let mut geometry = output.geometry();
            let scale = output.scale();
            workspace.draw(frame, geometry.size, scale)?;
            let offset: Point<i32, Logical> = workspace.offset
                .to_logical(scale as f64)
                .to_i32_round();
            geometry.loc.x -= offset.x;
            geometry.loc.y -= offset.y;
            self.window_map.borrow().draw_windows(&self.log, renderer, frame, geometry, scale)?;
        }
        Ok(())
    }

    pub fn xdg_new_toplevel (&self, surface: ToplevelSurface) {
        // place the window at a random location on the primary output
        // or if there is not output in a [0;800]x[0;800] square
        let output_geometry = self.with_primary().map(|o| o.geometry())
            .unwrap_or_else(|| Rectangle::from_loc_and_size((0, 0), (800, 800)));
        let max_x = output_geometry.loc.x + (((output_geometry.size.w as f32) / 3.0) * 2.0) as i32;
        let max_y = output_geometry.loc.y + (((output_geometry.size.h as f32) / 3.0) * 2.0) as i32;
        let x_range = Uniform::new(output_geometry.loc.x, max_x);
        let y_range = Uniform::new(output_geometry.loc.y, max_y);
        let mut rng = rand::thread_rng();
        let x = x_range.sample(&mut rng);
        let y = y_range.sample(&mut rng);
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        self.window_map.borrow_mut().insert(SurfaceKind::Xdg(surface), (x, y).into());
    }

    pub fn xdg_new_popup (&self, surface: PopupSurface) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        self.window_map.borrow_mut().insert_popup(PopupKind::Xdg(surface));
    }

    pub fn grabbed (&self, surface: &WlSurface, seat: &WlSeat, serial: Serial)
        -> Option<(PointerHandle, GrabStartData)>
    {
        let pointer = Seat::from_resource(&seat).unwrap().get_pointer().unwrap();
        // Check that this surface has a click grab.
        if pointer.has_grab(serial) {
            let start_data = pointer.grab_start_data().unwrap();
            let focus      = start_data.focus.clone();
            // If the focus was for a different surface, ignore the request.
            if let Some((client, _)) = focus && client.as_ref().same_client_as(surface.as_ref()) {
                Some((pointer, start_data))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn xdg_move (&self, surface: &ToplevelSurface, seat: WlSeat, serial: Serial) {
        if let Some((pointer, start_data)) = self.grabbed(
            surface.get_surface().unwrap(), &seat, serial
        ) {
            let toplevel = SurfaceKind::Xdg(surface.clone());
            let mut initial_window_location = self.window_map.borrow().location(&toplevel).unwrap();
            // If surface is maximized then unmaximize it
            if let Some(current_state) = surface.current_state() {
                if current_state.states.contains(xdg_toplevel::State::Maximized) {
                    let fs_changed = surface.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.size = None;
                    });
                    if fs_changed.is_ok() {
                        surface.send_configure();
                        let pos = pointer.current_location();
                        initial_window_location = (pos.x as i32, pos.y as i32).into();
                    }
                }
            }
            pointer.set_grab(MoveSurfaceGrab {
                start_data,
                window_map: self.window_map.clone(),
                toplevel,
                initial_window_location,
            }, serial);
        }
    }

    pub fn shell_move (&self, surface: ShellSurface, seat: WlSeat, serial: Serial) {
        if let Some((pointer, start_data)) = self.grabbed(
            surface.get_surface().unwrap(), &seat, serial
        ) {
            let toplevel = SurfaceKind::Wl(surface);
            let initial_window_location = self.window_map.borrow().location(&toplevel).unwrap();
            pointer.set_grab(MoveSurfaceGrab {
                start_data, window_map: self.window_map.clone(), toplevel, initial_window_location,
            }, serial);
        }
    }

    pub fn xdg_resize (
        &self, surface: &ToplevelSurface, seat: WlSeat, serial: Serial, edges: XdgResizeEdge
    ) {
        if let Some((pointer, start_data)) = self.grabbed(
            surface.get_surface().unwrap(), &seat, serial
        ) {
            let toplevel = SurfaceKind::Xdg(surface.clone());
            let initial_window_location = self.window_map.borrow().location(&toplevel).unwrap();
            let geometry = self.window_map.borrow().geometry(&toplevel).unwrap();
            let initial_window_size = geometry.size;
            with_states(surface.get_surface().unwrap(), move |states| {
                states.data_map.get::<RefCell<SurfaceData>>().unwrap().borrow_mut().resize_state =
                    ResizeState::Resizing(ResizeData {
                        edges: edges.into(), initial_window_location, initial_window_size,
                    });
            }).unwrap();

            pointer.set_grab(ResizeSurfaceGrab {
                start_data,
                toplevel,
                edges: edges.into(),
                initial_window_size,
                last_window_size: initial_window_size,
            }, serial);
        }
    }
    
    pub fn shell_resize (
        &self, surface: ShellSurface, seat: WlSeat, serial: Serial, edges: Resize,
    ) {
        if let Some((pointer, start_data)) = self.grabbed(
            surface.get_surface().unwrap(), &seat, serial
        ) {
            let toplevel = SurfaceKind::Wl(surface.clone());
            let initial_window_location = self.window_map.borrow().location(&toplevel).unwrap();
            let geometry = self.window_map.borrow().geometry(&toplevel).unwrap();
            let initial_window_size = geometry.size;
            with_states(surface.get_surface().unwrap(), move |states| {
                states.data_map.get::<RefCell<SurfaceData>>().unwrap().borrow_mut().resize_state =
                    ResizeState::Resizing(ResizeData {
                        edges: edges.into(),
                        initial_window_location,
                        initial_window_size,
                    }); }).unwrap();
            let grab = ResizeSurfaceGrab {
                start_data,
                toplevel,
                edges: edges.into(),
                initial_window_size,
                last_window_size: initial_window_size,
            };
            pointer.set_grab(grab, serial);
        }
    }

    pub fn xdg_ack_configure (&self, surface: &WlSurface, configure: ToplevelConfigure) {
        let waiting_for_serial = with_states(&surface, |states| {
            if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                if let ResizeState::WaitingForFinalAck(_, serial) = data.borrow().resize_state {
                    return Some(serial);
                }
            }
            None
        }).unwrap();
        if let Some(serial) = waiting_for_serial {
            // When the resize grab is released the surface
            // resize state will be set to WaitingForFinalAck
            // and the client will receive a configure request
            // without the resize state to inform the client
            // resizing has finished. Here we will wait for
            // the client to acknowledge the end of the
            // resizing. To check if the surface was resizing
            // before sending the configure we need to use
            // the current state as the received acknowledge
            // will no longer have the resize state set
            if configure.serial >= serial && with_states(&surface, |states| states.data_map
                .get::<Mutex<XdgToplevelSurfaceRoleAttributes>>().unwrap()
                .lock().unwrap()
                .current.states.contains(xdg_toplevel::State::Resizing)).unwrap()
            {
                with_states(&surface, |states| {
                    let mut data = states.data_map.get::<RefCell<SurfaceData>>().unwrap()
                        .borrow_mut();
                    if let ResizeState::WaitingForFinalAck(resize_data, _) = data.resize_state {
                        data.resize_state = ResizeState::WaitingForCommit(resize_data);
                    } else {
                        unreachable!()
                    }
                })
                .unwrap();
            }
        }
    }

    pub fn xdg_fullscreen (&self, surface: &ToplevelSurface, output: Option<WlOutput>) {
        // NOTE: This is only one part of the solution. We can set the
        // location and configure size here, but the surface should be rendered fullscreen
        // independently from its buffer size
        let wl_surface = if let Some(surface) = surface.get_surface() {
            surface
        } else {
            // If there is no underlying surface just ignore the request
            return;
        };
        let output_geometry = self.fullscreen_output_geometry(wl_surface, output.as_ref());
        if let Some(geometry) = output_geometry {
            if let Some(surface) = surface.get_surface() {
                let mut window_map = self.window_map.borrow_mut();
                if let Some(kind) = window_map.find(surface) {
                    window_map.set_location(&kind, geometry.loc);
                }
            }
            let ret = surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Fullscreen);
                state.size = Some(geometry.size);
                state.fullscreen_output = output;
            });
            if ret.is_ok() {
                surface.send_configure();
            }
        }
    }

    pub fn xdg_unfullscreen (&self, surface: &ToplevelSurface) {
        let ret = surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.size = None;
            state.fullscreen_output = None;
        });
        if ret.is_ok() {
            surface.send_configure();
        }
    }

    pub fn xdg_maximize (&self, surface: &ToplevelSurface) {
        // NOTE: This should use layer-shell when it is implemented to
        // get the correct maximum size
        let output_geometry = {
            let window_map = self.window_map.borrow();
            surface.get_surface()
                .and_then(|s| window_map.find(s))
                .and_then(|k| window_map.location(&k))
                .and_then(|position| self.find_by_position(position).map(|o| o.geometry()))
        };
        if let Some(geometry) = output_geometry {
            if let Some(surface) = surface.get_surface() {
                let mut window_map = self.window_map.borrow_mut();
                if let Some(kind) = window_map.find(surface) {
                    window_map.set_location(&kind, geometry.loc);
                }
            }
            let ret = surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Maximized);
                state.size = Some(geometry.size);
            });
            if ret.is_ok() {
                surface.send_configure();
            }
        }
    }

    pub fn xdg_unmaximize (&self, surface: &ToplevelSurface) {
        let ret = surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.size = None;
        });
        if ret.is_ok() {
            surface.send_configure();
        }
    }

    /// place the window at a random location on the primary output
    /// or if there is not output in a [0;800]x[0;800] square
    pub fn set_toplevel (&self, surface: ShellSurface) {
        let output_geometry = self.with_primary().map(|o| o.geometry())
            .unwrap_or_else(|| Rectangle::from_loc_and_size((0, 0), (800, 800)));
        let max_x =
            output_geometry.loc.x + (((output_geometry.size.w as f32) / 3.0) * 2.0) as i32;
        let max_y =
            output_geometry.loc.y + (((output_geometry.size.h as f32) / 3.0) * 2.0) as i32;
        let x_range = Uniform::new(output_geometry.loc.x, max_x);
        let y_range = Uniform::new(output_geometry.loc.y, max_y);
        let mut rng = rand::thread_rng();
        let x = x_range.sample(&mut rng);
        let y = y_range.sample(&mut rng);
        self.window_map.borrow_mut().insert(SurfaceKind::Wl(surface), (x, y).into());
    }

    pub fn set_fullscreen (&self, surface: ShellSurface, output: Option<WlOutput>) {
        // NOTE: This is only one part of the solution. We can set the
        // location and configure size here, but the surface should be rendered fullscreen
        // independently from its buffer size
        if let Some(wl_surface) = surface.get_surface() {
            let output_geometry = self.fullscreen_output_geometry(wl_surface, output.as_ref());
            if let Some(geometry) = output_geometry {
                self.window_map.borrow_mut().insert(SurfaceKind::Wl(surface), geometry.loc);
            }
        } else {
            // If there is no underlying surface just ignore the request
            return;
        };
    }

    fn fullscreen_output_geometry(
        &self,
        wl_surface: &wl_surface::WlSurface,
        wl_output: Option<&wl_output::WlOutput>,
    ) -> Option<Rectangle<i32, Logical>> {
        // First test if a specific output has been requested
        // if the requested output is not found ignore the request
        if let Some(wl_output) = wl_output {
            return self.find_by_output(&wl_output).map(|o| o.geometry());
        }
        // There is no output preference, try to find the output
        // where the window is currently active
        let window_location = self.window_map.borrow().find(wl_surface)
            .and_then(|kind| self.window_map.borrow().location(&kind));
        if let Some(location) = window_location {
            let window_output = self.find_by_position(location).map(|o| o.geometry());
            if let Some(result) = window_output {
                return Some(result);
            }
        }
        // Fallback to primary output
        self.with_primary().map(|o| o.geometry())
    }

    pub fn x11_start (&self) {
        if let Err(e) = self.xwayland.start() {
            error!(self.log, "Failed to start XWayland: {}", e);
        }
    }

    pub fn x11_exited (&mut self) -> Result<(), Box<dyn Error>> {
        error!(self.log, "Xwayland crashed");
        Ok(())
    }

    pub fn x11_ready (
        &mut self,
        conn:   UnixStream,
        client: Client,
        handle: &LoopHandle<'static, App>
    ) -> Result<(), Box<dyn Error>> {
        let screen = 0; // Create an X11 connection. XWayland only uses screen 0.
        let stream = DefaultStream::from_unix_stream(conn)?;
        let conn   = RustConnection::connect_to_stream(stream, screen)?;
        let atoms  = Atoms::new(&conn)?.reply()?;
        let screen = &conn.setup().roots[0];
        conn.change_window_attributes( // Actually become the WM by redirecting some operations
            screen.root,
            &ChangeWindowAttributesAux::default().event_mask(EventMask::SUBSTRUCTURE_REDIRECT),
        )?;
        let win = conn.generate_id()?; // Tell XWayland that we are the WM
                                       // by acquiring the WM_S0 selection.
                                       // No X11 clients are accepted before this.
        conn.create_window(
            screen.root_depth, win, screen.root,
            0, 0, 1, 1, 0, // x, y, width, height, border width
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &Default::default(),
        )?;
        conn.set_selection_owner(win, atoms.WM_S0, x11rb::CURRENT_TIME)?;
        // XWayland wants us to do this to function properly...?
        conn.composite_redirect_subwindows(screen.root, Redirect::MANUAL)?;
        conn.flush()?;
        let conn = Rc::new(conn);
        self.x11state = Some(X11State {
            conn: Rc::clone(&conn),
            atoms,
            unpaired: Default::default()
        });
        handle.insert_source(X11Source::new(conn), move |events, _, state| {
            for event in events.into_iter() {
                state.compositor.borrow_mut().x11_handle(event, &client)?;
            }
            Ok(())
        })?;
        Ok(())
    }
    
    fn x11_handle (&self, event: X11Event, client: &Client) -> Result<(), ReplyOrIdError> {
        if self.x11state.is_none() {
            warn!(self.log, "X11: Got event while not ready: {:?}", event);
            return Ok(())
        }
        let X11State { conn, atoms, unpaired } = self.x11state.as_ref().unwrap();
        match event {
            X11Event::ConfigureRequest(r) => {
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
            }
            X11Event::MapRequest(r) => {
                // Just grant the wish
                conn.map_window(r.window)?;
            }
            X11Event::ClientMessage(msg) => {
                if msg.type_ == atoms.WL_SURFACE_ID {
                    // We get a WL_SURFACE_ID message when Xwayland creates a WlSurface for a
                    // window. Both the creation of the surface and this client message happen at
                    // roughly the same time and are sent over different sockets (X11 socket and
                    // wayland socket). Thus, we could receive these two in any order. Hence, it
                    // can happen that we get None below when X11 was faster than Wayland.
                    let location = match conn.get_geometry(msg.window)?.reply() {
                        Ok(geo) => (geo.x as i32, geo.y as i32).into(),
                        Err(err) => {
                            error!(
                                self.log,
                                "Failed to get geometry for {:x}, perhaps the window was already destroyed?",
                                msg.window;
                                "err" => format!("{:?}", err),
                            );
                            (0, 0).into()
                        }
                    };

                    let id = msg.data.as_data32()[0];
                    let surface = client.get_resource::<WlSurface>(id);
                    info!(
                        self.log,
                        "X11 surface {:x?} corresponds to WlSurface {:x} = {:?}", msg.window, id, surface,
                    );
                    match surface {
                        None => {
                            unpaired.borrow_mut().insert(id, (msg.window, location));
                        },
                        Some(surface) => {
                            self.x11_new_window(msg.window, surface, location)
                        },
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn x11_new_window (
        &self,
        window:   X11Window,
        surface:  WlSurface,
        location: Point<i32, Logical>
    ) {
        if give_role(&surface, "x11_surface").is_err() {
            // It makes no sense to post a protocol error here since that would only kill Xwayland
            error!(self.log, "Surface {:x?} already has a role?!", surface);
            return;
        }
        self.window_map.borrow_mut().insert(SurfaceKind::X11(X11Surface { surface }), location);
    }

    pub fn commit (&mut self, surface: &WlSurface) {
        self.commit_x11(surface);
        if !is_sync_subsurface(surface) {
            self.commit_non_sync_subsurface(surface);
        }
        let mut window_map = self.window_map.borrow_mut();
        if let Some(toplevel) = window_map.find(surface) {
            self.commit_initial_configure(surface, &toplevel);
            window_map.refresh_toplevel(&toplevel);
            self.commit_toplevel_resize(&mut *window_map, surface, &toplevel);
        }
        if let Some(popup) = window_map.find_popup(surface) {
            self.commit_popup(surface, &popup);
        }
    }

    /// Called when a WlSurface commits. Removes it from the unpaired list
    pub fn commit_x11 (&mut self, surface: &WlSurface) {
        if surface.as_ref().client().is_some() {
            if let Some(state) = &self.x11state {
                let window = state.unpaired.borrow_mut().remove(&surface.as_ref().id());
                if let Some((window, location)) = window {
                    self.x11_new_window(window, surface.clone(), location);
                }
            }
        }
    }

    /// Update the buffer of all child surfaces
    fn commit_non_sync_subsurface (&self, surface: &WlSurface) {
        with_surface_tree_upward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |_, states, _| {
                states.data_map.insert_if_missing(|| RefCell::new(SurfaceData::default()));
                let mut data = states.data_map.get::<RefCell<SurfaceData>>().unwrap()
                    .borrow_mut();
                data.update_buffer(&mut *states.cached_state.current::<SurfaceAttributes>());
            },
            |_, _, _| true,
        );
    }

    /// send the initial configure if relevant
    fn commit_initial_configure (&self, surface: &WlSurface, toplevel: &SurfaceKind) {
        if let SurfaceKind::Xdg(ref toplevel) = toplevel {
            if !with_states(surface, |states| {
                states.data_map.get::<Mutex<XdgToplevelSurfaceRoleAttributes>>().unwrap()
                    .lock().unwrap().initial_configure_sent
            }).unwrap() {
                toplevel.send_configure();
            }
        }
    }

    fn commit_toplevel_resize (&self, window_map: &mut WindowMap, surface: &WlSurface, toplevel: &SurfaceKind) {
        // Don't move this line into the closure passed `with_states` -
        // method already uses with_states internally and nesting them deadlocks the internal futex
        let geometry = window_map.geometry(&toplevel).expect("Missing top-level geometry.");
        if let Some(location) = with_states(surface, |states| {
            let mut data = states.data_map.get::<RefCell<SurfaceData>>().unwrap().borrow_mut();
            let mut new_location = None;
            // If the window is being resized by top or left, its location must be adjusted
            // accordingly.
            match data.resize_state {
                ResizeState::Resizing(resize_data) |
                ResizeState::WaitingForFinalAck(resize_data, _) |
                ResizeState::WaitingForCommit(resize_data) => {
                    let ResizeData { edges, initial_window_location, initial_window_size } =
                        resize_data;
                    if edges.intersects(ResizeEdge::TOP_LEFT) {
                        let mut location = window_map.location(&toplevel).unwrap();
                        if edges.intersects(ResizeEdge::LEFT) {
                            location.x = initial_window_location.x +
                                (initial_window_size.w - geometry.size.w);
                        }
                        if edges.intersects(ResizeEdge::TOP) {
                            location.y = initial_window_location.y +
                                (initial_window_size.h - geometry.size.h);
                        }
                        new_location = Some(location);
                    }
                }
                ResizeState::NotResizing => (),
            }
            // Finish resizing.
            if let ResizeState::WaitingForCommit(_) = data.resize_state {
                data.resize_state = ResizeState::NotResizing;
            }
            new_location
        }).unwrap() {
            self.window_map.borrow_mut().set_location(&toplevel, location);
        }
    }

    fn commit_popup (&self, surface: &WlSurface, popup: &PopupKind) {
        let PopupKind::Xdg(ref popup) = popup;
        if !with_states(surface, |states| {
            states.data_map.get::<Mutex<XdgPopupSurfaceRoleAttributes>>().unwrap()
                .lock().unwrap()
                .initial_configure_sent
        }).unwrap() {
            // TODO: properly recompute the geometry with the whole of positioner state
            popup.send_configure();
        }
    }

    pub fn retain (&mut self, f: impl FnMut(&Output) -> bool) {
        self.outputs.retain(f);
        self.arrange();
    }

    pub fn height(&self, x: i32) -> Option<i32> {
        // This is a simplification, we only arrange the outputs on the y axis side-by-side
        self.outputs
            .iter()
            .find(|output| {
                let geometry = output.geometry();
                x >= geometry.loc.x && x < (geometry.loc.x + geometry.size.w)
            })
            .map(|output| output.size().h)
    }

    pub fn update(
        &mut self, mode: Option<OutputMode>, scale: Option<f32>, mut f: impl FnMut(&Output) -> bool
    ) {
        let output = self.outputs.iter_mut().find(|o| f(&**o));
        if let Some(output) = output {
            if let Some(mode) = mode {
                output.output.delete_mode(output.current_mode);
                output.output.change_current_state(
                    Some(mode), None, Some(output.output_scale), None);
                output.output.set_preferred(mode);
                output.current_mode = mode;
            }
            if let Some(scale) = scale {
                // Calculate in which direction the scale changed
                let rescale = output.scale() / scale;
                {
                    // We take the current location of our toplevels and move them
                    // to the same location using the new scale
                    let mut window_map = self.window_map.borrow_mut();
                    for surface in output.surfaces.iter() {
                        let toplevel = window_map.find(surface);

                        if let Some(toplevel) = toplevel {
                            let current_location = window_map.location(&toplevel);

                            if let Some(location) = current_location {
                                let output_geometry = output.geometry();

                                if output_geometry.contains(location) {
                                    let mut toplevel_output_location =
                                        (location - output_geometry.loc).to_f64();
                                    toplevel_output_location.x *= rescale as f64;
                                    toplevel_output_location.y *= rescale as f64;
                                    window_map.set_location(
                                        &toplevel,
                                        output_geometry.loc + toplevel_output_location.to_i32_round(),
                                    );
                                }
                            }
                        }
                    }
                }
                let output_scale = scale.round() as i32;
                output.scale = scale;
                if output.output_scale != output_scale {
                    output.output_scale = output_scale;
                    output.output.change_current_state(
                        Some(output.current_mode),
                        None,
                        Some(output_scale),
                        None,
                    );
                }
            }
        }

        self.arrange();
    }

    pub fn update_by_name(
        &mut self, mode: Option<OutputMode>, scale: Option<f32>, name: impl AsRef<str>
    ) {
        self.update(mode, scale, |o| o.name() == name.as_ref())
    }

    pub fn update_scale_by_name(
        &mut self, scale: f32, name: impl AsRef<str>
    ) {
        self.update_by_name(None, Some(scale), name)
    }

    pub fn update_mode_by_name(
        &mut self, mode: OutputMode, name: impl AsRef<str>
    ) {
        self.update_by_name(Some(mode), None, name)
    }

    pub fn clear (&self) {
        self.window_map.borrow_mut().clear()
    }

    /// Send frame events so that client start drawing their next frame
    pub fn send_frames (&self, frames: u32) {
        self.window_map.borrow().send_frames(frames);
    }

    pub fn refresh (&mut self) {
        self.window_map.borrow_mut().refresh();
        // Clean-up dead surfaces
        self.outputs.iter_mut()
            .for_each(|o| o.surfaces.retain(|s| s.as_ref().is_alive()));
        self.window_map.borrow().with_windows_from_bottom_to_top(|kind, location, &bbox| {
            for output in self.outputs.iter_mut() {
                // Check if the bounding box of the toplevel intersects with
                // the output, if not no surface in the tree can intersect with
                // the output.
                if !output.geometry().overlaps(bbox) {
                    if let Some(surface) = kind.get_surface() {
                        with_surface_tree_downward(
                            surface,
                            (),
                            |_, _, _| TraversalAction::DoChildren(()),
                            |wl_surface, _, _| {
                                if output.surfaces.contains(wl_surface) {
                                    output.output.leave(wl_surface);
                                    output.surfaces.retain(|s| s != wl_surface);
                                }
                            },
                            |_, _, _| true,
                        )
                    }
                    continue;
                }
                if let Some(surface) = kind.get_surface() {
                    with_surface_tree_downward(
                        surface,
                        location,
                        |_, states, location| {
                            let mut location = *location;
                            let data = states.data_map.get::<RefCell<SurfaceData>>();
                            if data.is_some() {
                                if states.role == Some("subsurface") {
                                    let current = states.cached_state.current::<SubsurfaceCachedState>();
                                    location += current.location;
                                }
                                TraversalAction::DoChildren(location)
                            } else {
                                // If the parent surface is unmapped, then the child surfaces are hidden as
                                // well, no need to consider them here.
                                TraversalAction::SkipChildren
                            }
                        },
                        |wl_surface, states, &loc| {
                            let data = states.data_map.get::<RefCell<SurfaceData>>();

                            if let Some(size) = data.and_then(|d| d.borrow().size()) {
                                let surface_rectangle = Rectangle { loc, size };

                                if output.geometry().overlaps(surface_rectangle) {
                                    // We found a matching output, check if we already sent enter
                                    if !output.surfaces.contains(wl_surface) {
                                        output.output.enter(wl_surface);
                                        output.surfaces.push(wl_surface.clone());
                                    }
                                } else {
                                    // Surface does not match output, if we sent enter earlier
                                    // we should now send leave
                                    if output.surfaces.contains(wl_surface) {
                                        output.output.leave(wl_surface);
                                        output.surfaces.retain(|s| s != wl_surface);
                                    }
                                }
                            } else {
                                // Maybe the the surface got unmapped, send leave on output
                                if output.surfaces.contains(wl_surface) {
                                    output.output.leave(wl_surface);
                                    output.surfaces.retain(|s| s != wl_surface);
                                }
                            }
                        },
                        |_, _, _| true,
                    )
                }
            }
        });
    }

}

#[derive(Debug)]
pub struct X11State {
    conn:     Rc<RustConnection>,
    atoms:    Atoms,
    unpaired: Rc<RefCell<HashMap<u32, (X11Window, Point<i32, Logical>)>>>
}

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        WM_S0,
        WL_SURFACE_ID,
    }
}

pub struct Output {
    name:         String,
    output:       output::Output,
    global:       Option<Global<wl_output::WlOutput>>,
    surfaces:     Vec<WlSurface>,
    current_mode: OutputMode,
    scale:        f32,
    output_scale: i32,
    location:     Point<i32, Logical>,
    userdata:     UserDataMap,
}

impl Output {
    fn new(
        name: impl AsRef<str>,
        location: Point<i32, Logical>,
        display: &mut Display,
        physical: PhysicalProperties,
        mode: OutputMode,
        log: Logger,
    ) -> Self {
        let (output, global) = output::Output::new(display, name.as_ref().into(), physical, log);
        let scale = std::env::var(format!("ANVIL_SCALE_{}", name.as_ref()))
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0)
            .max(1.0);
        let output_scale = scale.round() as i32;
        output.change_current_state(Some(mode), None, Some(output_scale), Some(location));
        output.set_preferred(mode);
        Self {
            name: name.as_ref().to_owned(),
            global: Some(global),
            output,
            location,
            surfaces: Vec::new(),
            current_mode: mode,
            scale,
            output_scale,
            userdata: Default::default(),
        }
    }
    pub fn userdata(&self) -> &UserDataMap {
        &self.userdata
    }
    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        Rectangle { loc: self.location(), size: self.size() }
    }
    pub fn size(&self) -> Size<i32, Logical> {
        self.current_mode.size.to_f64().to_logical(self.scale as f64).to_i32_round()
    }
    pub fn location(&self) -> Point<i32, Logical> {
        self.location
    }
    pub fn scale(&self) -> f32 {
        self.scale
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn current_mode(&self) -> OutputMode {
        self.current_mode
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.global.take().unwrap().destroy();
    }
}

//impl OutputMap {
    //pub fn new(
        //log:        &Logger,
        //display:    &Rc<RefCell<Display>>,
        //window_map: &Rc<RefCell<WindowMap>>,
    //) -> Self {
        //Self {
            //display:    display.clone(),
            //outputs:    Vec::new(),
            //window_map: window_map.clone(),
            //log:        log.clone(),
        //}
    //}

//}

#[derive(Debug)]
struct Window {
    log: Logger,
    pub location: Point<i32, Logical>,
    /// A bounding box over this window and its children.
    ///
    /// Used for the fast path of the check in `matching`, and as the fall-back for the window
    /// geometry if that's not set explicitly.
    pub bbox:     Rectangle<i32, Logical>,
    pub toplevel: SurfaceKind,
}

impl Window {

    fn new (log: &Logger, location: Point<i32, Logical>, toplevel: SurfaceKind) -> Self {
        Self {
            log: log.clone(),
            location,
            toplevel,
            bbox: Rectangle::default()
        }
    }

    /// Finds the topmost surface under this point if any and returns it together with the location of this
    /// surface.
    fn matching(&self, point: Point<f64, Logical>) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        if !self.bbox.to_f64().contains(point) {
            return None;
        }
        // need to check more carefully
        let found = RefCell::new(None);
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                self.location,
                |wl_surface, states, location| {
                    let mut location = *location;
                    let data = states.data_map.get::<RefCell<SurfaceData>>();
                    if states.role == Some("subsurface") {
                        let current = states.cached_state.current::<SubsurfaceCachedState>();
                        location += current.location;
                    }
                    if data.map(|data| data.borrow().contains_point(
                        &*states.cached_state.current(), point - location.to_f64())
                    ).unwrap_or(false) {
                        *found.borrow_mut() = Some((wl_surface.clone(), location));
                    }
                    TraversalAction::DoChildren(location)
                },
                |_, _, _| {},
                |_, _, _| {
                    // only continue if the point is not found
                    found.borrow().is_none()
                },
            );
        }
        found.into_inner()
    }

    fn self_update(&mut self) {
        let mut bounding_box = Rectangle::from_loc_and_size(self.location, (0, 0));
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                self.location,
                |_, states, &loc| {
                    let mut loc = loc;
                    let data = states.data_map.get::<RefCell<SurfaceData>>();
                    if let Some(size) = data.and_then(|d| d.borrow().size()) {
                        if states.role == Some("subsurface") {
                            let current = states.cached_state.current::<SubsurfaceCachedState>();
                            loc += current.location;
                        }
                        // Update the bounding box.
                        bounding_box = bounding_box.merge(Rectangle::from_loc_and_size(loc, size));
                        TraversalAction::DoChildren(loc)
                    } else {
                        // If the parent surface is unmapped, then the child surfaces are hidden as
                        // well, no need to consider them here.
                        TraversalAction::SkipChildren
                    }
                },
                |_, _, _| {},
                |_, _, _| true,
            );
        }
        self.bbox = bounding_box;
    }

    /// Returns the geometry of this window.
    /// It's the set geometry with the full bounding box as the fallback.
    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        let surface = self.toplevel.get_surface().unwrap();
        let get_geometry = |data: &smithay::wayland::compositor::SurfaceData|{
            data.cached_state.current::<SurfaceCachedState>().geometry};
        with_states(surface, get_geometry).unwrap().unwrap_or(self.bbox)
    }

    /// Sends the frame callback to all the subsurfaces in this
    /// window that requested it
    pub fn send_frame(&self, time: u32) {
        if let Some(wl_surface) = self.toplevel.get_surface() {
            with_surface_tree_downward(
                wl_surface,
                (),
                |_, _, &()| TraversalAction::DoChildren(()),
                |_, states, &()| {
                    // the surface may not have any user_data if it is a subsurface and has not
                    // yet been commited
                    SurfaceData::send_frame(&mut *states.cached_state.current(), time)
                },
                |_, _, &()| true,
            );
        }
    }
}

pub struct WindowMap {
    log:     Logger,
    windows: Vec<Window>,
    popups:  Vec<Popup>,
}

impl WindowMap {

    pub fn init (log: &Logger) -> Self {
        Self { log: log.clone(), windows: vec![], popups: vec![] }
    }

    pub fn insert(&mut self, toplevel: SurfaceKind, location: Point<i32, Logical>) {
        let mut window = Window::new(&self.log, location, toplevel);
        window.self_update();
        self.windows.insert(0, window);
    }

    pub fn windows(&self) -> impl Iterator<Item = SurfaceKind> + '_ {
        self.windows.iter().map(|w| w.toplevel.clone())
    }

    pub fn insert_popup(&mut self, popup: PopupKind) {
        let popup = Popup { popup };
        self.popups.push(popup);
    }

    pub fn get_surface_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        for w in &self.windows {
            if let Some(surface) = w.matching(point) {
                return Some(surface);
            }
        }
        None
    }

    pub fn get_surface_and_bring_to_top(
        &mut self,
        point: Point<f64, Logical>,
    ) -> Option<(wl_surface::WlSurface, Point<i32, Logical>)> {
        let mut found = None;
        for (i, w) in self.windows.iter().enumerate() {
            if let Some(surface) = w.matching(point) {
                found = Some((i, surface));
                break;
            }
        }
        if let Some((i, surface)) = found {
            let winner = self.windows.remove(i);
            // Take activation away from all the windows
            for window in self.windows.iter() {
                window.toplevel.set_activated(false);
            }
            // Give activation to our winner
            winner.toplevel.set_activated(true);
            self.windows.insert(0, winner);
            Some(surface)
        } else {
            None
        }
    }

    pub fn with_windows_from_bottom_to_top(
        &self, mut f: impl FnMut(&SurfaceKind, Point<i32, Logical>, &Rectangle<i32, Logical>)
    ) {
        for w in self.windows.iter().rev() {
            f(&w.toplevel, w.location, &w.bbox)
        }
    }

    pub fn with_child_popups(
        &self, base: &wl_surface::WlSurface, mut f: impl FnMut(&PopupKind)
    ) {
        for w in self.popups.iter().rev().filter(move |w| w.popup.parent().as_ref() == Some(base)) {
            f(&w.popup)
        }
    }

    pub fn refresh(&mut self) {
        self.windows.retain(|w| w.toplevel.alive());
        self.popups.retain(|p| p.popup.alive());
        for w in &mut self.windows {
            w.self_update();
        }
    }

    /// Refreshes the state of the toplevel, if it exists.
    pub fn refresh_toplevel(&mut self, toplevel: &SurfaceKind) {
        if let Some(w) = self.windows.iter_mut().find(|w| &w.toplevel == toplevel) {
            w.self_update();
        }
    }

    pub fn clear(&mut self) {
        self.windows.clear();
    }

    /// Finds the toplevel corresponding to the given `WlSurface`.
    pub fn find(&self, surface: &wl_surface::WlSurface) -> Option<SurfaceKind> {
        self.windows.iter().find_map(|w| {
            if w.toplevel.get_surface().map(|s| s.as_ref().equals(surface.as_ref())).unwrap_or(false) {
                Some(w.toplevel.clone())
            } else {
                None
            }
        })
    }

    /// Finds the popup corresponding to the given `WlSurface`.
    pub fn find_popup(&self, surface: &wl_surface::WlSurface) -> Option<PopupKind> {
        self.popups.iter().find_map(|p| {
            if p.popup.get_surface().map(|s| s.as_ref().equals(surface.as_ref())).unwrap_or(false) {
                Some(p.popup.clone())
            } else {
                None
            }
        })
    }

    /// Returns the location of the toplevel, if it exists.
    pub fn location(&self, toplevel: &SurfaceKind) -> Option<Point<i32, Logical>> {
        self.windows.iter().find(|w| &w.toplevel == toplevel).map(|w| w.location)
    }

    /// Sets the location of the toplevel, if it exists.
    pub fn set_location(&mut self, toplevel: &SurfaceKind, location: Point<i32, Logical>) {
        if let Some(w) = self.windows.iter_mut().find(|w| &w.toplevel == toplevel) {
            w.location = location;
            w.self_update();
        }
    }

    /// Returns the geometry of the toplevel, if it exists.
    pub fn geometry(&self, toplevel: &SurfaceKind) -> Option<Rectangle<i32, Logical>> {
        let window = self.windows.iter().find(|w| &w.toplevel == toplevel);
        window.map(|w| w.geometry())
    }

    pub fn send_frames(&self, time: u32) {
        for window in &self.windows {
            window.send_frame(time);
        }
    }

    pub fn draw_windows<R, E, F, T>(
        &self,
        log:          &Logger,
        renderer:     &mut R,
        frame:        &mut F,
        output_rect:  Rectangle<i32, Logical>,
        output_scale: f32,
    ) -> Result<(), SwapBuffersError>
    where
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: std::error::Error + Into<SwapBuffersError>,
        T: Texture + 'static,
    {
        let mut result = Ok(());
        // redraw the frame, in a simple but inneficient way
        self.with_windows_from_bottom_to_top(|toplevel_surface, mut initial_place, &bounding_box| {
            // skip windows that do not overlap with a given output
            if !output_rect.overlaps(bounding_box) {
                return;
            }
            initial_place.x -= output_rect.loc.x;
            initial_place.y -= output_rect.loc.y;
            if let Some(wl_surface) = toplevel_surface.get_surface() {
                // this surface is a root of a subsurface tree that needs to be drawn
                if let Err(err) =
                    draw_surface_tree(log, renderer, frame, &wl_surface, initial_place, output_scale)
                {
                    result = Err(err);
                }
                // furthermore, draw its popups
                let toplevel_geometry_offset = self
                    .geometry(toplevel_surface)
                    .map(|g| g.loc)
                    .unwrap_or_default();
                self.with_child_popups(&wl_surface, |popup| {
                    let location = popup.location();
                    let draw_location = initial_place + location + toplevel_geometry_offset;
                    if let Some(wl_surface) = popup.get_surface() {
                        if let Err(err) = draw_surface_tree(
                            log, renderer, frame, &wl_surface, draw_location, output_scale
                        ) {
                            result = Err(err);
                        }
                    }
                });
            }
        });
        result
    }

}

pub struct Popup { pub popup: PopupKind, }

#[derive(Clone)]
pub enum PopupKind { Xdg(PopupSurface), }

impl PopupKind {
    pub fn alive(&self) -> bool {
        match *self { PopupKind::Xdg(ref t) => t.alive(), }
    }
    pub fn get_surface(&self) -> Option<&wl_surface::WlSurface> {
        match *self { PopupKind::Xdg(ref t) => t.get_surface(), }
    }
    pub fn parent(&self) -> Option<wl_surface::WlSurface> {
        with_states(match self.get_surface() {
            Some(s) => s,
            None => return None,
        }, |states| states.data_map.get::<Mutex<XdgPopupSurfaceRoleAttributes>>().unwrap()
            .lock().unwrap()
            .parent.clone()
        ).ok().flatten()
    }
    pub fn location(&self) -> Point<i32, Logical> {
        with_states(match self.get_surface() {
            Some(s) => s,
            None => return (0, 0).into(),
        }, |states| states.data_map.get::<Mutex<XdgPopupSurfaceRoleAttributes>>().unwrap()
            .lock().unwrap()
            .current.geometry
        ).unwrap_or_default().loc
    }
}

#[derive(Default)]
pub struct SurfaceData {
    pub buffer: Option<wl_buffer::WlBuffer>,
    pub texture: Option<Box<dyn std::any::Any + 'static>>,
    pub geometry: Option<Rectangle<i32, Logical>>,
    pub resize_state: ResizeState,
    pub buffer_dimensions: Option<Size<i32, Physical>>,
    pub buffer_scale: i32,
}

impl SurfaceData {
    pub fn update_buffer(&mut self, attrs: &mut SurfaceAttributes) {
        match attrs.buffer.take() {
            Some(BufferAssignment::NewBuffer { buffer, .. }) => {
                // new contents
                self.buffer_dimensions = buffer_dimensions(&buffer);
                self.buffer_scale = attrs.buffer_scale;
                if let Some(old_buffer) = std::mem::replace(&mut self.buffer, Some(buffer)) {
                    old_buffer.release();
                }
                self.texture = None;
            }
            Some(BufferAssignment::Removed) => {
                // remove the contents
                self.buffer = None;
                self.buffer_dimensions = None;
                self.texture = None;
            }
            None => {}
        }
    }

    /// Returns the size of the surface.
    pub fn size(&self) -> Option<Size<i32, Logical>> {
        self.buffer_dimensions
            .map(|dims| dims.to_logical(self.buffer_scale))
    }

    /// Checks if the surface's input region contains the point.
    pub fn contains_point(&self, attrs: &SurfaceAttributes, point: Point<f64, Logical>) -> bool {
        let size = match self.size() {
            None => return false, // If the surface has no size, it can't have an input region.
            Some(size) => size,
        };

        let rect = Rectangle {
            loc: (0, 0).into(),
            size,
        }
        .to_f64();

        // The input region is always within the surface itself, so if the surface itself doesn't contain the
        // point we can return false.
        if !rect.contains(point) {
            return false;
        }

        // If there's no input region, we're done.
        if attrs.input_region.is_none() {
            return true;
        }

        attrs
            .input_region
            .as_ref()
            .unwrap()
            .contains(point.to_i32_floor())
    }

    /// Send the frame callback if it had been requested
    pub fn send_frame(attrs: &mut SurfaceAttributes, time: u32) {
        for callback in attrs.frame_callbacks.drain(..) {
            callback.done(time);
        }
    }

}

#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceKind {
    Xdg(ToplevelSurface),
    Wl(ShellSurface),
    X11(X11Surface),
}

impl SurfaceKind {
    pub fn alive(&self) -> bool {
        match *self {
            SurfaceKind::Xdg(ref t) => t.alive(),
            SurfaceKind::Wl(ref t) => t.alive(),
            SurfaceKind::X11(ref t) => t.alive(),
        }
    }
    pub fn get_surface(&self) -> Option<&wl_surface::WlSurface> {
        match *self {
            SurfaceKind::Xdg(ref t) => t.get_surface(),
            SurfaceKind::Wl(ref t) => t.get_surface(),
            SurfaceKind::X11(ref t) => t.get_surface(),
        }
    }
    /// Activate/Deactivate this window
    pub fn set_activated(&self, active: bool) {
        if let SurfaceKind::Xdg(ref t) = self {
            let changed = t.with_pending_state(|state| {
                if active {
                    state.states.set(xdg_toplevel::State::Activated)
                } else {
                    state.states.unset(xdg_toplevel::State::Activated)
                }
            });
            if let Ok(true) = changed {
                t.send_configure();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct X11Surface {
    surface: WlSurface,
}

impl std::cmp::PartialEq for X11Surface {
    fn eq(&self, other: &Self) -> bool {
        self.alive() && other.alive() && self.surface == other.surface
    }
}

impl X11Surface {
    pub fn alive(&self) -> bool {
        self.surface.as_ref().is_alive()
    }

    pub fn get_surface(&self) -> Option<&WlSurface> {
        if self.alive() {
            Some(&self.surface)
        } else {
            None
        }
    }
}

pub struct X11Source {
    connection: Rc<RustConnection>,
    generic: Generic<Fd>,
}

impl X11Source {
    pub fn new(connection: Rc<RustConnection>) -> Self {
        let fd = Fd(connection.stream().as_raw_fd());
        let generic = Generic::new(fd, Interest::READ, CalloopMode::Level);
        Self { connection, generic }
    }
}

impl EventSource for X11Source {
    type Event = Vec<X11Event>;
    type Metadata = ();
    type Ret = Result<(), ReplyOrIdError>;

    fn process_events<C>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: C,
    ) -> IOResult<PostAction>
    where
        C: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        fn inner<C>(conn: &RustConnection, mut callback: C) -> Result<(), ReplyOrIdError>
        where
            C: FnMut(Vec<X11Event>, &mut ()) -> Result<(), ReplyOrIdError>,
        {
            let mut events = Vec::new();
            while let Some(event) = conn.poll_for_event()? {
                events.push(event);
            }
            if !events.is_empty() {
                callback(events, &mut ())?;
            }
            conn.flush()?;
            Ok(())
        }
        let connection = &self.connection;
        self.generic.process_events(readiness, token, |_, _| {
            inner(connection, &mut callback).map_err(|err| IOError::new(ErrorKind::Other, err))?;
            Ok(PostAction::Continue)
        })
    }

    fn register(&mut self, poll: &mut Poll, factory: &mut TokenFactory) -> IOResult<()> {
        self.generic.register(poll, factory)
    }

    fn reregister(&mut self, poll: &mut Poll, factory: &mut TokenFactory) -> IOResult<()> {
        self.generic.reregister(poll, factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> IOResult<()> {
        self.generic.unregister(poll)
    }
}

pub fn draw_surface_tree<R, E, F, T>(
    log:          &Logger,
    renderer:     &mut R,
    frame:        &mut F,
    root:         &WlSurface,
    location:     Point<i32, Logical>,
    output_scale: f32,
) -> Result<(), SwapBuffersError>
where
    R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
    F: Frame<Error = E, TextureId = T>,
    E: std::error::Error + Into<SwapBuffersError>,
    T: Texture + 'static,
{
    let mut result = Ok(());
    with_surface_tree_upward(root, location, |_surface, states, location| {
        let mut location = *location;
        // Pull a new buffer if available
        if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
            let mut data = data.borrow_mut();
            let attributes = states.cached_state.current::<SurfaceAttributes>();
            if data.texture.is_none() {
                if let Some(buffer) = data.buffer.take() {
                    let damage = attributes.damage.iter().map(|dmg| match dmg {
                        Damage::Buffer(rect) => *rect,
                        // TODO also apply transformations
                        Damage::Surface(rect) => rect.to_buffer(attributes.buffer_scale),
                    }).collect::<Vec<_>>();
                    match renderer.import_buffer(&buffer, Some(states), &damage) {
                        Some(Ok(m)) => {
                            let buffer = if let Some(BufferType::Shm) = buffer_type(&buffer) {
                                buffer.release();
                                None
                            } else {
                                Some(buffer)
                            };
                            data.texture = Some(Box::new(BufferTextures { buffer, texture: m }))
                        }
                        Some(Err(err)) => {
                            warn!(log, "Error loading buffer: {:?}", err);
                            buffer.release();
                        }
                        None => {
                            error!(log, "Unknown buffer format for: {:?}", buffer);
                            buffer.release();
                        }
                    }
                }
            }
            // Now, should we be drawn ?
            if data.texture.is_some() {
                // if yes, also process the children
                if states.role == Some("subsurface") {
                    let current = states.cached_state.current::<SubsurfaceCachedState>();
                    location += current.location;
                }
                TraversalAction::DoChildren(location)
            } else {
                // we are not displayed, so our children are neither
                TraversalAction::SkipChildren
            }
        } else {
            // we are not displayed, so our children are neither
            TraversalAction::SkipChildren
        }
    },
    |_surface, states, location| {
        let mut location = *location;
        if let Some(ref data) = states.data_map.get::<RefCell<SurfaceData>>() {
            let mut data = data.borrow_mut();
            let buffer_scale = data.buffer_scale;
            if let Some(texture) = data
                .texture
                .as_mut()
                .and_then(|x| x.downcast_mut::<BufferTextures<T>>())
            {
                // we need to re-extract the subsurface offset, as the previous closure
                // only passes it to our children
                if states.role == Some("subsurface") {
                    let current = states.cached_state.current::<SubsurfaceCachedState>();
                    location += current.location;
                }
                if let Err(err) = frame.render_texture_at(
                    &texture.texture,
                    location.to_f64().to_physical(output_scale as f64).to_i32_round(),
                    buffer_scale,
                    output_scale as f64,
                    Transform::Normal, /* TODO */
                    1.0,
                ) {
                    result = Err(err.into());
                }
            }
        }
    }, |_, _, _| true);

    result
}

struct BufferTextures<T> {
    buffer: Option<wl_buffer::WlBuffer>,
    texture: T,
}

impl<T> Drop for BufferTextures<T> {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.release();
        }
    }
}
