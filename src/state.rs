mod prelude;
mod wayland;
mod handle;
mod pointer;
mod xwayland;

use self::prelude::*;
use self::wayland::WaylandListener;
use self::handle::DelegatedState;
use self::pointer::Pointer;
use self::xwayland::XWaylandState;

pub struct State {
    logger:  Logger,
    /// A wayland socket listener
    wayland: WaylandListener,
    /// A collection of views into the workspace, bound to engine outputs
    screens: Vec<ScreenState>,
    /// A collection of windows that are mapped across the screens
    windows: Vec<WindowState>,
    /// State of the mouse pointer
    pub pointer:   Pointer,
    /// State of the X11 integration.
    pub xwayland:  XWaylandState,
    /// States of smithay-provided implementations of compositor features
    pub delegated: DelegatedState,
    /// Commands to run after successful initialization
    startup: Vec<(String, Vec<String>)>,
}

impl State {

    pub fn new (engine: &mut impl Engine<Self>) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:    engine.logger(),
            screens:   vec![],
            windows:   vec![],
            pointer:   Pointer::new(engine)?,
            wayland:   WaylandListener::new(engine)?,
            xwayland:  XWaylandState::new(engine)?,
            delegated: DelegatedState::new(engine)?,
            startup:   vec![],
        })
    }

    pub fn ready (&mut self) -> Result<(), Box<dyn Error>> {
        debug!(self.logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
        debug!(self.logger, "WAYLAND_DISPLAY={:?}", ::std::env::var("WAYLAND_DISPLAY"));
        debug!(self.logger, "{:?}", self.startup);
        for (cmd, args) in self.startup.iter() {
            debug!(self.logger, "Spawning {cmd} {args:?}");
            std::process::Command::new(cmd).args(args).spawn()?;
        }
        Ok(())
    }

    /// Add a command to run on startup.
    /// TODO: Integrate a `systemd --user` session
    pub fn startup_add (&mut self, command: &str, args: &[&str]) -> usize {
        self.startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self.startup.len() - 1
    }

    /// Add a viewport into the workspace.
    pub fn screen_add (&mut self, screen: ScreenState) -> usize {
        self.screens.push(screen);
        self.screens.len() - 1
    }

    /// Add an control seat over the workspace.
    pub fn seat_add (&mut self, name: impl Into<String>) -> Result<Seat<Self>, Box<dyn Error>> {
        use smithay::input::keyboard::XkbConfig;
        use smithay::wayland::input_method::InputMethodSeat;
        let mut seat = self.delegated.seat_add(name);
        seat.add_pointer();
        seat.add_keyboard(XkbConfig::default(), 200, 25)?;
        seat.add_input_method(XkbConfig::default(), 200, 25);
        Ok(seat)
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

}

type ScreenId = usize;

impl<'a> Widget for State {

    type RenderData = ScreenId;

    fn render <'r> (
        &'r self, context: RenderContext<'r, Self::RenderData>
    ) -> Result<(), Box<dyn Error>> {

        use smithay::backend::renderer::{
            buffer_dimensions,
            ImportAll,
            utils::RendererSurfaceStateUserData
        };

        let RenderContext { renderer, output, data: screen } = context;

        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );

        let (src, dest): (Rectangle<f64, Buffer>, Rectangle<i32, Physical>) = (
            Rectangle::from_loc_and_size((0.0, 0.0), (size.w as f64, size.h as f64)),
            Rectangle::from_loc_and_size((0, 0), size)
        );

        for window in self.windows.iter() {

            let surface = match window.window.toplevel() {
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
                                    warn!(self.logger, "Loading {m:?}");
                                    entry.insert(Box::new(m));
                                }
                                Some(Err(err)) => {
                                    warn!(self.logger, "Error loading buffer: {}", err);
                                    return Err(err);
                                }
                                None => {
                                    error!(self.logger, "Unknown buffer format for: {:?}", buffer);
                                }
                            }

                        } else {
                            warn!(self.logger, "No buffer in {surface_data:?}")
                        }

                    }

                } else {
                    warn!(self.logger, "No RendererSurfaceState for {surface:?}")
                }

                Ok(())
            })?;
        };

        let mut frame = renderer.render(size, transform)?;

        frame.clear([0.2,0.3,0.4,1.0], &[dest])?;

        for window in self.windows.iter() {

            let surface = match window.window.toplevel() {
                Kind::Xdg(xdgsurface) => xdgsurface.wl_surface(),
                Kind::X11(x11surface) => &x11surface.surface
            };

            with_states(surface, |surface_data| {

                if let Some(data) = surface_data.data_map.get::<RendererSurfaceStateUserData>() {

                    if let Some(texture) = data.borrow().texture::<Gles2Renderer>(frame.id()) {

                        frame.render_texture_from_to(
                            texture, src, dest, &[dest], Transform::Normal, 1.0f32
                        ).unwrap();

                    } else {
                        warn!(self.logger, "No texture in this renderer for {data:?}")
                    }

                } else {
                    warn!(self.logger, "No RendererSurfaceState for {surface:?}")
                }

            });
        }

        self.pointer.render(&mut frame, size, &self.screens[screen])?;

        frame.finish()?;

        for window in self.windows.iter() {
            window.window.send_frame(
                output,
                Duration::from(self.delegated.clock.now()),
                Some(Duration::from_secs(1)),
                smithay::desktop::utils::surface_primary_scanout_output
            );
        }

        Ok(())
    }

    fn handle <B: InputBackend> (&mut self, event: InputEvent<B>) {
        debug!(self.logger, "Received input event")
    }

}

pub struct WindowState {
    window: Window,
    center: Point<f64, Logical>,
    size:   Size<f64, Logical>
}

impl WindowState {
    pub fn new (window: Window) -> Self {
        Self { window, center: (0.0, 0.0).into(), size: (0.0, 0.0).into() }
    }
}

pub struct ScreenState {
    center: Point<f64, Logical>,
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

        //let mut damage_tracked_renderer = DamageTrackedRenderer::new((800, 600), 1.0, Transform::Normal);

        // Create the render elements from the surface
        //let location = Point::from((100, 100));
        //let render_elements: Vec<WaylandSurfaceRenderElement<_>> =
            //render_elements_from_surface_tree(&mut renderer, &surface, location, 1.0, log.clone());

        //// Render the element(s)
        //damage_tracked_renderer
            //.render_output(&mut renderer, 0, &*render_elements, [0.8, 0.8, 0.9, 1.0], log.clone())
            //.expect("failed to render output");

        // Render the windows in the current frame.
