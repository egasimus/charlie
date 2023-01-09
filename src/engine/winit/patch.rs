use crate::prelude::*;
use std::collections::HashMap;
use smithay::backend::egl::{EGLContext, EGLSurface, Error as EGLError};
use smithay::backend::egl::native::XlibWindow;
use smithay::backend::egl::context::GlAttributes;
use smithay::backend::egl::display::EGLDisplay;
use smithay::backend::input::InputEvent;
use smithay::backend::renderer::{Bind, Renderer, Frame, ImportEgl, ImportDma};
use smithay::backend::renderer::gles2::{Gles2Renderer, Gles2Frame};
use smithay::backend::winit::{
    Error as WinitError, WindowSize, WinitVirtualDevice, WinitEvent,
    WinitKeyboardInputEvent,
    WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
    WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
};
use smithay::reexports::winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
    event_loop::{EventLoop as WinitEventLoop, ControlFlow},
    platform::run_return::EventLoopExtRunReturn,
    platform::unix::WindowExtUnix,
    window::{WindowId, WindowBuilder, Window as WinitWindow},
};
use smithay::utils::{Rectangle, Transform};
use smithay::{
    delegate_dmabuf,
    backend::allocator::dmabuf::Dmabuf,
    reexports::wayland_server::protocol::{
        wl_buffer::WlBuffer,
        wl_surface::WlSurface
    },
    wayland::{
        buffer::BufferHandler,
        dmabuf::{DmabufHandler, DmabufState, DmabufGlobal, ImportError}
    }
};
use wayland_egl as wegl;

/// Contains the main event loop, spawns one or more windows, and dispatches events to them.
pub struct WinitEngineBackend {
    pub logger: Logger,
    events:  WinitEventLoop<()>,
    started: Option<Instant>,
    egl:     EGLDisplay,
    damage:  bool,
    windows: HashMap<WindowId, WinitEngineWindow>
}

impl WinitEngineBackend {

    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Initializing Winit event loop");
        let events = WinitEventLoop::new();
        // Null window to host the EGLDisplay
        let window = Arc::new(WindowBuilder::new()
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&events)
            .map_err(WinitError::InitFailed)?);
        let egl = EGLDisplay::new(window, logger.clone()).unwrap();
        let damage = egl.supports_damage();
        Ok(Self {
            logger:  logger.clone(),
            events,
            egl,
            damage,
            started: None,
            windows: HashMap::new()
        })
    }

    pub fn window_add (
        &mut self, display: &Display<State>, title: &str, width: f64, height: f64
    ) -> Result<&mut WinitEngineWindow, Box<dyn Error>> {
        let window = WinitEngineWindow::new(
            &self.logger, &self.events, display, &self.egl, title, width, height
        )?;
        let window_id = window.id();
        self.windows.insert(window_id, window);
        Ok(self.window_get(&window_id))
    }

    pub fn window_get (&mut self, window_id: &WindowId) -> &mut WinitEngineWindow {
        self.windows.get_mut(&window_id).unwrap()
    }

    pub fn dispatch (&mut self, mut callback: impl FnMut(WinitEvent))
        -> Result<(), WinitEngineBackendError>
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
                    }
                },
                None => {
                    warn!(logger, "Received event for unknown window id {window_id:?}")
                }
            }
            _ => {}
        });
        if closed {
            Err(WinitEngineBackendError::WindowClosed)
        } else {
            Ok(())
        }
    }

}

pub enum WinitEngineBackendError {
    WindowClosed,
}

pub struct WinitEngineWindow {
    logger:        Logger,
    title:         String,
    width:         f64,
    height:        f64,
    window:        Arc<WinitWindow>,
    closing:       bool,
    rollover:      u32,
    renderer:      Gles2Renderer,
    surface:       Rc<EGLSurface>,
    resized:       Rc<Cell<Option<Size<i32, Physical>>>>,
    size:          Rc<RefCell<WindowSize>>,
    is_x11:        bool,
    //dmabuf_state:  DmabufState,
    //dmabuf_global: DmabufGlobal
}

impl WinitEngineWindow {

    pub fn new (
        logger:  &Logger,
        events:  &WinitEventLoop<()>,
        display: &Display<State>,
        egl:     &EGLDisplay,
        title:   &str,
        width:   f64,
        height:  f64
    ) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Initializing Winit window: {title} ({width}x{height})");
        let (window_id, window) = Self::window_build(logger, events, title, width, height)?;
        debug!(logger, "Created Winit window: {title} ({width}x{height})");
        let (surface, renderer, is_x11) = Self::window_setup(logger, display, egl, &window)?;
        Ok(Self {
            logger:   logger.clone(),
            title:    title.into(),
            closing:  false,
            rollover: 0,
            size: {
                let (w, h): (u32, u32) = window.inner_size().into();
                Rc::new(RefCell::new(WindowSize {
                    physical_size: (w as i32, h as i32).into(),
                    scale_factor: window.scale_factor(),
                }))
            },
            resized: Rc::new(Cell::new(None)),
            width,
            height,
            window,
            surface,
            renderer,
            is_x11,
            //dmabuf_state,
            //dmabuf_global,
        })
    }

    fn window_build (
        logger: &Logger,
        events: &WinitEventLoop<()>,
        title:  &str,
        width:  f64,
        height: f64
    ) -> Result<(WindowId, Arc<WinitWindow>), Box<dyn Error>> {
        let window = WindowBuilder::new()
            .with_inner_size(LogicalSize::new(width, height))
            .with_title(title)
            .with_visible(true)
            .build(events)
            .map_err(WinitError::InitFailed)?;
        let window_id = window.id();
        let window = Arc::new(window);
        Ok((window_id, window))
    }

    fn window_setup (
        logger:  &Logger,
        display: &Display<State>,
        egl:     &EGLDisplay,
        window:  &WinitWindow
    ) -> Result<(Rc<EGLSurface>, Gles2Renderer, bool), Box<dyn Error>> {
        let gl_attributes = GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        };
        let context = EGLContext::new_with_config(
            egl, gl_attributes, Default::default(), logger.clone()
        )?;
        debug!(logger, "Created EGL context for Winit window");
        let is_x11 = !window.wayland_surface().is_some();
        let surface = if let Some(wl_surface) = window.wayland_surface() {
            debug!(logger, "Using Wayland backend for Winit window");
            let (width, height): (i32, i32) = window.inner_size().into();
            EGLSurface::new(
                egl,
                context.pixel_format().unwrap(),
                context.config_id(),
                unsafe {
                    wegl::WlEglSurface::new_from_raw(wl_surface as *mut _, width, height)
                }.map_err(|err| WinitError::Surface(err.into()))?,
                logger.clone(),
            ).map_err(EGLError::CreationFailed)?
        } else if let Some(xlib_window) = window.xlib_window().map(XlibWindow) {
            debug!(logger, "Using X11 backend for Winit window {xlib_window:?}");
            EGLSurface::new(
                egl,
                context.pixel_format().unwrap(),
                context.config_id(),
                xlib_window,
                logger.clone(),
            ).map_err(EGLError::CreationFailed)?
        } else {
            unreachable!("No backends for winit other then Wayland and X11 are supported")
        };
        debug!(logger, "Unbinding EGL context");
        let _ = context.unbind()?;
        let mut renderer = unsafe { Gles2Renderer::new(context, logger.clone())? };
        renderer.bind_wl_display(&display.handle())?;
        info!(logger, "EGL hardware-acceleration enabled");
        //let dmabuf_formats = renderer.dmabuf_formats().cloned().collect::<Vec<_>>();
        //let mut dmabuf_state = DmabufState::new();
        //let dmabuf_global = dmabuf_state.create_global::<WinitEngineWindow, _>(
            //&display.handle(), dmabuf_formats, self.logger.clone(),
        //);
        Ok((Rc::new(surface), renderer, is_x11))
    }

    pub fn id (&self) -> WindowId {
        self.window.id()
    }

    pub fn render (
        &mut self,
        render: impl Fn(&mut Gles2Frame, Size<i32, Physical>)->Result<(), Box<dyn Error>>
    ) -> Result<(), Box<dyn Error>> {
        self.renderer.bind(self.surface.clone())?;
        if let Some(size) = self.resized.take() {
            self.surface.resize(size.w, size.h, 0, 0);
        }
        let size = self.surface.get_size().unwrap();
        let mut frame = self.renderer.render(size, Transform::Normal)?;
        render(&mut frame, size)?;
        frame.finish()?;
        self.surface.swap_buffers(None)?;
        Ok(())
    }

    fn dispatch (
        &mut self, started: &Instant, event: WindowEvent, mut callback: &mut impl FnMut(WinitEvent)
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

//impl BufferHandler for WinitEngineWindow {
    //fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
//}

//impl DmabufHandler for WinitEngineWindow {
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

//delegate_dmabuf!(WinitEngineWindow);
