use crate::prelude::*;

use smithay::output::{PhysicalProperties, Subpixel, Mode};

use smithay::backend::{
    input::InputEvent,
    egl::{
        Error as EGLError, EGLContext, EGLSurface,
        native::XlibWindow,
        context::GlAttributes,
        display::EGLDisplay
    },
    renderer::Bind,
    winit::{
        Error as WinitError, WindowSize, WinitVirtualDevice, WinitEvent,
        WinitKeyboardInputEvent,
        WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
        WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
    }
};

use smithay::reexports::winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
    event_loop::{EventLoop as WinitEventLoop, ControlFlow},
    platform::run_return::EventLoopExtRunReturn,
    platform::unix::WindowExtUnix,
    window::{WindowId, WindowBuilder, Window as WinitWindow},
};

use wayland_egl as wegl;

type ScreenId = usize;

type WinitRenderData = (ScreenId, Size<i32, Physical>);

/// Contains the winit and wayland event loops, spawns one or more windows,
/// and dispatches events to them.
pub struct WinitEngine<W: 'static> {
    logger:       Logger,
    running:      Arc<AtomicBool>,
    started:      Option<Instant>,
    events:       EventLoop<'static, W>,
    winit_events: WinitEventLoop<()>,
    display:      Rc<RefCell<Display<W>>>,
    egl_display:  EGLDisplay,
    egl_context:  EGLContext,
    renderer:     Gles2Renderer,
    outputs:      HashMap<WindowId, WinitHostWindow>,
}

impl<W> Stoppable for WinitEngine<W> {

    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }

}

impl<W> Engine for WinitEngine<W> where W: Widget<RenderData=WinitRenderData> {

    type State = W;

    fn logger (&self) -> Logger {
        self.logger.clone()
    }

    fn display (&self) -> &Rc<RefCell<Display<W>>> {
        &self.display
    }

    fn events (&self) -> &EventLoop<'static, W> {
        &self.events
    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        &mut self.renderer
    }

    fn output_add (&mut self, name: &str, screen: ScreenId) -> Result<(), Box<dyn Error>> {
        let window = WinitHostWindow::new(
            &self.logger, &self.winit_events, &Self::make_context(&self.logger, &self.egl_context)?,
            &format!("Output {screen}"), 720.0, 540.0,
            screen
        )?;
        let window_id = window.id();
        self.outputs.insert(window_id, window);
        Ok(())
    }

    fn input_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn tick (&mut self, state: &mut W) -> Result<(), Box<dyn Error>> {
        // Dispatch input events
        self.dispatch(|event| match event {
            WinitEvent::Resized { size, scale_factor } => {
                //panic!("host resize unsupported");
            }
            WinitEvent::Input(event) => {
                state.handle(event)
            }
            _ => (),
        })?;
        // Render each output
        for (_, output) in self.outputs.iter() {
            output.render(&mut self.renderer, state)?;
        }
        // Advance the event loop
        self.events.dispatch(Some(Duration::from_millis(1)), state)?;
        state.refresh()?;
        Ok(self.display.borrow_mut().flush_clients()?)
    }

}

impl<W: Widget + 'static> WinitEngine<W> {

    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Starting Winit engine");
        let winit_events = WinitEventLoop::new();
        let window = Arc::new(WindowBuilder::new() // Null window to host the EGLDisplay
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&winit_events)
            .map_err(WinitError::InitFailed)?);
        let egl_display = EGLDisplay::new(window, logger.clone()).unwrap();
        let egl_context = EGLContext::new_with_config(&egl_display, GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        }, Default::default(), logger.clone())?;
        let renderer = Self::make_renderer(logger, &egl_context)?;
        Ok(Self {
            logger:       logger.clone(),
            running:      Arc::new(AtomicBool::new(true)),
            started:      None,
            events:       EventLoop::try_new()?,
            winit_events,
            display:      Rc::new(RefCell::new(Display::new()?)),
            egl_display,
            egl_context,
            renderer,
            outputs:      HashMap::new(),
        })
    }

    fn make_context (logger: &Logger, egl: &EGLContext) -> Result<EGLContext, Box<dyn Error>> {
        Ok(EGLContext::new_shared_with_config(egl.display(), egl, GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        }, Default::default(), logger.clone())?)
    }

    fn make_renderer (logger: &Logger, egl: &EGLContext) -> Result<Gles2Renderer, Box<dyn Error>> {
        let egl = Self::make_context(logger, egl)?;
        Ok(unsafe { Gles2Renderer::new(egl, logger.clone()) }?)
    }

    pub fn window_get (&mut self, window_id: &WindowId) -> &mut WinitHostWindow {
        self.outputs.get_mut(&window_id).unwrap()
    }

    pub fn dispatch (&mut self, mut callback: impl FnMut(WinitEvent))
        -> Result<(), WinitHostError>
    {
        let mut closed = false;
        if self.started.is_none() {
            let event = InputEvent::DeviceAdded { device: WinitVirtualDevice };
            callback(WinitEvent::Input(event));
            self.started = Some(Instant::now());
        }
        let started = &self.started.unwrap();
        let logger  = &self.logger;
        let outputs = &mut self.outputs;
        self.winit_events.run_return(move |event, _target, control_flow| {
            //debug!(self.logger, "{target:?}");
            match event {
                Event::RedrawEventsCleared => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::RedrawRequested(_id) => {
                    callback(WinitEvent::Refresh);
                }
                Event::WindowEvent { window_id, event } => match outputs.get_mut(&window_id) {
                    Some(window) => {
                        window.dispatch(started, event, &mut callback);
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
            Err(WinitHostError::WindowClosed)
        } else {
            Ok(())
        }
    }

}

#[derive(Debug)]
pub struct WinitHostWindow {
    logger:   Logger,
    title:    String,
    width:    f64,
    height:   f64,
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

impl WinitHostWindow {

    /// Create a new host window
    pub fn new (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        egl:    &EGLContext,
        title:  &str,
        width:  f64,
        height: f64,
        screen: ScreenId
    ) -> Result<Self, Box<dyn Error>> {

        let (w, h, hz, subpixel) = (720, 540, 60_000, Subpixel::Unknown);

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

    /// Render this output into its corresponding host window
    pub fn render (
        &self,
        renderer: &mut Gles2Renderer,
        state:    &mut impl Widget<RenderData=WinitRenderData>
    ) -> Result<(), Box<dyn Error>> {
        if let Some(size) = self.resized.take() {
            self.surface.resize(size.w, size.h, 0, 0);
        }
        renderer.bind(self.surface.clone())?;
        let size = self.surface.get_size().unwrap();
        state.render(RenderContext {
            renderer, output: &self.output, data: (self.screen, size)
        })?;
        self.surface.swap_buffers(None)?;
        Ok(())
    }

    /// Build the window
    fn build (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        title:  &str,
        width:  f64,
        height: f64
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

    /// Dispatch input events from the host window to the hosted compositor.
    fn dispatch (
        &mut self,
        started:  &Instant,
        event:    WindowEvent,
        callback: &mut impl FnMut(WinitEvent)
    ) -> () {
        debug!(self.logger, "Winit Window Event: {self:?} {event:?}");
        let duration = Instant::now().duration_since(*started);
        let nanos = duration.subsec_nanos() as u64;
        let time = ((1000 * duration.as_secs()) + (nanos / 1_000_000)) as u32;
        match event {

            WindowEvent::Resized(psize) => {
                trace!(self.logger, "Resizing window to {:?}", psize);
                let scale_factor = self.window.scale_factor();
                let mut wsize    = self.size.borrow_mut();
                let (pw, ph): (u32, u32) = psize.into();
                wsize.physical_size = (pw as i32, ph as i32).into();
                wsize.scale_factor  = scale_factor;
                self.resized.set(Some(wsize.physical_size));
                callback(WinitEvent::Resized {
                    size: wsize.physical_size,
                    scale_factor,
                });
            }

            WindowEvent::Focused(focus) => {
                callback(WinitEvent::Focus(focus));
            }

            WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, } => {
                let mut wsize = self.size.borrow_mut();
                wsize.scale_factor = scale_factor;
                let (pw, ph): (u32, u32) = (*new_inner_size).into();
                self.resized.set(Some((pw as i32, ph as i32).into()));
                callback(WinitEvent::Resized {
                    size: (pw as i32, ph as i32).into(),
                    scale_factor: wsize.scale_factor,
                });
            }

            WindowEvent::KeyboardInput { input, .. } => {
                let KeyboardInput { scancode, state, .. } = input;
                match state {
                    ElementState::Pressed => self.rollover += 1,
                    ElementState::Released => {
                        self.rollover = self.rollover.checked_sub(1).unwrap_or(0)
                    }
                };
                callback(WinitEvent::Input(InputEvent::Keyboard {
                    event: WinitKeyboardInputEvent {
                        time, key: scancode, count: self.rollover, state,
                    },
                }));
            }

            WindowEvent::CursorMoved { position, .. } => {
                let lpos = position.to_logical(self.size.borrow().scale_factor);
                callback(WinitEvent::Input(InputEvent::PointerMotionAbsolute {
                    event: WinitMouseMovedEvent {
                        size: self.size.clone(), time, logical_position: lpos,
                    },
                }));
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let event = WinitMouseWheelEvent { time, delta };
                callback(WinitEvent::Input(InputEvent::PointerAxis { event }));
            }

            WindowEvent::MouseInput { state, button, .. } => {
                callback(WinitEvent::Input(InputEvent::PointerButton {
                    event: WinitMouseInputEvent {
                        time, button, state, is_x11: self.is_x11,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Started, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(WinitEvent::Input(InputEvent::TouchDown {
                    event: WinitTouchStartedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Moved, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Ended, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
                callback(WinitEvent::Input(InputEvent::TouchUp {
                    event: WinitTouchEndedEvent { time, id },
                }))
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Cancelled, id, .. }) => {
                callback(WinitEvent::Input(InputEvent::TouchCancel {
                    event: WinitTouchCancelledEvent { time, id },
                }));
            }

            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                callback(WinitEvent::Input(InputEvent::DeviceRemoved {
                    device: WinitVirtualDevice,
                }));
                warn!(self.logger, "Window closed");
                self.closing = true;
            }

            _ => {}

        }
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
