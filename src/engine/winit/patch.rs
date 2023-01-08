use crate::prelude::*;
use std::collections::HashMap;
use smithay::backend::egl::{EGLContext, EGLSurface, Error as EGLError};
use smithay::backend::egl::native::XlibWindow;
use smithay::backend::egl::context::GlAttributes;
use smithay::backend::egl::display::EGLDisplay;
use smithay::backend::input::InputEvent;
use smithay::backend::renderer::gles2::Gles2Renderer;
use smithay::backend::winit::{
    Error as WinitError, WindowSize, WinitVirtualDevice, WinitEvent,
    WinitKeyboardInputEvent,
    WinitMouseMovedEvent, WinitMouseWheelEvent, WinitMouseInputEvent,
    WinitTouchStartedEvent, WinitTouchMovedEvent, WinitTouchEndedEvent, WinitTouchCancelledEvent
};
use smithay::reexports::winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent, ElementState, KeyboardInput, Touch, TouchPhase},
    event_loop::{EventLoop, ControlFlow},
    platform::run_return::EventLoopExtRunReturn,
    platform::unix::WindowExtUnix,
    window::{WindowId, WindowBuilder, Window as WinitWindow},
};
use wayland_egl as wegl;

/// Contains the main event loop, spawns one or more windows, and dispatches events to them.
pub struct WinitEngineBackend {
    logger:  Logger,
    events:  EventLoop<()>,
    started: Option<Instant>,
    windows: HashMap<WindowId, WinitEngineWindow>
}

impl WinitEngineBackend {

    fn new (logger: &Logger) -> Self {
        debug!(logger, "Initializing Winit event loop.");
        Self {
            logger:  logger.clone(),
            events:  EventLoop::new(),
            started: None,
            windows: HashMap::new()
        }
    }

    fn window (
        &mut self, title: &str, width: f64, height: f64
    ) -> Result<&mut WinitEngineWindow, Box<dyn Error>> {
        debug!(self.logger, "Initializing Winit window: {title} ({width}x{height})");
        let window = WindowBuilder::new()
            .with_inner_size(LogicalSize::new(width, height))
            .with_title(title)
            .with_visible(true)
            .build(&self.events)
            .map_err(WinitError::InitFailed)?;
        let window_id = window.id();
        let window = Arc::new(window);
        debug!(self.logger, "Created Winit window: {title} ({width}x{height})");
        let display = EGLDisplay::new(window.clone(), self.logger.clone())?;
        debug!(self.logger, "Created EGL display for Winit window: {title} ({width}x{height})");
        let damage = display.supports_damage();
        debug!(self.logger, "Damage tracking for Winit window: {title} ({width}x{height}): {damage}");
        self.windows.insert(window_id, WinitEngineWindow {
            logger: self.logger.clone(),
            title:  title.into(),
            width,
            height,
            window,
            display,
            damage,
            closing: false,
            rollover: 0,
            state: None
        });
        Ok(self.windows.get_mut(&window_id).unwrap())
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

struct WinitEngineWindow {
    logger:   Logger,
    title:    String,
    width:    f64,
    height:   f64,
    window:   Arc<WinitWindow>,
    display:  EGLDisplay,
    damage:   bool,
    closing:  bool,
    rollover: u32,
    state:    Option<WinitEngineWindowState>,
}

impl WinitEngineWindow {

    fn init (mut self) -> Result<Self, Box<dyn Error>> {
        let Self { title, width, height, .. } = &self;
        if self.state.is_none() {
            let state = WinitEngineWindowState::init(&self)?;
            self.state = Some(state);
        } else {
            warn!(self.logger, "Winit window: {title} ({width}x{height}) already initialized");
        }
        Ok(self)
    }

    fn dispatch (
        &mut self, started: &Instant, event: WindowEvent, mut callback: &mut impl FnMut(WinitEvent)
    ) -> () {
        if let Some(state) = &self.state {
            let duration = Instant::now().duration_since(*started);
            let nanos = duration.subsec_nanos() as u64;
            let time = ((1000 * duration.as_secs()) + (nanos / 1_000_000)) as u32;
            match event {

                WindowEvent::Resized(psize) => {
                    trace!(self.logger, "Resizing window to {:?}", psize);
                    let scale_factor = self.window.scale_factor();
                    let mut wsize    = state.size.borrow_mut();
                    let (pw, ph): (u32, u32) = psize.into();
                    wsize.physical_size = (pw as i32, ph as i32).into();
                    wsize.scale_factor  = scale_factor;
                    state.resized.set(Some(wsize.physical_size));
                    callback(WinitEvent::Resized {
                        size: wsize.physical_size,
                        scale_factor,
                    });
                }

                WindowEvent::Focused(focus) => {
                    callback(WinitEvent::Focus(focus));
                }

                WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size, } => {
                    let mut wsize = state.size.borrow_mut();
                    wsize.scale_factor = scale_factor;
                    let (pw, ph): (u32, u32) = (*new_inner_size).into();
                    state.resized.set(Some((pw as i32, ph as i32).into()));
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
                    let lpos = position.to_logical(state.size.borrow().scale_factor);
                    callback(WinitEvent::Input(InputEvent::PointerMotionAbsolute {
                        event: WinitMouseMovedEvent {
                            size: state.size.clone(), time, logical_position: lpos,
                        },
                    }));
                }

                WindowEvent::MouseWheel { delta, .. } => {
                    let event = WinitMouseWheelEvent { time, delta };
                    callback(WinitEvent::Input(InputEvent::PointerAxis { event }));
                }

                WindowEvent::MouseInput { state: element_state, button, .. } => {
                    callback(WinitEvent::Input(InputEvent::PointerButton {
                        event: WinitMouseInputEvent {
                            time, button, state: element_state, is_x11: state.is_x11,
                        },
                    }));
                }

                WindowEvent::Touch(Touch { phase: TouchPhase::Started, location, id, .. }) => {
                    let location = location.to_logical(state.size.borrow().scale_factor);
                    callback(WinitEvent::Input(InputEvent::TouchDown {
                        event: WinitTouchStartedEvent {
                            size: state.size.clone(), time, location, id,
                        },
                    }));
                }

                WindowEvent::Touch(Touch { phase: TouchPhase::Moved, location, id, .. }) => {
                    let location = location.to_logical(state.size.borrow().scale_factor);
                    callback(WinitEvent::Input(InputEvent::TouchMotion {
                        event: WinitTouchMovedEvent {
                            size: state.size.clone(), time, location, id,
                        },
                    }));
                }

                WindowEvent::Touch(Touch { phase: TouchPhase::Ended, location, id, .. }) => {
                    let location = location.to_logical(state.size.borrow().scale_factor);
                    callback(WinitEvent::Input(InputEvent::TouchMotion {
                        event: WinitTouchMovedEvent {
                            size: state.size.clone(), time, location, id,
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
        } else {
            warn!(self.logger, "Event dispatched to this window before it's initialized")
        }
    }

}

struct WinitEngineWindowState {
    renderer: Gles2Renderer,
    surface:  Rc<EGLSurface>,
    resized:  Rc<Cell<Option<Size<i32, Physical>>>>,
    size:     Rc<RefCell<WindowSize>>,
    is_x11:   bool,
}

impl WinitEngineWindowState {

    /// Bind a Winit window to EGL.
    /// Works differently when Winit is running in Wayland vs X11.
    fn init (window: &WinitEngineWindow) -> Result<Self, Box<dyn Error>> {

        let WinitEngineWindow { title, width, height, .. } = window;

        let gl_attributes = GlAttributes {
            version: (3, 0), profile: None, vsync: true, debug: cfg!(debug_assertions),
        };

        let context = EGLContext::new_with_config(
            &window.display, gl_attributes, Default::default(), window.logger.clone()
        )?;

        debug!(window.logger, "Created EGL context for Winit window: {title} ({width}x{height})");

        let is_x11 = !window.window.wayland_surface().is_some();

        let surface = if let Some(wl_surface) = window.window.wayland_surface() {

            debug!(window.logger, "Using Wayland backend for Winit window: {title} ({width}x{height})");

            let (width, height): (i32, i32) = window.window.inner_size().into();

            EGLSurface::new(
                &window.display,
                context.pixel_format().unwrap(),
                context.config_id(),
                unsafe {
                    wegl::WlEglSurface::new_from_raw(wl_surface as *mut _, width, height)
                }.map_err(|err| WinitError::Surface(err.into()))?,
                window.logger.clone(),
            ).map_err(EGLError::CreationFailed)?

        } else if let Some(xlib_window) = window.window.xlib_window().map(XlibWindow) {

            debug!(window.logger, "Using X11 backend for Winit window: {title} ({width}x{height})");

            EGLSurface::new(
                &window.display,
                context.pixel_format().unwrap(),
                context.config_id(),
                xlib_window,
                window.logger.clone(),
            ).map_err(EGLError::CreationFailed)?

        } else {

            unreachable!("No backends for winit other then Wayland and X11 are supported")

        };

        context.unbind();

        let (w, h): (u32, u32) = window.window.inner_size().into();
        let size = Rc::new(RefCell::new(WindowSize {
            physical_size: (w as i32, h as i32).into(),
            scale_factor: window.window.scale_factor(),
        }));

        Ok(WinitEngineWindowState {
            renderer: unsafe { Gles2Renderer::new(context, window.logger.clone())?.into() },
            surface:  Rc::new(surface),
            resized:  Rc::new(Cell::new(None)),
            size,
            is_x11
        })

    }

}
