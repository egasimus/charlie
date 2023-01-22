use super::prelude::*;

pub struct Desktop {
    logger: Logger,
    clock:  Clock<Monotonic>,
    /// A collection of windows that are mapped across the screens
    windows: Vec<WindowState>,
    /// A collection of views into the workspace, bound to engine outputs
    pub screens: Vec<ScreenState>,
    compositor: CompositorState,
    xdg_shell: XdgShellState,
}

impl Desktop {

    pub fn new <T> (logger: &Logger, handle: &DisplayHandle) -> Result<Self, Box<dyn Error>>
    where
        T: GlobalDispatch<WlCompositor,    ()> +
           GlobalDispatch<WlSubcompositor, ()> +
           GlobalDispatch<XdgWmBase,       ()>
    {
        Ok(Self {
            logger:     logger.clone(),
            clock:      Clock::new()?,
            compositor: CompositorState::new::<T, _>(&handle, logger.clone()),
            xdg_shell:  XdgShellState::new::<T, _>(&handle, logger.clone()),
            windows:    vec![],
            screens:    vec![],
        })
    }

    /// Add a viewport into the workspace.
    pub fn screen_add (&mut self, screen: ScreenState) -> usize {
        self.screens.push(screen);
        self.screens.len() - 1
    }

    /// Add a window to the workspace.
    pub fn window_add (&mut self, window: Window) -> usize {
        self.windows.push(WindowState::new(window));
        self.windows.len() - 1
    }

    /// Find a window by its top level surface.
    pub fn window_find (&self, surface: &WlSurface) -> Option<&Window> {
        self.windows.iter()
            .find(|w| w.window.toplevel().wl_surface() == surface)
            .map(|w|&w.window)
    }

    pub fn import (&self, renderer: &mut Gles2Renderer) -> Result<(), Box<dyn Error>> {
        for window in self.windows.iter() {
            window.import(&self.logger, renderer)?;
        }
        Ok(())
    }

    pub fn render (&self, frame: &mut Gles2Frame, screen_id: usize, size: Size<i32, Physical>) -> Result<(), Box<dyn Error>> {
        for window in self.windows.iter() {
            window.render(&self.logger, frame, self.screens[screen_id].center, size)?;
        }
        Ok(())
    }

    pub fn send_frames (&self, output: &Output) {
        for window in self.windows.iter() {
            window.window.send_frame(
                output,
                Duration::from(self.clock.now()),
                Some(Duration::from_secs(1)),
                smithay::desktop::utils::surface_primary_scanout_output
            );
        }
    }

}

delegate_compositor!(Desktop);

impl CompositorHandler for Desktop {

    fn compositor_state (&mut self) -> &mut CompositorState {
        &mut self.compositor
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
        if let Some(window) = self.window_find(&surface) {
            window.on_commit();
        } else {
            warn!(self.logger, "could not find window for root toplevel surface {surface:?}");
        };
    }

}

delegate_xdg_shell!(Desktop);

impl XdgShellHandler for Desktop {

    fn xdg_shell_state (&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel (&mut self, surface: ToplevelSurface) {
        debug!(self.logger, "New toplevel surface: {surface:?}");
        surface.send_configure();
        self.window_add(Window::new(Kind::Xdg(surface)));
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

    fn ack_configure(&mut self, surface: WlSurface, configure: smithay::wayland::shell::xdg::Configure) {
        debug!(self.logger, "ack_configure {surface:?} -> {configure:?}");
    }
}

pub struct ScreenState {
    pub center: Point<f64, Logical>,
    size:   Size<f64, Logical>
}

impl ScreenState {
    pub fn new (
        center: impl Into<Point<f64, Logical>>,
        size:   impl Into<Size<f64, Logical>>
    ) -> Self {
        Self { center: center.into(), size: size.into() }
    }
    #[inline]
    pub fn center (&self) -> &Point<f64, Logical> {
        &self.center
    }
}

pub struct WindowState {
    pub window: Window,
    center: Point<f64, Logical>,
    size:   Size<f64, Logical>
}

impl WindowState {

    pub fn new (window: Window) -> Self {
        Self { window, center: (0.0, 0.0).into(), size: (0.0, 0.0).into() }
    }

    /// Import the window's surface into the renderer as a texture
    pub fn import (&self, logger: &Logger, renderer: &mut Gles2Renderer)
        -> Result<(), Box<dyn Error>>
    {
        let surface = match self.window.toplevel() {
            Kind::Xdg(xdgsurface) => xdgsurface.wl_surface(),
            Kind::X11(x11surface) => &x11surface.surface
        };
        with_states(surface, |surface_data| {
            if let Some(data) = surface_data.data_map.get::<RendererSurfaceStateUserData>() {
                let data = &mut *data.borrow_mut();
                let texture_id = (
                    TypeId::of::<<Gles2Renderer as Renderer>::TextureId>(),
                    renderer.id().clone()
                );
                if let Entry::Vacant(entry) = data.textures.entry(texture_id) {
                    if let Some(buffer) = data.buffer.as_ref() {
                        match renderer.import_buffer(
                            buffer, Some(surface_data), &match buffer_dimensions(buffer) {
                                Some(size) => vec![Rectangle::from_loc_and_size((0, 0), size)],
                                None       => vec![]
                            }
                        ) {
                            Some(Ok(m)) => {
                                warn!(logger, "Loading {m:?}");
                                entry.insert(Box::new(m));
                            }
                            Some(Err(err)) => {
                                warn!(logger, "Error loading buffer: {}", err);
                                return Err(err);
                            }
                            None => {
                                error!(logger, "Unknown buffer format for: {:?}", buffer);
                            }
                        }
                    } else {
                        warn!(logger, "No buffer in {surface_data:?}")
                    }
                }
            } else {
                warn!(logger, "No RendererSurfaceState for {surface:?}")
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Render the window's imported texture into the current frame
    pub fn render (
        &self,
        logger: &Logger,
        frame:  &mut Gles2Frame,
        offset: Point<f64, Logical>,
        size:   Size<i32, Physical>
    )
        -> Result<(), Box<dyn Error>>
    {
        let (src, dest, damage): (Rectangle<f64, Buffer>, Rectangle<i32, Physical>, Rectangle<i32, Physical>) = (
            Rectangle::from_loc_and_size((0.0, 0.0), (size.w as f64, size.h as f64)),
            Rectangle::from_loc_and_size((
                self.center.x as i32 + offset.x as i32,
                self.center.y as i32 + offset.y as i32
            ), size),
            Rectangle::from_loc_and_size((0, 0), size)
        );
        let surface = match self.window.toplevel() {
            Kind::Xdg(xdgsurface) => xdgsurface.wl_surface(),
            Kind::X11(x11surface) => &x11surface.surface
        };
        with_states(surface, |surface_data| {
            if let Some(data) = surface_data.data_map.get::<RendererSurfaceStateUserData>() {
                if let Some(texture) = data.borrow().texture::<Gles2Renderer>(frame.id()) {
                    frame.render_texture_from_to(
                        texture, src, dest, &[damage], Transform::Normal, 1.0f32
                    ).unwrap();
                } else {
                    warn!(logger, "No texture in this renderer for {data:?}");
                    //frame.render_texture_from_to(
                        //&self.pointer.texture, src, dest, &[damage], Transform::Flipped180, 1.0f32
                    //).unwrap();
                }
            } else {
                warn!(logger, "No RendererSurfaceState for {surface:?}")
            }
        });
        Ok(())
    }

}
