mod patch;

use crate::prelude::*;
use crate::engine::winit::patch::{WinitEngineBackend, WinitEngineWindow};
use smithay::backend::winit::WinitEvent;
use smithay::reexports::winit::window::{WindowId, WindowBuilder, Window as WinitWindow};

pub struct WinitEngine {
    logger:   Logger,
    running:  Arc<AtomicBool>,
    backend:  WinitEngineBackend,
    events:   EventLoop<'static, ()>,
    outputs:  Vec<WinitOutput>,
    inputs:   Vec<WinitInput>
}

impl Stoppable for WinitEngine {
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
}

impl Engine for WinitEngine {
    fn output_add (&mut self) -> Result<(), Box<dyn Error>> {
        Ok(self.outputs.push(WinitOutput::new(&mut self.backend)?))
    }
    fn start (&mut self, app: &mut State) {
        self.start_running();
        while self.is_running() {
            if self.backend.dispatch(|event| match event {
                WinitEvent::Resized { size, scale_factor } => {
                    //panic!("host resize unsupported");
                }
                WinitEvent::Input(event) => {
                    app.on_input(event)
                }
                _ => (),
            }).is_err() {
                self.stop_running()
            }
        }
    }
}

impl WinitEngine {
    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "Starting Winit engine");
        Ok(Self {
            logger:  logger.clone(),
            running: Arc::new(AtomicBool::new(true)),
            backend: WinitEngineBackend::new(logger)?,
            events:  EventLoop::try_new()?,
            inputs:  vec![],
            outputs: vec![]
        })
    }
}

pub struct WinitOutput(WindowId);

impl WinitOutput {
    fn new (backend: &mut WinitEngineBackend) -> Result<Self, Box<dyn Error>> {
        let window = backend.window("Charlie", 720.0, 540.0)?;
        Ok(Self(window.id()))
    }
}

pub struct WinitInput {}
