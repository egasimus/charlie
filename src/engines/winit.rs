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
    renderer::{Bind, Renderer, Frame},
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

pub struct WinitEngine<W: 'static> {
    logger:  Logger,
    running: Arc<AtomicBool>,
    events:  EventLoop<'static, W>,
    display: Rc<RefCell<Display<W>>>,
    winit_host:    WinitHost,
    winit_outputs: Vec<WinitOutput>,
    winit_inputs:  Vec<WinitInput>
}

impl<W> Stoppable for WinitEngine<W> {
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
}

impl<W: Widget + 'static> WinitEngine<W> {
    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Starting Winit engine");
        Ok(Self {
            logger:  logger.clone(),
            running: Arc::new(AtomicBool::new(true)),
            events:  EventLoop::try_new()?,
            display: Rc::new(RefCell::new(Display::new()?)),
            winit_host:    WinitHost::new(logger)?,
            winit_inputs:  vec![],
            winit_outputs: vec![]
        })
    }
}

type ScreenId = usize;

impl<W: Widget<RenderData=ScreenId>> Engine<W> for WinitEngine<W> {
    fn logger (&self) -> Logger {
        self.logger.clone()
    }
    fn display_handle (&self) -> DisplayHandle {
        self.display.borrow().handle()
    }
    fn display_fd (&self) -> i32 {
        self.display.borrow_mut().backend().poll_fd().as_raw_fd()
    }
    fn display_dispatcher (&self) -> Box<dyn Fn(&mut W) -> Result<usize, std::io::Error>> {
        let display = self.display.clone();
        Box::new(move |widget| { display.borrow_mut().dispatch_clients(widget) })
    }
    fn event_handle (&self) -> LoopHandle<'static, W> {
        self.events.handle()
    }
    fn renderer (&mut self) -> &mut Gles2Renderer {
        self.winit_host.renderer()
    }
    fn output_add (&mut self, name: &str, screen: ScreenId) -> Result<(), Box<dyn Error>> {
        Ok(self.winit_outputs.push(WinitOutput::new(name, &mut self.winit_host, screen)?))
    }
    fn input_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(self.winit_inputs.push(WinitInput::new(name)?))
    }
    fn tick (&mut self, widget: &mut W) -> Result<(), Box<dyn Error>> {
        // Dispatch input events
        self.winit_host.dispatch(|/*window_id,*/ event| match event {
            WinitEvent::Resized { size, scale_factor } => {
                //panic!("host resize unsupported");
            }
            WinitEvent::Input(event) => {
                widget.handle(event)
            }
            _ => (),
        })?;
        // Render each output
        for output in self.winit_outputs.iter_mut() {
            output.render(&mut self.winit_host, widget)?;
        }
        // Advance the event loop
        self.events.dispatch(Some(Duration::from_millis(1)), widget)?;
        widget.refresh()?;
        Ok(self.display.borrow_mut().flush_clients()?)
    }
}

pub struct WinitInput;

impl WinitInput {
    pub fn new (name: &str) -> Result<Self, Box<dyn Error>> {
        Ok(Self)
    }
}

/// An output bound to a winit host window.
#[derive(Debug)]
pub struct WinitOutput {
    /// Which host window contains this output
    id:     WindowId,
    /// Which screen is shown on this output
    screen: ScreenId,
    /// The output
    output: Output,
    /// Damage tracking
    damage: DamageTrackedRenderer,
}

impl WinitOutput {

    /// Create a new host window and attach an output to it.
    fn new (
        name:       &str,
        winit_host: &mut WinitHost,
        screen:     ScreenId,
    ) -> Result<Self, Box<dyn Error>> {
        let w = 720;
        let h = 540;
        let hz = 60_000;
        let output = Output::new(name.to_string(), PhysicalProperties {
            size:     (w, h).into(),
            subpixel: Subpixel::Unknown,
            make:     "Smithay".into(),
            model:    "Winit".into()
        }, winit_host.logger.clone());
        output.change_current_state(
            Some(Mode { size: (w, h).into(), refresh: hz }),
            None,
            None,
            None
        );
        Ok(Self {
            id: winit_host.window_add(name, 720.0, 540.0)?.id(),
            screen,
            damage: DamageTrackedRenderer::from_output(&output),
            output,
        })
    }

    /// Render this output into its corresponding host window
    fn render (
        &mut self,
        winit_host: &mut WinitHost,
        state:      &mut impl Widget<RenderData=ScreenId>
    ) -> Result<(), Box<dyn Error>> {
        winit_host.window_render(&self.id, &mut |renderer, size|{
            state.render(RenderContext {
                output: &self.output,
                data:   self.screen,
                renderer,
            })
        })
    }

}

/// Contains the main event loop, spawns one or more windows, and dispatches events to them.
pub struct WinitHost {
    pub logger: Logger,
    events:      WinitEventLoop<()>,
    started:     Option<Instant>,
    egl_display: EGLDisplay,
    egl_context: EGLContext,
    renderer:    Gles2Renderer,
    damage:      bool,
    windows:     HashMap<WindowId, WinitHostWindow>
}

impl WinitHost {

    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Initializing Winit event loop");
        let events = WinitEventLoop::new();
        let window = Arc::new(WindowBuilder::new() // Null window to host the EGLDisplay
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&events)
            .map_err(WinitError::InitFailed)?);
        let egl_display = EGLDisplay::new(window, logger.clone()).unwrap();
        let egl_context = EGLContext::new_with_config(&egl_display, GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        }, Default::default(), logger.clone())?;
        Ok(Self {
            logger:  logger.clone(),
            events,
            damage: egl_display.supports_damage(),
            egl_display,
            renderer: Self::make_renderer(logger, &egl_context)?,
            egl_context,
            started: None,
            windows: HashMap::new()
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

    pub fn renderer (&mut self) -> &mut Gles2Renderer {
        &mut self.renderer
    }

    pub fn window_add (
        &mut self, title: &str, width: f64, height: f64
    ) -> Result<&mut WinitHostWindow, Box<dyn Error>> {
        let egl = Self::make_context(&self.logger, &self.egl_context)?;
        let window = WinitHostWindow::new(&self.logger, &self.events, &egl, title, width, height)?;
        let window_id = window.id();
        self.windows.insert(window_id, window);
        Ok(self.window_get(&window_id))
    }

    pub fn window_get (&mut self, window_id: &WindowId) -> &mut WinitHostWindow {
        self.windows.get_mut(&window_id).unwrap()
    }

    pub fn window_render (
        &mut self,
        window_id: &WindowId,
        render: &mut impl FnMut(&mut Gles2Renderer, Size<i32, Physical>)->Result<(), Box<dyn Error>>
    ) -> Result<(), Box<dyn Error>> {
        let renderer = &mut self.renderer;
        let window   = self.windows.get_mut(&window_id).unwrap();
        window.render(renderer, render)
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
        let events  = &mut self.events;
        let windows = &mut self.windows;
        events.run_return(move |event, _target, control_flow| match event {
            Event::RedrawEventsCleared => {
                *control_flow = ControlFlow::Exit;
            }
            Event::RedrawRequested(_id) => {
                callback(WinitEvent::Refresh);
            }
            Event::WindowEvent { window_id, event } => match windows.get_mut(&window_id) {
                Some(window) => {
                    window.dispatch(started, event, &mut callback);
                    if window.closing {
                        windows.remove(&window_id);
                        closed = true;
                    }
                },
                None => {
                    warn!(logger, "Received event for unknown window id {window_id:?}")
                }
            }
            _ => {}
        });
        if closed {
            Err(WinitHostError::WindowClosed)
        } else {
            Ok(())
        }
    }

}

pub struct WinitHostWindow {
    logger:   Logger,
    title:    String,
    width:    f64,
    height:   f64,
    window:   Arc<WinitWindow>,
    closing:  bool,
    rollover: u32,
    surface:  Rc<EGLSurface>,
    resized:  Rc<Cell<Option<Size<i32, Physical>>>>,
    size:     Rc<RefCell<WindowSize>>,
    is_x11:   bool,
}

impl WinitHostWindow {

    /// Create a new host window
    pub fn new (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        egl:    &EGLContext,
        title:  &str,
        width:  f64,
        height: f64
    ) -> Result<Self, Box<dyn Error>> {
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
        })
    }

    /// Get the window id
    pub fn id (&self) -> WindowId {
        self.window.id()
    }

    /// Render something inside the window
    pub fn render (
        &mut self,
        renderer: &mut Gles2Renderer,
        render:   &mut impl FnMut(&mut Gles2Renderer, Size<i32, Physical>)->Result<(), Box<dyn Error>>
    ) -> Result<(), Box<dyn Error>> {
        if let Some(size) = self.resized.take() {
            self.surface.resize(size.w, size.h, 0, 0);
        }
        renderer.bind(self.surface.clone())?;
        let size = self.surface.get_size().unwrap();
        render(renderer, size)?;
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
    ) -> Result<Arc<WinitWindow>, Box<dyn Error>> {
        debug!(logger, "Building Winit window: {title} ({width}x{height})");
        let window = WindowBuilder::new()
            .with_inner_size(LogicalSize::new(width, height))
            .with_title(title)
            .with_visible(true)
            .build(events)
            .map_err(WinitError::InitFailed)?;
        Ok(Arc::new(window))
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

//use smithay::{
    //delegate_dmabuf,
    //backend::allocator::dmabuf::Dmabuf,
    //reexports::wayland_server::protocol::{
        //wl_buffer::WlBuffer,
        //wl_surface::WlSurface
    //},
    //wayland::{
        //buffer::BufferHandler,
        //dmabuf::{DmabufHandler, DmabufState, DmabufGlobal, ImportError}
    //}
//};

//impl BufferHandler for WinitHostWindow {
    //fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
//}

//impl DmabufHandler for WinitHostWindow {
    //fn dmabuf_state(&mut self) -> &mut DmabufState {
        //&mut self.dmabuf_state
    //}

    //fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf) -> Result<(), ImportError> {
        //self.renderer
            //.import_dmabuf(&dmabuf, None)
            //.map(|_| ())
            //.map_err(|_| ImportError::Failed)
    //}
//}

//delegate_dmabuf!(WinitHostWindow);