use super::prelude::*;
use super::pointer::*;

use smithay::wayland::shell::xdg::Configure;

delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);

/// Contains the state of the features that are delegated to Smithay's default implementations.
pub struct DelegatedState {
    logger: Logger,
    pub compositor:     CompositorState,
    pub xdg_shell:      XdgShellState,
    pub shm:            ShmState,
    pub output_manager: OutputManagerState,
    pub seat:           SeatState<State>,
    pub data_device:    DataDeviceState,
    display_handle:     DisplayHandle,
    pub clock:          Clock<Monotonic>,
}

impl DelegatedState {
    pub fn new (engine: &impl Engine<State>) -> Result<Self, Box<dyn Error>> {
        let dh = engine.display_handle();
        Ok(Self {
            logger:         engine.logger(),
            compositor:     CompositorState::new::<State, _>(&dh, engine.logger()),
            xdg_shell:      XdgShellState::new::<State, _>(&dh, engine.logger()),
            shm:            ShmState::new::<State, _>(&dh, vec![], engine.logger()),
            output_manager: OutputManagerState::new_with_xdg_output::<State>(&dh),
            seat:           SeatState::new(),
            data_device:    DataDeviceState::new::<State, _>(&dh, engine.logger()),
            display_handle: dh,
            clock:          Clock::new()?
        })
    }

    pub fn seat_add (&mut self, name: impl Into<String>) -> Seat<State> {
        self.seat.new_wl_seat(&self.display_handle, name.into(), self.logger.clone())
    }
}

impl XdgShellHandler for State {

    fn xdg_shell_state (&mut self) -> &mut XdgShellState {
        &mut self.delegated.xdg_shell
    }

    fn new_toplevel (&mut self, surface: ToplevelSurface) {
        let window = Window::new(Kind::Xdg(surface));
        // place the window at a random location on the primary output
        // or if there is not output in a [0;800]x[0;800] square
        use rand::distributions::{Distribution, Uniform};
        let output = self.space.outputs().next().cloned();
        let output_geometry = output.and_then(|o| {
            let geo  = self.space.output_geometry(&o)?;
            let map  = smithay::desktop::layer_map_for_output(&o);
            let zone = map.non_exclusive_zone();
            Some(Rectangle::from_loc_and_size(geo.loc + zone.loc, zone.size))
        }).unwrap_or_else(|| Rectangle::from_loc_and_size((0, 0), (800, 800)));
        let max_x = output_geometry.loc.x + (((output_geometry.size.w as f32) / 3.0) * 2.0) as i32;
        let max_y = output_geometry.loc.y + (((output_geometry.size.h as f32) / 3.0) * 2.0) as i32;
        let x_range = Uniform::new(output_geometry.loc.x, max_x);
        let y_range = Uniform::new(output_geometry.loc.y, max_y);
        let mut rng = rand::thread_rng();
        let x = x_range.sample(&mut rng);
        let y = y_range.sample(&mut rng);
        self.space.map_element(window, (x, y), true);
    }

    fn new_popup (&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|surface| { surface.geometry = positioner.get_geometry(); });
        //if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            //slog::warn!(self.log, "Failed to track popup: {}", err);
        //}
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|surface| {
            let geometry       = positioner.get_geometry();
            surface.geometry   = geometry;
            surface.positioner = positioner;
        });
        surface.send_repositioned(token);
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

    fn grab (&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // TODO popup grabs
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        debug!(self.logger, "ack_configure {surface:?} -> {configure:?}");
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;

    fn seat_state (&mut self) -> &mut SeatState<State> {
        &mut self.delegated.seat
    }

    fn cursor_image (
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, _seat: &smithay::input::Seat<Self>, _focused: Option<&WlSurface>) {
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &smithay::wayland::data_device::DataDeviceState {
        &self.delegated.data_device
    }
}

impl ClientDndGrabHandler for State {}

impl ServerDndGrabHandler for State {}

impl CompositorHandler for State {
    fn compositor_state (&mut self) -> &mut CompositorState {
        &mut self.delegated.compositor
    }

    /// Commit each surface - what does it do?
    fn commit (&mut self, surface: &WlSurface) {
        // What does this do?
        on_commit_buffer_handler(surface);
    }
}

/// Should be called on `WlSurface::commit`
pub fn grab_handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    let window = space.elements().find(|w| w.toplevel().wl_surface() == surface).cloned()?;
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
        &self.delegated.shm
    }
}
