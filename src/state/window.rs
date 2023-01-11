use super::prelude::*;

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
