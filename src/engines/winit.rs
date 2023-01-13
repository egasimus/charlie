use crate::prelude::*;

use smithay::{
    delegate_dmabuf,
    output::{PhysicalProperties, Subpixel, Mode},
    backend::{
        allocator::dmabuf::Dmabuf,
        input::InputEvent,
        egl::{
            Error as EGLError, EGLContext, EGLSurface,
            native::XlibWindow,
            context::GlAttributes,
            display::EGLDisplay
        },
        renderer::{
            Bind,
            ImportDma,
            ImportEgl
        },
        winit::{
            Error as WinitError, WindowSize, WinitVirtualDevice, WinitEvent,
            WinitKeyboardInputEvent,
            WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
            WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
        }
    },
    wayland::{
        buffer::BufferHandler,
        dmabuf::{
            DmabufGlobal,
            DmabufState,
            DmabufHandler,
            DmabufGlobalData
        },
        output::{
            OutputManagerState
        }
    },
    reexports::{
        wayland_server::GlobalDispatch,
        wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        winit::{
            dpi::LogicalSize,
            event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
            event_loop::{EventLoop as WinitEventLoop, ControlFlow},
            platform::run_return::EventLoopExtRunReturn,
            platform::unix::WindowExtUnix,
            window::{WindowId, WindowBuilder, Window as WinitWindow},
        }
    }
};

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::{ShmHandler, ShmState};

use wayland_egl as wegl;

type ScreenId = usize;

type WinitRenderData = (ScreenId, Size<i32, Physical>);

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
    flush:        FlushCallback,
}

impl Engine for WinitEngine {

    /// Initialize winit engine
    fn new (
        logger:  &Logger,
        display: &DisplayHandle,
        flush:   FlushCallback
    ) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Starting Winit engine");
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
        let mut renderer = Self::make_renderer(logger, &egl_context)?;
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
            flush
        })
    }

    fn logger (&self) -> Logger {
        self.logger.clone()
    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        &mut self.renderer
    }

    fn output_add (
        &mut self,
        name:   &str,
        screen: ScreenId,
        width:  i32,
        height: i32
    ) -> Result<(), Box<dyn Error>> {
        let window = WinitHostWindow::new(
            &self.logger, &self.events, &Self::make_context(&self.logger, &self.egl_context)?,
            &format!("Output {screen}"), width, height,
            screen
        )?;
        let window_id = window.id();
        self.outputs.insert(window_id, window);
        Ok(())
    }

    fn input_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn tick (&mut self, state: &mut impl Widget<WinitRenderData>) -> Result<(), Box<dyn Error>> {
        // Dispatch input events
        self.dispatch(|screen_id, event| match event {
            WinitEvent::Resized { size, scale_factor } => {
                crit!(self.logger, "host resize unsupported");
            }
            WinitEvent::Input(event) => {
                state.update(screen_id, event)
            }
            _ => (),
        })?;
        // Render each output
        for (_, output) in self.outputs.iter() {
            output.render(&mut self.renderer, state)?;
        }
        // Advance the event loop
        Ok((self.flush)(state)?)
    }

}


impl Stoppable for WinitEngine {

    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
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
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(&mut self, _global: &smithay::wayland::dmabuf::DmabufGlobal, dmabuf: smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), smithay::wayland::dmabuf::ImportError> {
        self.renderer.import_dmabuf(&dmabuf, None).map(|_| ()).map_err(|_| smithay::wayland::dmabuf::ImportError::Failed)
    }
}

delegate_dmabuf!(WinitEngine);

impl WinitEngine {

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

    pub fn dispatch (&mut self, mut callback: impl FnMut(ScreenId, WinitEvent))
        -> Result<(), WinitHostError>
    {
        let mut closed = false;
        if self.started.is_none() {
            let event = InputEvent::DeviceAdded { device: WinitVirtualDevice };
            callback(0, WinitEvent::Input(event));
            self.started = Some(Instant::now());
        }
        let started = &self.started.unwrap();
        let logger  = &self.logger;
        let outputs = &mut self.outputs;
        self.events.run_return(move |event, _target, control_flow| {
            //debug!(self.logger, "{target:?}");
            match event {
                Event::RedrawEventsCleared => {
                    *control_flow = ControlFlow::Exit;
                }
                Event::RedrawRequested(_id) => {
                    callback(0, WinitEvent::Refresh);
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

impl WinitHostWindow {

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

    /// Render this output into its corresponding host window
    pub fn render (
        &self,
        renderer: &mut Gles2Renderer,
        state:    &mut impl Widget<WinitRenderData>
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

    /// Dispatch input events from the host window to the hosted compositor.
    fn dispatch (
        &mut self,
        started:  &Instant,
        event:    WindowEvent,
        callback: &mut impl FnMut(ScreenId, WinitEvent)
    ) -> () {
        //debug!(self.logger, "Winit Window Event: {self:?} {event:?}");
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
                callback(self.screen, WinitEvent::Resized {
                    size: wsize.physical_size,
                    scale_factor,
                });
            }

            WindowEvent::Focused(focus) => {
                callback(self.screen, WinitEvent::Focus(focus));
            }

            WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, } => {
                let mut wsize = self.size.borrow_mut();
                wsize.scale_factor = scale_factor;
                let (pw, ph): (u32, u32) = (*new_inner_size).into();
                self.resized.set(Some((pw as i32, ph as i32).into()));
                callback(self.screen, WinitEvent::Resized {
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
                callback(self.screen, WinitEvent::Input(InputEvent::Keyboard {
                    event: WinitKeyboardInputEvent {
                        time, key: scancode, count: self.rollover, state,
                    },
                }));
            }

            WindowEvent::CursorMoved { position, .. } => {
                let lpos = position.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::PointerMotionAbsolute {
                    event: WinitMouseMovedEvent {
                        size: self.size.clone(), time, logical_position: lpos,
                    },
                }));
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let event = WinitMouseWheelEvent { time, delta };
                callback(self.screen, WinitEvent::Input(InputEvent::PointerAxis { event }));
            }

            WindowEvent::MouseInput { state, button, .. } => {
                callback(self.screen, WinitEvent::Input(InputEvent::PointerButton {
                    event: WinitMouseInputEvent {
                        time, button, state, is_x11: self.is_x11,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Started, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchDown {
                    event: WinitTouchStartedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Moved, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Ended, location, id, .. }) => {
                let location = location.to_logical(self.size.borrow().scale_factor);
                callback(self.screen, WinitEvent::Input(InputEvent::TouchMotion {
                    event: WinitTouchMovedEvent {
                        size: self.size.clone(), time, location, id,
                    },
                }));
                callback(self.screen, WinitEvent::Input(InputEvent::TouchUp {
                    event: WinitTouchEndedEvent { time, id },
                }))
            }

            WindowEvent::Touch(Touch { phase: TouchPhase::Cancelled, id, .. }) => {
                callback(self.screen, WinitEvent::Input(InputEvent::TouchCancel {
                    event: WinitTouchCancelledEvent { time, id },
                }));
            }

            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                callback(self.screen, WinitEvent::Input(InputEvent::DeviceRemoved {
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
