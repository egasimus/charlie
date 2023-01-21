use crate::prelude::*;

mod winit_update;
pub use winit_update::*;

mod winit_render;
pub use winit_render::*;

use smithay::{
    delegate_dmabuf,
    output::{PhysicalProperties, Subpixel, Mode},
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::{
            Error as EGLError, EGLContext, EGLSurface,
            native::XlibWindow,
            context::GlAttributes,
            display::EGLDisplay
        },
        renderer::{ImportDma, ImportEgl},
        winit::{Error as WinitError, WindowSize}
    },
    wayland::{
        buffer::BufferHandler,
        dmabuf::{DmabufGlobal, DmabufState, DmabufHandler, ImportError},
        output::{OutputManagerState}
    },
    reexports::{
        winit::{
            dpi::LogicalSize,
            event_loop::{EventLoop as WinitEventLoop},
            platform::unix::WindowExtUnix,
            window::{WindowId, WindowBuilder, Window as WinitWindow},
        }
    }
};

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::{ShmHandler, ShmState};

use wayland_egl as wegl;

type ScreenId = usize;

/// Contains the winit and wayland event loops, spawns one or more windows,
/// and dispatches events to them.
pub struct WinitEngine {
    logger:       Logger,
    running:      Arc<AtomicBool>,
    started:      Option<Instant>,
    events:       WinitEventLoop<()>,
    egl_display:  EGLDisplay,
    egl_context:  EGLContext,
    renderer:     Gles2Renderer,
    shm:          ShmState,
    dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    outputs:      HashMap<WindowId, WinitHostWindow>,
    out_manager:  OutputManagerState,
}

impl WinitEngine {
    pub fn window_get (&mut self, window_id: &WindowId) -> &mut WinitHostWindow {
        self.outputs.get_mut(&window_id).unwrap()
    }
}

impl<W> Engine<'static, WinitUpdateContext, WinitRenderContext<'static>, W> for WinitEngine where 
    W: Widget<'static, WinitUpdateContext, WinitRenderContext<'static>>,
{

    /// Initialize winit engine
    fn new (
        logger:  &Logger,
        display: &DisplayHandle,
    ) -> Result<Self, Box<dyn Error>> {

        debug!(logger, "Starting Winit engine");

        let events = WinitEventLoop::new();

        // Null window to host the EGLDisplay
        let window = Arc::new(WindowBuilder::new()
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&events)
            .map_err(WinitError::InitFailed)?);

        let egl_display = EGLDisplay::new(window, logger.clone()).unwrap();

        let egl_context = EGLContext::new_with_config(&egl_display, GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        }, Default::default(), logger.clone())?;

        let mut renderer = make_renderer(logger, &egl_context)?;

        let dmabuf_state = if renderer.bind_wl_display(&display).is_ok() {
            info!(logger, "EGL hardware-acceleration enabled");
            let mut state = DmabufState::new();
            let global = state.create_global::<Self, _>(
                display,
                renderer.dmabuf_formats().cloned().collect::<Vec<_>>(),
                logger.clone(),
            );
            Some((state, global))
        } else {
            None
        };

        Ok(Self {
            logger:       logger.clone(),
            shm:          ShmState::new::<Self, _>(&display, vec![], logger.clone()),
            out_manager:  OutputManagerState::new_with_xdg_output::<Self>(&display),
            running:      Arc::new(AtomicBool::new(true)),
            started:      None,
            events,
            egl_display,
            egl_context,
            dmabuf_state,
            renderer,
            outputs:      HashMap::new(),
        })
    }

    fn logger (&self) -> Logger {
        self.logger.clone()
    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        &mut self.renderer
    }

}

impl Inputs for WinitEngine {
    fn input_added (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

impl Outputs for WinitEngine {

    fn output_added (
        &mut self, name: &str, screen: ScreenId, width: i32, height: i32
    ) -> Result<(), Box<dyn Error>> {

        let window = WinitHostWindow::new(
            &self.logger, &self.events, &make_context(&self.logger, &self.egl_context)?,
            &format!("Output {screen}"), width, height,
            screen
        )?;

        let window_id = window.id();

        self.outputs.insert(window_id, window);

        Ok(())

    }

}

impl BufferHandler for WinitEngine {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

delegate_output!(WinitEngine);

impl ShmHandler for WinitEngine {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

delegate_shm!(WinitEngine);

impl DmabufHandler for WinitEngine {

    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf) -> Result<(), ImportError> {
        self.renderer.import_dmabuf(&dmabuf, None).map(|_| ()).map_err(|_| ImportError::Failed)
    }

}

delegate_dmabuf!(WinitEngine);

/// A window created by Winit, displaying a compositor output
#[derive(Debug)]
pub struct WinitHostWindow {
    logger:   Logger,
    title:    String,
    width:    i32,
    height:   i32,
    window:   WinitWindow,
    closing:  bool,
    rollover: u32,
    surface:  Rc<EGLSurface>,
    resized:  Rc<Cell<Option<Size<i32, Physical>>>>,
    size:     Rc<RefCell<WindowSize>>,
    is_x11:   bool,
    /// Which viewport is rendered to this window
    screen:   ScreenId,
    /// The wayland output
    output:   Output,
}

/// Build a host window
impl<'a> WinitHostWindow {

    /// Create a new host window
    pub fn new (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        egl:    &EGLContext,
        title:  &str,
        width:  i32,
        height: i32,
        screen: ScreenId
    ) -> Result<Self, Box<dyn Error>> {

        let (w, h, hz, subpixel) = (width, height, 60_000, Subpixel::Unknown);

        let output = Output::new(title.to_string(), PhysicalProperties {
            size: (w, h).into(), subpixel, make: "Smithay".into(), model: "Winit".into()
        }, logger.clone());

        output.change_current_state(
            Some(Mode { size: (w, h).into(), refresh: hz }), None, None, None
        );

        let window = Self::build(logger, events, title, width, height)?;

        let (w, h): (u32, u32) = window.inner_size().into();

        Ok(Self {
            logger:   logger.clone(),
            title:    title.into(),
            closing:  false,
            rollover: 0,
            size: Rc::new(RefCell::new(WindowSize {
                physical_size: (w as i32, h as i32).into(),
                scale_factor:  window.scale_factor(),
            })),
            width,
            height,
            resized: Rc::new(Cell::new(None)),
            surface: Self::surface(logger, egl, &window)?,
            is_x11:  window.wayland_surface().is_none(),
            window,
            screen,
            output
        })
    }

    /// Get the window id
    pub fn id (&self) -> WindowId {
        self.window.id()
    }

    /// Build the window
    fn build (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        title:  &str,
        width:  i32,
        height: i32
    ) -> Result<WinitWindow, Box<dyn Error>> {
        debug!(logger, "Building Winit window: {title} ({width}x{height})");
        let window = WindowBuilder::new()
            .with_inner_size(LogicalSize::new(width, height))
            .with_title(title)
            .with_visible(true)
            .build(events)
            .map_err(WinitError::InitFailed)?;
        Ok(window)
    }

    /// Obtain the window surface (varies on whether winit is running in wayland or x11)
    fn surface (
        logger: &Logger,
        egl:    &EGLContext,
        window: &WinitWindow
    ) -> Result<Rc<EGLSurface>, Box<dyn Error>> {
        debug!(logger, "Setting up Winit window: {window:?}");
        debug!(logger, "Created EGL context for Winit window");
        let surface = if let Some(surface) = window.wayland_surface() {
            Self::surface_wl(logger, &egl, window.inner_size().into(), surface)?
        } else if let Some(xlib_window) = window.xlib_window().map(XlibWindow) {
            Self::surface_x11(logger, &egl, xlib_window)?
        } else {
            unreachable!("No backends for winit other then Wayland and X11 are supported")
        };
        let _ = egl.unbind()?;
        Ok(Rc::new(surface))
    }

    /// Obtain the window surface when running in wayland
    fn surface_wl (
        logger:          &Logger,
        egl:             &EGLContext,
        (width, height): (i32, i32),
        surface:         *mut std::os::raw::c_void
    ) -> Result<EGLSurface, Box<dyn Error>> {
        debug!(logger, "Using Wayland backend for Winit window");
        Ok(EGLSurface::new(
            egl.display(),
            egl.pixel_format().unwrap(),
            egl.config_id(),
            unsafe {
                wegl::WlEglSurface::new_from_raw(surface as *mut _, width, height)
            }.map_err(|err| WinitError::Surface(err.into()))?,
            logger.clone(),
        )?)
    }

    /// Obtain the window surface when running in X11
    fn surface_x11 (
        logger: &Logger,
        egl:    &EGLContext,
        window: XlibWindow
    ) -> Result<EGLSurface, Box<dyn Error>> {
        debug!(logger, "Using X11 backend for Winit window {window:?}");
        Ok(EGLSurface::new(
            egl.display(),
            egl.pixel_format().unwrap(),
            egl.config_id(),
            window,
            logger.clone(),
        ).map_err(EGLError::CreationFailed)?)
    }

}

#[derive(Debug)]
pub enum WinitHostError {
    WindowClosed,
}

impl std::fmt::Display for WinitHostError {
    fn fmt (&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for WinitHostError {}

fn make_renderer (logger: &Logger, egl: &EGLContext) -> Result<Gles2Renderer, Box<dyn Error>> {
    let egl = make_context(logger, egl)?;
    Ok(unsafe { Gles2Renderer::new(egl, logger.clone()) }?)
}

fn make_context (logger: &Logger, egl: &EGLContext) -> Result<EGLContext, Box<dyn Error>> {
    Ok(EGLContext::new_shared_with_config(egl.display(), egl, GlAttributes {
        version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
    }, Default::default(), logger.clone())?)
}
