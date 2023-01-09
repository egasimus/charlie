mod patch;

use crate::prelude::*;
use crate::engine::winit::patch::WinitEngineBackend;
use smithay::backend::winit::WinitEvent;
use smithay::reexports::winit::window::WindowId;
use smithay::output::{Output, PhysicalProperties, Subpixel, Mode};

pub struct WinitEngine {
    logger:  Logger,
    running: Arc<AtomicBool>,
    events:  EventLoop<'static, State>,
    display: Display<State>,
    backend: WinitEngineBackend,
    outputs: Vec<WinitOutput>,
    inputs:  Vec<WinitInput>
}

impl Stoppable for WinitEngine {
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
}

impl Engine for WinitEngine {
    fn display_handle (&self) -> DisplayHandle {
        self.display.handle()
    }
    fn event_handle (&self) -> LoopHandle<'static, State> {
        self.events.handle()
    }
    fn output_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(self.outputs.push(WinitOutput::new(name, &self.display, &mut self.backend)?))
    }
    fn input_add (&mut self) -> Result<(), Box<dyn Error>> {
        self.inputs.push(WinitInput {});
        unimplemented!();
    }
    fn start (&mut self, state: &mut State) {
        self.start_running();
        while self.is_running() {
            if self.backend.dispatch(|/*window_id,*/ event| match event {
                WinitEvent::Resized { size, scale_factor } => {
                    //panic!("host resize unsupported");
                }
                WinitEvent::Input(event) => {
                    state.on_input(event)
                }
                _ => (),
            }).is_err() {
                self.stop_running()
            }
            for output in self.outputs.iter() {
                output.render(&mut self.backend, state).unwrap();
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
            events:  EventLoop::try_new()?,
            display: Display::new()?,
            backend: WinitEngineBackend::new(logger)?,
            inputs:  vec![],
            outputs: vec![]
        })
    }
}

pub struct WinitInput {}

pub struct WinitOutput {
    output:      Output,
    host_window: WindowId,
}

impl WinitOutput {
    fn new (
        name: &str, display: &Display<State>, backend: &mut WinitEngineBackend
    ) -> Result<Self, Box<dyn Error>> {
        let output = Output::new(name.to_string(), PhysicalProperties {
            size:     (720, 540).into(),
            subpixel: Subpixel::Unknown,
            make:     "Smithay".into(),
            model:    "Winit".into()
        }, backend.logger.clone());
        output.set_preferred(Mode {
            size: (720, 540).into(),
            refresh: 60_000
        });
        let host_window = backend.window_add(display, name, 720.0, 540.0)?.id();
        Ok(Self { output, host_window })
    }
    fn render (&self, backend: &mut WinitEngineBackend, state: &mut State)
        -> Result<(), Box<dyn Error>>
    {
        backend.window_get(&self.host_window).render(|frame, size|{
            use smithay::utils::Rectangle;
            use smithay::backend::renderer::Frame;
            let rect: Rectangle<i32, Physical> = Rectangle::from_loc_and_size((0, 0), size);
            frame.clear([0.2,0.3,0.4,1.0], &[rect])?;
            state.render(frame, size)
        })
    }
}
