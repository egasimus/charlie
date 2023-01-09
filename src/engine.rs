use crate::prelude::*;

pub(crate) mod udev;
pub(crate) mod winit;

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

pub trait Engine: Stoppable + Sized {

    fn init (self) -> Result<Self, Box<dyn Error>> {
        Ok(self)
    }

    /// Obtain a copy of the logger.
    fn logger (&self) -> Logger;

    /// Obtain a handle to the display.
    fn display_handle (&self) -> DisplayHandle;

    /// Obtain a handle to the event loop.
    fn event_handle (&self) -> LoopHandle<'static, State>;

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

    fn dispatch (&mut self, state: &mut State) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn start (&mut self, app: &mut State) {
        unimplemented!{};
    }

    fn tick (&mut self, state: &mut State) {
        unimplemented!{};
    }

}

