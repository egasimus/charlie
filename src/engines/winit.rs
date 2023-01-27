use crate::prelude::*;

use smithay::{
    output::{PhysicalProperties, Subpixel, Mode},
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::{
            Error as EGLError, EGLContext, EGLSurface,
            native::XlibWindow,
            context::GlAttributes,
            display::EGLDisplay
        },
        renderer::{Bind, ImportDma, ImportEgl},
        input::InputEvent,
        winit::{
            Error as WinitError,
            WindowSize,
            WinitInput,
            WinitEvent,
            WinitVirtualDevice,
            WinitKeyboardInputEvent,
            WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
            WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
        }
    },
    wayland::{
        buffer::BufferHandler,
        dmabuf::{DmabufGlobal, DmabufState, DmabufHandler, ImportError},
        output::{OutputManagerState},
        shm::{ShmHandler, ShmState}
    },
    reexports::{
        winit::{
            dpi::LogicalSize,
            event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
            event_loop::{ControlFlow, EventLoop as WinitEventLoop},
            platform::run_return::EventLoopExtRunReturn,
            platform::unix::WindowExtUnix,
            window::{WindowId, WindowBuilder, Window as WinitWindow},
        },
        wayland_server::protocol::wl_buffer::WlBuffer
    }
};

use wayland_egl as wegl;

smithay::delegate_output!(App<WinitEngine>);

smithay::delegate_shm!(App<WinitEngine>);

smithay::delegate_dmabuf!(App<WinitEngine>);

/// Contains the winit and wayland event loops, spawns one or more windows,
/// and dispatches events to them.
pub struct WinitEngine {
    logger:        Logger,
    running:       Arc<AtomicBool>,
    started:       Cell<Option<Instant>>,
    winit_events:  Rc<RefCell<WinitEventLoop<()>>>,
    egl_display:   EGLDisplay,
    egl_context:   EGLContext,
    renderer:      Rc<RefCell<Gles2Renderer>>,
    shm:           ShmState,
    dmabuf_state:  DmabufState,
    dmabuf_global: DmabufGlobal,
    outputs:       RefCell<HashMap<WindowId, WinitHostWindow>>,
    out_manager:   OutputManagerState,
}

impl Engine for WinitEngine {

    /// Initialize winit engine
    fn new (logger: &Logger, display: &DisplayHandle) -> Result<Self, Box<dyn Error>> {

        debug!(logger, "Starting Winit engine");

        // Create the Winit event loop
        let winit_events = WinitEventLoop::new();

        // Create a null window to host the EGLDisplay
        let window = Arc::new(WindowBuilder::new()
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&winit_events)
            .map_err(WinitError::InitFailed)?);

        // Create the renderer and EGL context
        let egl_display = EGLDisplay::new(window, logger.clone()).unwrap();
        let egl_context = EGLContext::new_with_config(&egl_display, GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        }, Default::default(), logger.clone())?;
        let mut renderer = make_renderer(logger, &egl_context)?;

        // Init dmabuf support
        renderer.bind_wl_display(&display)?;
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = dmabuf_state.create_global::<App<Self>, _>(
            display,
            renderer.dmabuf_formats().cloned().collect::<Vec<_>>(),
            logger.clone(),
        );

        Ok(Self {
            logger:        logger.clone(),
            shm:           ShmState::new::<App<Self>, _>(&display, vec![], logger.clone()),
            out_manager:   OutputManagerState::new_with_xdg_output::<App<Self>>(&display),
            running:       Arc::new(AtomicBool::new(true)),
            started:       Cell::new(None),
            winit_events:  Rc::new(RefCell::new(winit_events)),
            egl_display,
            egl_context,
            dmabuf_state,
            dmabuf_global,
            renderer:      Rc::new(RefCell::new(renderer)),
            outputs:       RefCell::new(HashMap::new()),
        })
    }

    fn logger (&self) -> Logger {
        self.logger.clone()
    }

    fn renderer (&self) -> RefMut<Gles2Renderer> {
        self.renderer.borrow_mut()
    }

    /// Render to each host window
    fn render <R: EngineApp<Self> + 'static> (app: &mut R) -> StdResult<()> {
        let engine = app.engine();
        let mut renderer = engine.renderer();
        for (_, output) in engine.outputs.borrow().iter() {
            if let Some(size) = output.resized.take() {
                output.surface.resize(size.w, size.h, 0, 0);
            }
            renderer.bind(output.surface.clone())?;
            let size = output.surface.get_size().unwrap();
            app.render(&mut *renderer, &output.output, &size, output.screen)?;
            output.surface.swap_buffers(None)?;
        }
        Ok(())
    }

    /// Dispatch input events from the host window to the hosted root widget.
    fn update <U: EngineApp<Self> + 'static> (app: &mut U) -> StdResult<()> {
        let engine = app.engine();
        let mut closed = false;
        if engine.started.get().is_none() {
            //let event = InputEvent::DeviceAdded { device: WinitVirtualDevice };
            //callback(0, WinitEvent::Input(event));
            engine.started.set(Some(Instant::now()));
        }
        let started = &engine.started.get().unwrap();
        let logger = engine.logger.clone();
        let winit_events = engine.winit_events.clone();
        winit_events.borrow_mut().run_return(|event, _target, control_flow| {
            //debug!(self.logger, "{target:?}");
            match event {
                Event::RedrawEventsCleared => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::RedrawRequested(_id) => {
                    //callback(0, WinitEvent::Refresh);
                }
                Event::WindowEvent { window_id, event } => {
                    closed = engine.window_update(&window_id, event)
                }
                _ => {}
            }
        });

        if closed {
            Err(WinitHostError::WindowClosed.into())
        } else {
            Ok(())
        }

    }

}

impl WinitEngine {

    pub fn window_add (&self, window: WinitHostWindow) -> () {
        let window_id = window.id();
        self.outputs.borrow_mut().insert(window_id, window);
    }

    pub fn window_update <'a> (&self, window_id: &WindowId, event: WindowEvent<'a>) -> bool {
        match self.outputs.borrow().get(window_id) {
            Some(window) => {
                let duration = Instant::now().duration_since(self.started.get().unwrap());
                let nanos    = duration.subsec_nanos() as u64;
                let time     = ((1000 * duration.as_secs()) + (nanos / 1_000_000)) as u32;
                let result   = match event {
                    WindowEvent::CloseRequested |
                    WindowEvent::Destroyed      |
                    WindowEvent::Resized(_)     |
                    WindowEvent::Focused(_)     |
                    WindowEvent::ScaleFactorChanged { .. }
                        => Self::update_window(time, window, event),
                    WindowEvent::KeyboardInput { .. }
                        => Self::update_keyboard(time, window, event),
                    WindowEvent::CursorMoved { .. } |
                    WindowEvent::MouseWheel  { .. } |
                    WindowEvent::MouseInput  { .. }
                        => Self::update_mouse(time, window, event),
                    WindowEvent::Touch { .. }
                        => Self::update_touch(time, window, event),
                    _ => vec![],
                };
                if window.closing.get() {
                    self.window_del(&window_id);
                    return true;
                }
            },
            None => {
                warn!(self.logger, "Received event for unknown window id {window_id:?}")
            }
        }
        false
    }

    pub fn window_del (&self, window_id: &WindowId) -> () {
        self.outputs.borrow_mut().remove(&window_id);
    }

    fn update_window <'a> (
        time: u32, window: &WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                window.closing.set(true);
                vec![WinitEvent::Input(InputEvent::DeviceRemoved { device: WinitVirtualDevice, })]
            }
            WindowEvent::Resized(psize) => {
                let scale_factor = window.window.scale_factor();
                let mut wsize    = window.size.borrow_mut();
                let (pw, ph): (u32, u32) = psize.into();
                wsize.physical_size = (pw as i32, ph as i32).into();
                wsize.scale_factor  = scale_factor;
                window.resized.set(Some(wsize.physical_size));
                vec![WinitEvent::Resized { size: wsize.physical_size, scale_factor, }]
            }
            WindowEvent::Focused(focus) => {
                vec![WinitEvent::Focus(focus)]
            }
            WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, } => {
                let mut wsize = window.size.borrow_mut();
                wsize.scale_factor = scale_factor;
                let (pw, ph): (u32, u32) = (*new_inner_size).into();
                window.resized.set(Some((pw as i32, ph as i32).into()));
                let size = (pw as i32, ph as i32).into();
                let scale_factor = wsize.scale_factor;
                vec![WinitEvent::Resized { size, scale_factor }]
            }
            _ => vec![]
        }
    }

    fn update_keyboard <'a> (
        time: u32, window: &WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        match event {
            WindowEvent::KeyboardInput { input, .. } => {
                let KeyboardInput { scancode, state, .. } = input;
                window.rollover.set(match state {
                    ElementState::Pressed
                        => window.rollover.get() + 1,
                    ElementState::Released
                        => window.rollover.get().checked_sub(1).unwrap_or(0)
                });
                let event = WinitKeyboardInputEvent {
                    time, key: scancode, count: window.rollover.get(), state,
                };
                vec![WinitEvent::Input(InputEvent::Keyboard { event })]
            }
            _ => vec![]
        }
    }

    fn update_mouse <'a> (
        time: u32, window: &WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let size = window.size.clone();
                let logical_position = position.to_logical(window.size.borrow().scale_factor);
                let event = WinitMouseMovedEvent { time, size, logical_position };
                vec![WinitEvent::Input(InputEvent::PointerMotionAbsolute { event })]
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let event = WinitMouseWheelEvent { time, delta };
                vec![WinitEvent::Input(InputEvent::PointerAxis { event })]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let event = WinitMouseInputEvent { time, button, state, is_x11: window.is_x11 };
                vec![WinitEvent::Input(InputEvent::PointerButton { event }) ]
            },
            _ => vec![]
        }
    }

    fn update_touch <'a> (
        time: u32, window: &WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        let mut events = vec![];
        let size   = window.size.clone();
        let scale  = window.size.borrow().scale_factor;
        match event {
            WindowEvent::Touch(Touch { phase: TouchPhase::Started, location, id, .. }) => {
                let location = location.to_logical(scale);
                let event    = WinitTouchStartedEvent { size, time, location, id };
                events.push(WinitEvent::Input(InputEvent::TouchDown { event }));
            }
            WindowEvent::Touch(Touch { phase: TouchPhase::Moved, location, id, .. }) => {
                let location = location.to_logical(scale);
                let event    = WinitTouchMovedEvent { size, time, location, id };
                events.push(WinitEvent::Input(InputEvent::TouchMotion { event }));
            }
            WindowEvent::Touch(Touch { phase: TouchPhase::Ended, location, id, .. }) => {
                let location = location.to_logical(scale);
                let event    = WinitTouchMovedEvent { size, time, location, id };
                events.push(WinitEvent::Input(InputEvent::TouchMotion { event }));
                let event    = WinitTouchEndedEvent { time, id };
                events.push(WinitEvent::Input(InputEvent::TouchUp { event }));
            }
            WindowEvent::Touch(Touch { phase: TouchPhase::Cancelled, id, .. }) => {
                let event    = WinitTouchCancelledEvent { time, id };
                events.push(WinitEvent::Input(InputEvent::TouchCancel { event }));
            }
            _ => {}
        };
        events
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
            &self.logger,
            &self.winit_events.borrow(),
            &make_context(&self.logger, &self.egl_context)?,
            &format!("Output {screen}"),
            width,
            height,
            screen
        )?;
        let window_id = window.id();
        self.outputs.borrow_mut().insert(window_id, window);
        Ok(())
    }
}

impl BufferHandler for App<WinitEngine> {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for App<WinitEngine> {
    fn shm_state(&self) -> &ShmState {
        &self.engine.shm
    }
}

impl DmabufHandler for App<WinitEngine> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.engine.dmabuf_state
    }
    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf) -> Result<(), ImportError> {
        self.engine.renderer()
            .import_dmabuf(&dmabuf, None)
            .map(|_| ())
            .map_err(|_| ImportError::Failed)
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

/// A window created by Winit, displaying a compositor output
#[derive(Debug)]
pub struct WinitHostWindow {
    logger:   Logger,
    title:    String,
    width:    i32,
    height:   i32,
    pub window:   WinitWindow,
    /// Count of currently pressed keys
    pub rollover: Cell<u32>,
    /// Is this winit window hosted under X11 (as opposed to a Wayland session?)
    pub is_x11:   bool,
    /// Which viewport is rendered to this window
    pub screen: ScreenId,
    /// The wayland output
    pub output:   Output,
    /// The drawing surface
    pub surface:  Rc<EGLSurface>,
    /// The current window size
    pub size:     Rc<RefCell<WindowSize>>,
    /// Whether a new size has been specified, to apply on next render
    pub resized:  Rc<Cell<Option<Size<i32, Physical>>>>,
    /// Whether the window is closing
    pub closing:  Cell<bool>,
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

        // Determine the window dimensions
        let (w, h, hz, subpixel) = (width, height, 60_000, Subpixel::Unknown);

        // Create a new compositor output matching the window
        let output = Output::new(title.to_string(), PhysicalProperties {
            size: (w, h).into(), subpixel, make: "Smithay".into(), model: "Winit".into()
        }, logger.clone());

        // Set the output's mode
        output.change_current_state(
            Some(Mode { size: (w, h).into(), refresh: hz }), None, None, None
        );

        // Build the host window
        let window = Self::build(logger, events, title, width, height)?;

        // Store the window's inner size
        let (w, h): (u32, u32) = window.inner_size().into();
        let size = WindowSize {
            physical_size: (w as i32, h as i32).into(),
            scale_factor:  window.scale_factor(),
        };

        Ok(Self {
            logger:   logger.clone(),
            closing:  Cell::new(false),
            rollover: Cell::new(0),
            is_x11:   window.wayland_surface().is_none(),
            screen,
            output,
            surface:  Self::surface(logger, egl, &window)?,
            window,
            width,
            height,
            size:     Rc::new(RefCell::new(size)),
            resized:  Rc::new(Cell::new(None)),
            title:    title.into(),
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
