use crate::prelude::*;

pub trait Widget {

    type RenderData;

    fn init (&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn render <'r> (&'r self, context: RenderContext<'r, Self::RenderData>) -> Result<(), Box<dyn Error>>;

    fn handle <B: InputBackend> (&mut self, event: InputEvent<B>);

}

pub struct RenderContext<'a, D> {
    pub renderer: &'a mut Gles2Renderer,
    pub output:   &'a Output,
    pub data:     D
}

pub trait Stoppable {

    fn running (&self) -> &Arc<AtomicBool>;

    fn is_running (&self) -> bool {
        self.running().load(Ordering::SeqCst)
    }
    fn start_running (&self) {
        self.running().store(true, Ordering::SeqCst)
    }
    fn stop_running (&self) {
        self.running().store(false, Ordering::SeqCst)
    }

}

pub trait Engine<W: Widget>: Stoppable + Sized {

    fn init (self) -> Result<Self, Box<dyn Error>> {
        Ok(self)
    }

    /// Obtain a copy of the logger.
    fn logger (&self) -> Logger;

    /// Obtain a handle to the display.
    fn display_handle (&self) -> DisplayHandle;

    /// Obtain a pollable file descriptor for the display.
    fn display_fd (&self) -> i32;

    /// Obtain a callable which dispatches display state to clients.
    fn display_dispatcher (&self) -> Box<dyn Fn(&mut W) -> Result<usize, std::io::Error>>;

    /// Obtain a handle to the event loop.
    fn event_handle (&self) -> LoopHandle<'static, W>;

    /// Obtain a mutable reference to the renderer.
    fn renderer (&mut self) -> &mut Gles2Renderer;

    /// Add an output to the host and bind it to a compositor screen
    fn output_add (&mut self, name: &str, screen: usize) -> Result<(), Box<dyn Error>>;

    fn output_change (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn output_remove (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_add (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_change (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_remove (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn start (&mut self, app: &mut W) -> Result<(), Box<dyn Error>> {
        app.init()?;
        self.start_running();
        while self.is_running() {
            if self.dispatch(app).is_err() {
                self.stop_running();
                break
            }
            self.tick(app)?;
        }
        Ok(())
    }

    fn dispatch (&mut self, state: &mut W) -> Result<(), Box<dyn Error>>;

    fn tick (&mut self, state: &mut W) -> Result<(), Box<dyn Error>>;

}

// TODO:
// fn render (self, renderer, area) -> RenderResult
// struct RenderResult { used, damages }
