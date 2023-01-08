use crate::prelude::*;
use crate::backend::Engine;
use crate::compositor::Compositor;
use crate::controller::Controller;
use crate::workspace::Workspace;

pub struct App {
    pub log:         Logger,
    pub running:     Arc<AtomicBool>,
    pub startup:     Vec<Command>,
    pub start_time:  Instant,
    pub socket_name: Option<String>,
    pub compositor:  Rc<RefCell<Compositor>>,
    pub controller:  Controller,
    pub workspace:   Rc<RefCell<Workspace>>,
    pub handle:      LoopHandle<'static, Self>
}

impl<T: Engine + 'static> App<T> {

    pub fn init (backend: &'static T) -> Result<Self, Box<dyn Error>> {
        let log        = backend.logger();
        let display    = backend.display();
        init_xdg_output_manager(&mut *display.borrow_mut(), backend.logger());
        init_shm_global(&mut *display.borrow_mut(), vec![], backend.logger());
        let running    = Arc::new(AtomicBool::new(true));
        let compositor = Rc::new(RefCell::new(Compositor::init(backend)?));
        let background = backend.load_texture(BACKGROUND)?;
        let workspace  = Rc::new(RefCell::new(Workspace::init(&log, background)?));
        let controller = Controller::init(&log, &running, backend.display(), &compositor, &workspace);
        let handle     = backend.event_handle();
        Ok(Self {
            log:         backend.logger(),
            running,
            startup:     vec![],
            start_time:  Instant::now(),
            socket_name: None,
            compositor,
            controller,
            workspace,
            handle
        })
    }

    pub fn run (mut self, command: Command) -> Self {
        self.startup.push(command);
        self
    }

    /// Send frame events so that client start drawing their next frame
    pub fn send_elapsed (&self) {
        self.compositor.borrow().send_frames(self.start_time.elapsed().as_millis() as u32);
    }

    pub fn running (&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop (&self) {
        self.running.store(false, Ordering::SeqCst)
    }

    pub fn refresh (&self) {
        self.compositor.borrow_mut().refresh();
    }

    pub fn x11_start (&self) {
        self.compositor.borrow_mut().x11_start()
    }

    pub fn x11_ready (
        &mut self,
        conn:   UnixStream,
        client: Client,
        handle: &LoopHandle<'static, Self>
    ) -> Result<(), Box<dyn Error>> {
        info!(self.log, "XWayland ready, launching startup processes...");
        self.compositor.borrow_mut().x11_ready(conn, client, handle)?;
        for command in self.startup.iter_mut() {
            info!(self.log, "Launching {command:?}");
            command.spawn().unwrap();
        }
        Ok(())
    }

}
