use crate::prelude::*;

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
        renderer::{Bind, ImportDma, ImportEgl},
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

use smithay::{
    backend::{
        input::InputEvent,
        winit::{
            WinitInput,
            WinitEvent,
            WinitVirtualDevice,
            WinitKeyboardInputEvent,
            WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
            WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
        }
    },
    reexports::{
        winit::{
            event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
            event_loop::ControlFlow,
            platform::run_return::EventLoopExtRunReturn,
        }
    }
};

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::{ShmHandler, ShmState};

use wayland_egl as wegl;

delegate_output!(@<X: Widget + 'static> App<WinitEngine, X>);
delegate_shm!(@<X: Widget + 'static> App<WinitEngine, X>);
delegate_dmabuf!(@<X: Widget + 'static> App<WinitEngine, X>);

/// Contains the winit and wayland event loops, spawns one or more windows,
/// and dispatches events to them.
pub struct WinitEngine {
    logger:       Logger,
    running:      Arc<AtomicBool>,
    started:      Option<Instant>,
    winit_events: WinitEventLoop<()>,
    egl_display:  EGLDisplay,
    egl_context:  EGLContext,
    renderer:     Gles2Renderer,
    shm:          ShmState,
    dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    outputs:      HashMap<WindowId, WinitHostWindow>,
    out_manager:  OutputManagerState,
}

impl Engine for WinitEngine {

    /// Initialize winit engine
    fn new <W: Widget> (
        logger:  &Logger,
        display: &DisplayHandle,
    ) -> Result<Self, Box<dyn Error>> {
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
        let dmabuf_state = if renderer.bind_wl_display(&display).is_ok() {
            info!(logger, "EGL hardware-acceleration enabled");
            let mut state = DmabufState::new();
            let global = state.create_global::<App<Self, W>, _>(
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
            shm:          ShmState::new::<App<Self, W>, _>(&display, vec![], logger.clone()),
            out_manager:  OutputManagerState::new_with_xdg_output::<App<Self, W>>(&display),
            running:      Arc::new(AtomicBool::new(true)),
            started:      None,
            winit_events,
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

    /// Render to each host window
    fn render <W: Widget> (&mut self, state: &mut W) -> StdResult<()> {
        for (_, output) in self.outputs.iter() {
            if let Some(size) = output.resized.take() {
                output.surface.resize(size.w, size.h, 0, 0);
            }
            self.renderer.bind(output.surface.clone())?;
            let size = output.surface.get_size().unwrap();
            state.render(&mut self.renderer, &output.output, &size, output.screen)?;
            output.surface.swap_buffers(None)?;
        }
        Ok(())
    }

    /// Dispatch input events from the host window to the hosted root widget.
    fn update <W: Widget> (&mut self, state: &mut W) -> StdResult<()> {

        let mut closed = false;

        if self.started.is_none() {
            //let event = InputEvent::DeviceAdded { device: WinitVirtualDevice };
            //callback(0, WinitEvent::Input(event));
            self.started = Some(Instant::now());
        }

        let started = &self.started.unwrap();
        let logger  = self.logger.clone();
        let outputs = &mut self.outputs;

        self.winit_events.run_return(move |event, _target, control_flow| {
            //debug!(self.logger, "{target:?}");
            match event {
                Event::RedrawEventsCleared => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::RedrawRequested(_id) => {
                    //callback(0, WinitEvent::Refresh);
                }
                Event::WindowEvent { window_id, event } => match outputs.get_mut(&window_id) {
                    Some(window) => {
                        let duration = Instant::now().duration_since(*started);
                        let nanos    = duration.subsec_nanos() as u64;
                        let time     = ((1000 * duration.as_secs()) + (nanos / 1_000_000)) as u32;
                        let result   = match event {
                            WindowEvent::CloseRequested |
                            WindowEvent::Destroyed      |
                            WindowEvent::Resized(_)     |
                            WindowEvent::Focused(_)     |
                            WindowEvent::ScaleFactorChanged { .. }
                                => self.update_window(time, window, event),
                            WindowEvent::KeyboardInput { .. }
                                => self.update_keyboard(time, window, event),
                            WindowEvent::CursorMoved { .. } |
                            WindowEvent::MouseWheel  { .. } |
                            WindowEvent::MouseInput  { .. }
                                => self.update_mouse(time, window, event),
                            WindowEvent::Touch { .. }
                                => self.update_touch(time, window, event),
                            _ => vec![],
                        };
                        if window.closing {
                            outputs.remove(&window_id);
                            closed = true;
                        }
                    },
                    None => {
                        warn!(logger, "Received event for unknown window id {window_id:?}")
                    }
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
    pub fn window_get (&mut self, window_id: &WindowId) -> &mut WinitHostWindow {
        self.outputs.get_mut(&window_id).unwrap()
    }

    fn update_window <'a> (
        &mut self, time: u32, window: &mut WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                warn!(self.logger, "Window closed");
                window.closing = true;
                vec![WinitEvent::Input(InputEvent::DeviceRemoved { device: WinitVirtualDevice, })]
            }
            WindowEvent::Resized(psize) => {
                trace!(self.logger, "Resizing window to {:?}", psize);
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
        &mut self, time: u32, window: &mut WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        match event {
            WindowEvent::KeyboardInput { input, .. } => {
                let KeyboardInput { scancode, state, .. } = input;
                match state {
                    ElementState::Pressed
                        => window.rollover += 1,
                    ElementState::Released
                        => window.rollover = window.rollover.checked_sub(1).unwrap_or(0)
                };
                let event = WinitKeyboardInputEvent {
                    time, key: scancode, count: window.rollover, state,
                };
                vec![WinitEvent::Input(InputEvent::Keyboard { event })]
            }
            _ => vec![]
        }
    }

    fn update_mouse <'a> (
        &mut self, time: u32, window: &mut WinitHostWindow, event: WindowEvent<'a>
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
        &mut self, time: u32, window: &mut WinitHostWindow, event: WindowEvent<'a>
    ) -> Vec<WinitEvent> {
        let events = vec![];
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
            &self.winit_events,
            &make_context(&self.logger, &self.egl_context)?,
            &format!("Output {screen}"),
            width,
            height,
            screen
        )?;
        let window_id = window.id();
        self.outputs.insert(window_id, window);
        Ok(())
    }
}

impl<X: Widget + 'static> BufferHandler for App<WinitEngine, X> {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl<X: Widget + 'static> ShmHandler for App<WinitEngine, X> {
    fn shm_state(&self) -> &ShmState {
        &self.engine.shm
    }
}

impl<X: Widget + 'static> DmabufHandler for App<WinitEngine, X> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.engine.dmabuf_state.as_mut().unwrap().0
    }
    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf) -> Result<(), ImportError> {
        self.engine.renderer.import_dmabuf(&dmabuf, None).map(|_| ()).map_err(|_| ImportError::Failed)
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
    pub rollover: u32,
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
    pub closing:  bool,
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
