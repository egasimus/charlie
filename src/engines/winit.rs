use crate::prelude::*;

mod window;
pub use window::*;

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

impl Engine for WinitEngine {

    /// Initialize winit engine
    fn new <W: Widget> (
        logger:  &Logger,
        display: &DisplayHandle,
    ) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Starting Winit engine");
        // Create the Winit event loop
        let events = WinitEventLoop::new();
        // Create a null window to host the EGLDisplay
        let window = Arc::new(WindowBuilder::new()
            .with_inner_size(LogicalSize::new(16, 16))
            .with_title("Charlie Null")
            .with_visible(false)
            .build(&events)
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

    fn update <W> (&mut self, state: W) -> StdResult<()> {
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
                        window.update((started, event, &mut callback));
                        if window.is_closing() {
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

pub type WinitRenderContext<'a> = &'a mut (
    &'a mut Gles2Renderer,
    &'a Output,
    Size<i32, Physical>,
    ScreenId
);

pub type WinitUpdateContext = (
    InputEvent<WinitInput>,
    ScreenId
);
