use super::prelude::*;
use super::pointer::*;

use smithay::wayland::shell::xdg::Configure;

delegate_seat!(App);
delegate_data_device!(App);
delegate_output!(App);
delegate_compositor!(App);
delegate_shm!(App);
delegate_xdg_shell!(App);

/// Contains the state of the features that are delegated to Smithay's default implementations.
pub struct DelegatedState {
    logger: Logger,
    pub compositor:     CompositorState,
    pub xdg_shell:      XdgShellState,
    pub shm:            ShmState,
    pub output_manager: OutputManagerState,
    pub seat:           SeatState<App>,
    pub data_device:    DataDeviceState,
    display_handle:     DisplayHandle,
    pub clock:          Clock<Monotonic>,
}

impl DelegatedState {
    pub fn new (engine: &impl Engine) -> Result<Self, Box<dyn Error>> {
        let dh = engine.display_handle();
        Ok(Self {
            logger:         engine.logger(),
            compositor:     CompositorState::new::<App, _>(&dh, engine.logger()),
            xdg_shell:      XdgShellState::new::<App, _>(&dh, engine.logger()),
            shm:            ShmState::new::<App, _>(&dh, vec![], engine.logger()),
            output_manager: OutputManagerState::new_with_xdg_output::<App>(&dh),
            seat:           SeatState::new(),
            data_device:    DataDeviceState::new::<App, _>(&dh, engine.logger()),
            display_handle: dh,
            clock:          Clock::new()?
        })
    }

    pub fn seat_add (&mut self, name: impl Into<String>) -> Seat<App> {
        self.seat.new_wl_seat(&self.display_handle, name.into(), self.logger.clone())
    }
}

impl XdgShellHandler for App {

    fn xdg_shell_state (&mut self) -> &mut XdgShellState {
        &mut self.delegated.xdg_shell
    }

    fn new_toplevel (&mut self, surface: ToplevelSurface) {
        debug!(self.logger, "New toplevel surface: {surface:?}");
        surface.send_configure();
        let window = Window::new(Kind::Xdg(surface));
        self.window_add(window);
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
        //let seat = Seat::from_resource(&seat).unwrap();
        //let wl_surface = surface.wl_surface();
        //if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            //let pointer = seat.get_pointer().unwrap();
            //let window = self.window_find(wl_surface).unwrap();
            //let initial_window_location = Default::default();//self.space.element_location(&window).unwrap();
            //let grab = MoveSurfaceGrab { start_data, window: window.clone(), initial_window_location, };
            //pointer.set_grab(self, grab, serial, Focus::Clear);
        //}
    }

    fn resize_request (
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: XdgToplevelResizeEdge,
    ) {
        //let seat = Seat::from_resource(&seat).unwrap();
        //let wl_surface = surface.wl_surface();
        //if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            //let pointer = seat.get_pointer().unwrap();
            //let window = self.window_find(wl_surface).unwrap();
            ////let initial_window_location = Default::default();//self.space.element_location(&window).unwrap();
            ////let initial_window_size = (*window).geometry().size;
            //surface.with_pending_state(|state| { state.states.set(XdgToplevelState::Resizing); });
            //surface.send_configure();
            ////let grab = ResizeSurfaceGrab::start(
                ////start_data,
                ////window.clone(),
                ////edges.into(),
                ////Rectangle::from_loc_and_size(initial_window_location, initial_window_size),
            ////);
            ////pointer.set_grab(self, grab, serial, Focus::Clear);
        //}
    }

    fn grab (&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // TODO popup grabs
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        debug!(self.logger, "ack_configure {surface:?} -> {configure:?}");
    }
}

impl SeatHandler for App {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;

    fn seat_state (&mut self) -> &mut SeatState<App> {
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

impl DataDeviceHandler for App {
    fn data_device_state(&self) -> &smithay::wayland::data_device::DataDeviceState {
        &self.delegated.data_device
    }
}

impl ClientDndGrabHandler for App {}

impl ServerDndGrabHandler for App {}

impl CompositorHandler for App {
    fn compositor_state (&mut self) -> &mut CompositorState {
        &mut self.delegated.compositor
    }

    /// Commit each surface, binding a state data buffer to it.
    /// AFAIK This buffer contains the texture which is imported before each render.
    fn commit (&mut self, surface: &WlSurface) {
        //debug!(self.logger, "Commit {surface:?}");
        use smithay::backend::renderer::utils::{
            RendererSurfaceState         as State,
            RendererSurfaceStateUserData as StateData
        };
        let mut surface = surface.clone();
        loop {
            let mut is_new = false;
            warn!(self.logger, "Init surface: {surface:?}");
            with_states(&surface, |surface_data| {
                is_new = surface_data.data_map.insert_if_missing(||RefCell::new(State::default()));
                let mut data = surface_data.data_map.get::<StateData>().unwrap().borrow_mut();
                data.update_buffer(surface_data);
            });
            if is_new {
                add_destruction_hook(&surface, |data| {
                    let data = data.data_map.get::<StateData>();
                    if let Some(buffer) = data.and_then(|s|s.borrow_mut().buffer.take()) {
                        buffer.release()
                    }
                })
            }
            match get_parent(&surface) {
                Some(parent) => surface = parent,
                None => break
            }
        }
        if let Some(window) = self.desktop.borrow().window_find(&surface) {
            window.on_commit();
        } else {
            warn!(self.logger, "could not find window for root toplevel surface {surface:?}");
        };
    }
}

//// Should be called on `WlSurface::commit`
//pub fn grab_handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    //let window = space.elements().find(|w| w.toplevel().wl_surface() == surface).cloned()?;
    //let mut window_loc = space.element_location(&window)?;
    //let geometry = window.geometry();
    //let new_loc: Point<Option<i32>, Logical> = ResizeSurfaceState::with(surface, |state| {
        //state
            //.commit()
            //.and_then(|(edges, initial_rect)| {
                //// If the window is being resized by top or left, its location must be adjusted
                //// accordingly.
                //edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    //let new_x = edges
                        //.intersects(ResizeEdge::LEFT)
                        //.then_some(initial_rect.loc.x + (initial_rect.size.w - geometry.size.w));

                    //let new_y = edges
                        //.intersects(ResizeEdge::TOP)
                        //.then_some(initial_rect.loc.y + (initial_rect.size.h - geometry.size.h));

                    //(new_x, new_y).into()
                //})
            //})
            //.unwrap_or_default()
    //});
    //if let Some(new_x) = new_loc.x {
        //window_loc.x = new_x;
    //}
    //if let Some(new_y) = new_loc.y {
        //window_loc.y = new_y;
    //}
    //if new_loc.x.is_some() || new_loc.y.is_some() {
        //// If TOP or LEFT side of the window got resized, we have to move it
        //space.map_element(window, window_loc, false);
    //}
    //Some(())
//}

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for App {
    fn shm_state(&self) -> &ShmState {
        &self.delegated.shm
    }
}
