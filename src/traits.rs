use crate::prelude::*;

pub trait Widget {

    type RenderData;

    fn prepare (&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn render <'r> (&'r self, context: RenderContext<'r, Self::RenderData>)
        -> Result<(), Box<dyn Error>>;

    fn refresh (&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn update <B: InputBackend> (&mut self, screen_id: usize, event: InputEvent<B>);

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

pub trait Engine: Stoppable + 'static {

    type State;

    /// Obtain a copy of the logger.
    fn logger (&self) -> Logger;

    /// Obtain a reference to the display.
    fn display (&self) -> &Rc<RefCell<Display<Self::State>>>;

    /// Obtain a reference to the event loop.
    fn events (&self) -> &EventLoop<'static, Self::State>;

    /// Obtain a reference to the display.
    fn display_handle (&self) -> DisplayHandle {
        self.display().borrow().handle()
    }

    /// Obtain a pollable file descriptor for the display.
    fn display_fd (&self) -> i32 {
        self.display().borrow_mut().backend().poll_fd().as_raw_fd()
    }

    /// Obtain a callable which dispatches display state to clients.
    fn display_dispatcher (&self) -> Box<dyn Fn(&mut Self::State) -> Result<usize, std::io::Error>> {
        let display = self.display().clone();
        Box::new(move |widget| { display.borrow_mut().dispatch_clients(widget) })
    }

    /// Obtain a handle to the event loop.
    fn event_handle (&self) -> LoopHandle<'static, Self::State> {
        self.events().handle()
    }

    /// Obtain a mutable reference to the renderer.
    fn renderer (&mut self) -> &mut Gles2Renderer;

    /// Add an output to the host and bind it to a compositor screen
    fn output_add (&mut self, name: &str, screen: usize, width: i32, height: i32)
        -> Result<(), Box<dyn Error>>;

    fn output_change (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn output_remove (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_add (&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_change (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn input_remove (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }

    fn start (&mut self, app: &mut Self::State) -> Result<(), Box<dyn Error>> {
        self.prepare(app)?;
        self.start_running();
        while self.is_running() {
            if let Err(err) = self.tick(app) {
                crit!(self.logger(), "{err}");
                self.stop_running();
                break
            }
        }
        Ok(())
    }

    fn prepare (&mut self, app: &mut Self::State) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn tick (&mut self, state: &mut Self::State) -> Result<(), Box<dyn Error>>;

}

// TODO:
// fn render (self, renderer, area) -> RenderResult
// struct RenderResult { used, damages }
