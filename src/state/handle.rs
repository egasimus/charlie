use super::prelude::*;
use super::pointer::*;

delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);

pub struct DelegatedState {
    logger: Logger,
    pub compositor_state:     CompositorState,
    pub xdg_shell_state:      XdgShellState,
    pub shm_state:            ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state:           SeatState<State>,
    pub data_device_state:    DataDeviceState,
}

impl DelegatedState {
    pub fn new (engine: &impl Engine) -> Result<Self, Box<dyn Error>> {
        let dh = engine.display_handle();
        Ok(Self {
            logger: engine.logger(),
            compositor_state:     CompositorState::new::<State, _>(&dh, engine.logger()),
            xdg_shell_state:      XdgShellState::new::<State, _>(&dh, engine.logger()),
            shm_state:            ShmState::new::<State, _>(&dh, vec![], engine.logger()),
            output_manager_state: OutputManagerState::new_with_xdg_output::<State>(&dh),
            seat_state:           SeatState::new(),
            data_device_state:    DataDeviceState::new::<State, _>(&dh, engine.logger()),
        })
    }
}

impl XdgShellHandler for State {

    fn xdg_shell_state (&mut self) -> &mut XdgShellState {
        &mut self.delegated.xdg_shell_state
    }

    fn new_toplevel (&mut self, surface: ToplevelSurface) {
        let window = Window::new(Kind::Xdg(surface));
        self.space.map_element(window, (0, 0), false);
    }

    fn new_popup (&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // TODO: Popup handling using PopupManager
    }

    fn move_request (&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();
        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self.space.elements().find(|w| w.toplevel().wl_surface() == wl_surface).unwrap().clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request (
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: XdgToplevelResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();
        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self.space.elements()
                .find(|w| w.toplevel().wl_surface() == wl_surface).unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;
            surface.with_pending_state(|state| { state.states.set(XdgToplevelState::Resizing); });
            surface.send_configure();
            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::from_loc_and_size(initial_window_location, initial_window_size),
            );
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // TODO popup grabs
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.delegated.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
    fn focus_changed(&mut self, _seat: &smithay::input::Seat<Self>, _focused: Option<&WlSurface>) {}
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &smithay::wayland::data_device::DataDeviceState {
        &self.delegated.data_device_state
    }
}

impl ClientDndGrabHandler for State {}

impl ServerDndGrabHandler for State {}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.delegated.compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.space.elements().find(|w| w.toplevel().wl_surface() == &root) {
                window.on_commit();
            }
        };
        xdg_handle_commit(&self.space, surface);
        grab_handle_commit(&mut self.space, surface);
    }
}

/// Should be called on `WlSurface::commit`
pub fn xdg_handle_commit(space: &Space<Window>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|w| w.toplevel().wl_surface() == surface)
        .cloned()?;

    if let Kind::Xdg(_) = window.toplevel() {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.configure();
        }
    }

    Some(())
}

/// Should be called on `WlSurface::commit`
pub fn grab_handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|w| w.toplevel().wl_surface() == surface)
        .cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    let new_loc: Point<Option<i32>, Logical> = ResizeSurfaceState::with(surface, |state| {
        state
            .commit()
            .and_then(|(edges, initial_rect)| {
                // If the window is being resized by top or left, its location must be adjusted
                // accordingly.
                edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    let new_x = edges
                        .intersects(ResizeEdge::LEFT)
                        .then_some(initial_rect.loc.x + (initial_rect.size.w - geometry.size.w));

                    let new_y = edges
                        .intersects(ResizeEdge::TOP)
                        .then_some(initial_rect.loc.y + (initial_rect.size.h - geometry.size.h));

                    (new_x, new_y).into()
                })
            })
            .unwrap_or_default()
    });

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        // If TOP or LEFT side of the window got resized, we have to move it
        space.map_element(window, window_loc, false);
    }

    Some(())
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.delegated.shm_state
    }
}
