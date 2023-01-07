use crate::prelude::*;
use crate::backend::Backend;
use crate::compositor::Compositor;
use crate::controller::Controller;
use crate::workspace::Workspace;

pub struct App<T: Backend> {
    pub log:         Logger,
    pub backend:     T,
    pub running:     Arc<AtomicBool>,
    pub startup:     Vec<Command>,
    pub start_time:  Instant,
    pub socket_name: Option<String>,
    pub display:     Rc<RefCell<Display>>,
    pub compositor:  Rc<RefCell<Compositor<T>>>,
    pub controller:  Controller<T>,
    pub workspace:   Rc<RefCell<Workspace>>,
}

impl<T: Backend + 'static> App<T> {

    pub fn init (
        log:         &Logger,
        event_loop:  &EventLoop<'static, Self>,
        mut backend: T,
    ) -> Result<Self, Box<dyn Error>> {
        let display    = Rc::new(RefCell::new(Display::new()));
        init_xdg_output_manager(&mut *display.borrow_mut(), log.clone());
        init_shm_global(&mut *display.borrow_mut(), vec![], log.clone());
        let running    = Arc::new(AtomicBool::new(true));
        let compositor = Rc::new(RefCell::new(Compositor::init(&log, &display, &event_loop)?));
        let background = backend.load_texture(BACKGROUND)?;
        let workspace  = Rc::new(RefCell::new(Workspace::init(&log, background)?));
        let controller = Controller::init(&log, &running, &display, &compositor, &workspace);
        let mut app = Self {
            log:         log.clone(),
            backend,
            running,
            startup:     vec![],
            start_time:  Instant::now(),
            socket_name: None,
            display,
            compositor,
            controller,
            workspace,
        };
        T::post_init(&mut app, event_loop);
        Ok(app)
    }

    pub fn socket (&mut self, enable: bool) -> &mut Self {
        self.socket_name = if enable {
            let socket_name =
                self.display.borrow_mut().add_socket_auto().unwrap().into_string().unwrap();
            info!(self.log, "Listening on wayland socket"; "name" => socket_name.clone());
            ::std::env::set_var("WAYLAND_DISPLAY", &socket_name);
            Some(socket_name)
        } else {
            None
        };
        self
    }

    pub fn add_output (&mut self, name: &str) -> &mut Self {
        T::add_output(self, name);
        self
    }

    pub fn run (&mut self, command: Command) -> &mut Self {
        self.startup.push(command);
        self
    }

    pub fn start (&mut self, event_loop: &mut EventLoop<'static, Self>) -> Result<(), Box<dyn Error>> {
        self.compositor.borrow_mut().x11_start();
        self.start_time = Instant::now();
        info!(self.log, "Initialization completed, starting the main loop.");
        while self.running() {
            self.backend.dispatch_input(&mut self.controller)?;
            self.draw();
            self.flush();
            self.dispatch_event_loop(event_loop)?;
            self.flush();
            self.refresh();
        }
        self.clear();
        Ok(())
    }

    pub fn dispatch_event_loop (&mut self, event_loop: &mut EventLoop<'static, Self>) -> Result<(), std::io::Error> {
        event_loop.dispatch(Some(Duration::from_millis(16)), self)
    }

    pub fn draw (&mut self) {
        if let Err(SwapBuffersError::ContextLost(err)) = T::draw(self) {
            error!(self.log, "Critical Rendering Error: {}", err);
            self.stop();
        }
        self.send_frames(self.start_time.elapsed().as_millis() as u32);
    }

    /// Send frame events so that client start drawing their next frame
    pub fn send_frames (&self, frames: u32) {
        self.compositor.borrow().window_map.borrow().send_frames(frames);
    }

    pub fn clear (&self) {
        self.compositor.borrow().window_map.borrow_mut().clear()
    }

    pub fn running (&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop (&self) {
        self.running.store(false, Ordering::SeqCst)
    }

    pub fn flush (&mut self) {
        self.display.clone().borrow_mut().flush_clients(self);
    }

    pub fn refresh (&self) {
        self.compositor.borrow_mut().refresh();
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
