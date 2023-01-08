mod patch;

use crate::prelude::*;

use smithay::backend::renderer::gles2::Gles2Renderer;
use smithay::backend::winit::{self, WinitGraphicsBackend, WinitEventLoop};

pub struct WinitEngine {
    logger:   Logger,
    running:  Arc<AtomicBool>,
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
        Ok(self.outputs.push(WinitOutput::new(&self.logger)?))
    }
}

impl WinitEngine {
    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        debug!(logger, "starting winit engine");
        Ok(Self {
            logger:  logger.clone(),
            running: Arc::new(AtomicBool::new(true)),
            events:  EventLoop::try_new()?,
            inputs:  vec![],
            outputs: vec![]
        })
    }
}

pub struct WinitOutput {
    logger:  Logger,
    display: Display<()>,
    backend: WinitGraphicsBackend<Gles2Renderer>,
    events:  WinitEventLoop,
    size:    Size<i32, Physical>
}

impl WinitOutput {
    fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        let display = Display::new()?;
        let (backend, events) = winit::init::<Gles2Renderer, _>(logger.clone())?;
        let size = backend.window_size().physical_size;
        debug!(logger, "new winit output {size:?}");
        Ok(Self {
            logger: logger.clone(),
            display,
            backend,
            events,
            size
        })
    }
}

pub struct WinitInput {}
