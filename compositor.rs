use crate::App;
use crate::prelude::*;
use crate::grab::{MoveSurfaceGrab, ResizeSurfaceGrab, ResizeState, ResizeData};
use crate::surface::{SurfaceData, SurfaceKind};
use crate::window::WindowMap;
use crate::output::OutputMap;
use crate::popup::PopupKind;

pub struct Compositor {
    pub window_map: Rc<RefCell<WindowMap>>,
    pub output_map: Rc<RefCell<OutputMap>>,
}

impl Compositor {

    pub fn init (log: &Logger, display: &Rc<RefCell<Display>>) -> Rc<Self> {
        let window_map = Rc::new(RefCell::new(WindowMap::default()));
        let output_map = Rc::new(RefCell::new(OutputMap::new(
            display.clone(), window_map.clone(), log.clone())));
        compositor_init(
            &mut *display.borrow_mut(),
            |surface, mut data|data.get::<App>().unwrap()
                .compositor.window_map.as_ref().borrow_mut().commit(&surface),
            log.clone()
        );
        let compositor = Rc::new(Self { window_map, output_map });
        compositor.clone().init_xdg_shell(&log, &display);
        compositor.clone().init_wl_shell(&log, &display);
        compositor
    }

    pub fn init_xdg_shell (
        self: Rc<Self>, log: &Logger, display: &Rc<RefCell<Display>>,
    ) -> Arc<Mutex<XdgShellState>> {
        xdg_shell_init(
            &mut *display.borrow_mut(),
            move |shell_event, _dispatch_data| match shell_event {
                XdgRequest::NewToplevel { surface }
                    => self.xdg_new_toplevel(surface),
                XdgRequest::NewPopup { surface }
                    => self.xdg_new_popup(surface),
                XdgRequest::Move { surface, seat, serial, }
                    => self.xdg_move(&surface, seat, serial),
                XdgRequest::Resize { surface, seat, serial, edges }
                    => self.xdg_resize(&surface, seat, serial, edges),
                XdgRequest::AckConfigure { surface, configure: Configure::Toplevel(configure), .. }
                    => self.xdg_ack_configure(&surface, configure),
                XdgRequest::Fullscreen { surface, output, .. }
                    => self.xdg_fullscreen(&surface, output),
                XdgRequest::UnFullscreen { surface }
                    => self.xdg_unfullscreen(&surface),
                XdgRequest::Maximize { surface }
                    => self.xdg_maximize(&surface),
                XdgRequest::UnMaximize { surface }
                    => self.xdg_unmaximize(&surface),
                _ => (),
            },
            log.clone()
        ).0
    }

    pub fn xdg_new_toplevel (&self, surface: ToplevelSurface) {
        // place the window at a random location on the primary output
        // or if there is not output in a [0;800]x[0;800] square
        use rand::distributions::{Distribution, Uniform};
        let output_geometry = self.output_map
            .borrow().with_primary().map(|o| o.geometry())
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

    pub fn xdg_move (&self, surface: &ToplevelSurface, seat: WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch move.
        let pointer = seat.get_pointer().unwrap();
        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }
        let start_data = pointer.grab_start_data().unwrap();
        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .as_ref()
                .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            return;
        }
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

                    // NOTE: In real compositor mouse location should be mapped to a new window size
                    // For example, you could:
                    // 1) transform mouse pointer position from compositor space to window space (location relative)
                    // 2) divide the x coordinate by width of the window to get the percentage
                    //   - 0.0 would be on the far left of the window
                    //   - 0.5 would be in middle of the window
                    //   - 1.0 would be on the far right of the window
                    // 3) multiply the percentage by new window width
                    // 4) by doing that, drag will look a lot more natural
                    //
                    // but for anvil needs setting location to pointer location is fine
                    let pos = pointer.current_location();
                    initial_window_location = (pos.x as i32, pos.y as i32).into();
                }
            }
        }

        let grab = MoveSurfaceGrab {
            start_data,
            window_map: self.window_map.clone(),
            toplevel,
            initial_window_location,
        };

        pointer.set_grab(grab, serial);
    }

    pub fn xdg_resize (
        &self, surface: &ToplevelSurface, seat: WlSeat, serial: Serial, edges: ResizeEdge
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch resize.
        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .as_ref()
                .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            return;
        }
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
                .and_then(|position| {
                    self.output_map.borrow().find_by_position(position).map(|o| o.geometry())
                })
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

    pub fn init_wl_shell (
        self: Rc<Self>, log: &Logger, display: &Rc<RefCell<Display>>,
    ) -> Arc<Mutex<WlShellState>> {
        wl_shell_init(
            &mut *display.borrow_mut(),
            move |req: ShellRequest, _dispatch_data| {
                match req {
                    ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Toplevel, }
                        => self.set_toplevel(surface),
                    ShellRequest::SetKind { surface, kind: ShellSurfaceKind::Fullscreen { output, .. } }
                        => self.set_fullscreen(surface, output),
                    ShellRequest::Move { surface, seat, serial }
                        => self.shell_move(surface, seat, serial),
                    ShellRequest::Resize { surface, seat, serial, edges, }
                        => self.shell_resize(surface, seat, serial, edges),
                        _ => (),
                }
            }, log.clone()
        ).0
    }

    /// place the window at a random location on the primary output
    /// or if there is not output in a [0;800]x[0;800] square
    pub fn set_toplevel (&self, surface: ShellSurface) {
        use rand::distributions::{Distribution, Uniform};
        let output_geometry = self.output_map.borrow().with_primary().map(|o| o.geometry())
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

    pub fn shell_move (&self, surface: ShellSurface, seat: WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let pointer = seat.get_pointer().unwrap();
        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) { return; }
        let start_data = pointer.grab_start_data();
        // If the focus was for a different surface, ignore the request.
        if let Some(start_data) = start_data && start_data.focus.as_ref().unwrap().0.as_ref()
            .same_client_as(surface.get_surface().unwrap().as_ref())
        {
            let toplevel = SurfaceKind::Wl(surface);
            let initial_window_location = self.window_map.borrow().location(&toplevel).unwrap();
            pointer.set_grab(MoveSurfaceGrab {
                start_data, window_map: self.window_map.clone(), toplevel, initial_window_location,
            }, serial);
        }
    }
    
    pub fn shell_resize (
        &self, surface: ShellSurface, seat: WlSeat, serial: Serial, edges: Resize,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        // TODO: touch resize.
        let pointer = seat.get_pointer().unwrap();
        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) { return; }
        let start_data = pointer.grab_start_data();
        // If the focus was for a different surface, ignore the request.
        if let Some(start_data) = start_data && start_data.focus.as_ref().unwrap().0.as_ref()
            .same_client_as(surface.get_surface().unwrap().as_ref())
        {
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

    fn fullscreen_output_geometry(
        &self,
        wl_surface: &wl_surface::WlSurface,
        wl_output: Option<&wl_output::WlOutput>,
    ) -> Option<Rectangle<i32, Logical>> {
        // First test if a specific output has been requested
        // if the requested output is not found ignore the request
        if let Some(wl_output) = wl_output {
            return self.output_map.borrow().find_by_output(&wl_output).map(|o| o.geometry());
        }
        // There is no output preference, try to find the output
        // where the window is currently active
        let window_location = self.window_map.borrow()
            .find(wl_surface)
            .and_then(|kind| self.window_map.borrow().location(&kind));
        if let Some(location) = window_location {
            let window_output = self.output_map.borrow()
                .find_by_position(location).map(|o| o.geometry());
            if let Some(result) = window_output {
                return Some(result);
            }
        }
        // Fallback to primary output
        self.output_map.borrow().with_primary().map(|o| o.geometry())
    }

}
