mod patch;

use crate::prelude::*;
use crate::engine::winit::patch::WinitHost;
use smithay::backend::winit::WinitEvent;
use smithay::reexports::winit::window::WindowId;
use smithay::output::{Output, PhysicalProperties, Subpixel, Mode};

pub struct WinitEngine {
    logger:  Logger,
    running: Arc<AtomicBool>,
    events:  EventLoop<'static, State>,
    display: Display<State>,
    host:    WinitHost,
    outputs: Vec<WinitOutput>,
    inputs:  Vec<WinitInput>
}

impl Stoppable for WinitEngine {
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
}

impl Engine for WinitEngine {
    fn logger (&self) -> Logger {
        self.logger.clone()
    }
    fn display_handle (&self) -> DisplayHandle {
        self.display.handle()
    }
    fn event_handle (&self) -> LoopHandle<'static, State> {
        self.events.handle()
    }
    fn renderer (&mut self) -> &mut Gles2Renderer {
        self.host.renderer()
    }
    fn output_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        Ok(self.outputs.push(WinitOutput::new(name, &self.display, &mut self.host)?))
    }
    fn input_add (&mut self) -> Result<(), Box<dyn Error>> {
        self.inputs.push(WinitInput {});
        unimplemented!();
    }
    fn start (&mut self, state: &mut State) {
        self.start_running();
        while self.is_running() {
            if self.host.dispatch(|/*window_id,*/ event| match event {
                WinitEvent::Resized { size, scale_factor } => {
                    //panic!("host resize unsupported");
                }
                WinitEvent::Input(event) => {
                    state.on_input(event)
                }
                _ => (),
            }).is_err() {
                self.stop_running();
                break
            }
            for output in self.outputs.iter() {
                output.render(&mut self.host, state).unwrap();
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
            host:    WinitHost::new(logger)?,
            inputs:  vec![],
            outputs: vec![]
        })
    }
}

pub struct WinitInput {}

pub struct WinitOutput {
    output:         Output,
    host_window_id: WindowId,
}

impl WinitOutput {

    fn new (
        name:    &str,
        display: &Display<State>,
        host:    &mut WinitHost
    ) -> Result<Self, Box<dyn Error>> {
        let output = Output::new(name.to_string(), PhysicalProperties {
            size:     (720, 540).into(),
            subpixel: Subpixel::Unknown,
            make:     "Smithay".into(),
            model:    "Winit".into()
        }, host.logger.clone());
        output.set_preferred(Mode {
            size: (720, 540).into(),
            refresh: 60_000
        });
        let host_window_id = host.window_add(display, name, 720.0, 540.0)?.id();
        Ok(Self { output, host_window_id })
    }

    fn render (&self, host: &mut WinitHost, state: &mut State)
        -> Result<(), Box<dyn Error>>
    {
        host.window_render(&self.host_window_id, &|frame, size|{
            use smithay::utils::Rectangle;
            use smithay::backend::renderer::Frame;
            let rect: Rectangle<i32, Physical> = Rectangle::from_loc_and_size((0, 0), size);
            frame.clear([0.2,0.3,0.4,1.0], &[rect])?;
            state.render(frame, size)?;
            Ok(())
        })
        //let renderer = host.renderer();
        //let host_window = host.window_get(&self.host_window_id);
        //)
    }

}
