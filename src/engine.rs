use crate::prelude::*;

pub(crate) mod udev;
pub(crate) mod winit;

pub(crate) trait Stoppable {

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

pub(crate) trait Engine: Stoppable + Sized {

    fn init (self) -> Result<Self, Box<dyn Error>> {
        Ok(self)
    }

    fn output_add (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

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

    fn render_window (&mut self, output: &Screen, window: &Window) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn render_pointer (&mut self, output: &Screen, pointer: &Point<f64, Logical>) -> Result<(), Box<dyn Error>> {
        unimplemented!{};
    }

    fn start (&mut self, app: &mut State) {
        unimplemented!{};
    }

    fn tick (&mut self, state: &mut State) {
        unimplemented!{};
    }

}

