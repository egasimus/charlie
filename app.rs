use crate::prelude::*;
use crate::compositor::{Compositor, WindowMap};
use crate::controller::Controller;
use crate::workspace::Workspace;

pub struct App {
    pub log:         Logger,
    pub running:     Arc<AtomicBool>,
    pub start_time:  Instant,
    pub event_loop:  Rc<RefCell<EventLoop<'static, Self>>>,
    pub socket_name: Option<String>,
    pub display:     Rc<RefCell<Display>>,
    pub renderer:    Rc<RefCell<WinitGraphicsBackend>>,
    pub input:       Rc<RefCell<WinitInputBackend>>,
    pub windows:     Rc<RefCell<WindowMap>>,
    pub compositor:  Compositor,
    pub controller:  Controller,
    pub workspace:   Rc<RefCell<Workspace>>,
}

impl App {

    pub fn init (log: Logger) -> Result<Self, Box<dyn Error>> {
        let display    = Rc::new(RefCell::new(Display::new()));
        let event_loop = EventLoop::try_new()?;
        let (renderer, input) = App::init_io(&log)?;
        init_xdg_output_manager(&mut *display.borrow_mut(), log.clone());
        init_shm_global(&mut *display.borrow_mut(), vec![], log.clone());
        let running    = Arc::new(AtomicBool::new(true));
        let windows    = Rc::new(RefCell::new(WindowMap::init(&log)));
        let compositor = Compositor::init(&log, &display, &windows, &event_loop)?;
        let workspace  = Rc::new(RefCell::new(Workspace::init(&log, &renderer)?));
        let controller = Controller::init(&log, &running, &display, &compositor, &workspace);
        let app = Self {
            log,
            running,
            start_time: Instant::now(),
            event_loop: Rc::new(RefCell::new(event_loop)),
            socket_name: None,
            display,
            renderer,
            input,
            windows,
            compositor,
            controller,
            workspace,
        };
        app.init_loop(app.event_loop.borrow().handle());
        if app.renderer.borrow_mut().renderer().bind_wl_display(&app.display.borrow()).is_ok() {
            app.init_dmabuf();
        };
        Ok(app)
    }
    pub fn init_io (log: &Logger) -> Result<(
        Rc<RefCell<WinitGraphicsBackend>>,
        Rc<RefCell<WinitInputBackend>>
    ), winit::Error> {
        match winit::init(log.clone()) {
            Ok((renderer, input)) => {
                Ok((Rc::new(RefCell::new(renderer)), Rc::new(RefCell::new(input))))
            },
            Err(err) => {
                slog::crit!(log, "Failed to initialize Winit backend: {}", err);
                Err(err)
            }
        }
    }
    fn get_dmabuf_formats (&self) -> Vec<Format> {
        self.renderer.borrow_mut().renderer().dmabuf_formats().cloned()
            .collect::<Vec<_>>()
    }
    fn init_dmabuf (&self) {
        let renderer = self.renderer.clone();
        init_dmabuf_global(
            &mut *self.display.borrow_mut(),
            self.get_dmabuf_formats(),
            move |buffer, _| renderer.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
            self.log.clone()
        );
    }
    fn init_loop (&self, handle: LoopHandle<'static, Self>) {
        let log          = self.log.clone();
        let display      = self.display.clone();
        let same_display = self.display.clone();
        handle.insert_source( // init the wayland connection
            Generic::from_fd(display.borrow().get_poll_fd(), Interest::READ, CalloopMode::Level),
            move |_, _, state: &mut App| {
                let mut display = same_display.borrow_mut();
                match display.dispatch(std::time::Duration::from_millis(0), state) {
                    Ok(_) => Ok(PostAction::Continue),
                    Err(e) => {
                        error!(log, "I/O error on the Wayland display: {}", e);
                        state.running.store(false, Ordering::SeqCst);
                        Err(e)
                    }
                }
            },
        ).expect("Failed to init the wayland event source.");
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
        self.compositor.add_output(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode {
                size:    self.renderer.borrow().window_size().physical_size,
                refresh: 60_000
            }
        );
        self
    }
    pub fn run (&mut self, command: &mut Command) -> &mut Self {
        command.spawn().unwrap();
        self
    }
    pub fn start (&mut self) {
        //self.compositor.x11_start();
        self.start_time = Instant::now();
        info!(self.log, "Initialization completed, starting the main loop.");
        while self.running() {
            if !self.dispatch_input() { self.stop(); break; }
            self.draw();
            self.flush();
            if !self.dispatch_event_loop() { self.stop(); break; }
            self.flush();
            self.refresh();
        }
        self.clear();
    }
    pub fn dispatch_input (&mut self) -> bool {
        self.input.borrow_mut()
            .dispatch_new_events(|event| self.controller.process_input_event(event))
            .is_ok()
    }
    pub fn dispatch_event_loop (&mut self) -> bool {
        self.event_loop.clone().borrow_mut()
            .dispatch(Some(Duration::from_millis(16)), self)
            .is_ok()
    }
    pub fn draw (&mut self) {
        let result = {
            let workspace = self.workspace.borrow();
            self.renderer.borrow_mut().render(|mut renderer, mut frame| {
                // This is safe to do as with winit we are guaranteed to have exactly one output
                frame.clear([0.8, 0.8, 0.8, 1.0])?;
                self.compositor.draw(&mut renderer, &mut frame, &workspace)?;
                self.controller.draw(&mut renderer, &mut frame, 1.0)?;
                Ok(())
            }).map_err(Into::<SwapBuffersError>::into).and_then(|x| x)
        };
        self.renderer.borrow().window()
            .set_cursor_visible(self.controller.cursor_visible.get());
        if let Err(SwapBuffersError::ContextLost(err)) = result {
            error!(self.log, "Critical Rendering Error: {}", err);
            self.stop();
        }
        self.send_frames(self.start_time.elapsed().as_millis() as u32);
    }
    /// Send frame events so that client start drawing their next frame
    pub fn send_frames (&self, frames: u32) {
        self.compositor.window_map.borrow().send_frames(frames);
    }
    pub fn clear (&self) {
        self.compositor.window_map.borrow_mut().clear()
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
        self.compositor.window_map.borrow_mut().refresh();
        self.compositor.output_map.borrow_mut().refresh();
    }
}
