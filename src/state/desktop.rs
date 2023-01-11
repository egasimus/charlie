use super::prelude::*;

pub struct Desktop {
    logger:  Logger,
    /// A collection of windows that are mapped across the screens
    windows: Vec<WindowState>,
    /// A collection of views into the workspace, bound to engine outputs
    pub screens: Vec<ScreenState>,
}

impl Desktop {
    pub fn new (logger: Logger) -> Self {
        Self {
            logger,
            windows: vec![],
            screens: vec![],
        }
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

    pub fn render (&self, frame: &mut Gles2Frame, size: Size<i32, Physical>) -> Result<(), Box<dyn Error>> {
        for window in self.windows.iter() {
            window.render(&self.logger, frame, size)?;
        }
        Ok(())
    }

    pub fn tick (&self, output: &Output, time: Time<Monotonic>) {
        for window in self.windows.iter() {
            window.window.send_frame(
                output,
                Duration::from(time),
                Some(Duration::from_secs(1)),
                smithay::desktop::utils::surface_primary_scanout_output
            );
        }
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
    pub fn render (&self, logger: &Logger, frame: &mut Gles2Frame, size: Size<i32, Physical>)
        -> Result<(), Box<dyn Error>>
    {
        let (src, dest, damage): (Rectangle<f64, Buffer>, Rectangle<i32, Physical>, Rectangle<i32, Physical>) = (
            Rectangle::from_loc_and_size((0.0, 0.0), (size.w as f64, size.h as f64)),
            Rectangle::from_loc_and_size((20, 10), size),
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
                        texture, src, dest, &[damage], Transform::Flipped180, 1.0f32
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
