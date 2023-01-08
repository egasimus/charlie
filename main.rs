#![feature(int_roundings)]

mod prelude;
//mod backend;
//mod app;
//mod compositor;
//mod controller;
//mod workspace;

use crate::prelude::*;
//use crate::app::App;
//use crate::backend::{Engine, Winit, Udev};

fn main () -> Result<(), Box<dyn Error>> {
    let fuse = slog_async::Async::default(slog_term::term_full().fuse()).fuse();
    let logger = slog::Logger::root(fuse, o!());
    let _guard = slog_scope::set_global_logger(logger.clone());
    slog_stdlog::init().expect("Could not setup log backend");
    info!(&logger, "logger initialized");
    let engine = Winit::new(logger.clone())?.init()?;
    App::init(logger.clone(), engine, State::default())?.start()
}

struct App<E: Engine> {
    logger: Logger,
    engine: E,
    state:  State,
}

impl<E: Engine> App<E> {
    fn init (logger: Logger, engine: E, state: State) -> Result<Self, Box<dyn Error>> {
        // Init log
        Ok(Self { logger, engine, state })
    }
    fn start (&mut self) -> Result<(), Box<dyn Error>> {
        while self.engine.is_running() {
            if self.engine.dispatch(&mut self.state).is_err() {
                self.engine.stop();
                break
            }
            self.state.render(&mut self.engine);
            self.engine.tick(&self.state)
        }
        Ok(())
    }
}

trait Engine: Sized {
    fn init (self) -> Result<Self, Box<dyn Error>> {
        Ok(self)
    }
    fn add_screen (&mut self) -> Result<(), Box<dyn Error>>;
    fn running (&self) -> &Arc<AtomicBool>;
    fn is_running (&self) -> bool {
        self.running().load(Ordering::SeqCst)
    }
    fn stop (&self) {
        self.running().store(false, Ordering::SeqCst)
    }
    fn dispatch (&mut self, state: &mut State) -> Result<(), Box<dyn Error>>;
    fn render_window (&mut self, screen: &Screen, window: &Window) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }
    fn render_pointer (&mut self, screen: &Screen, pointer: &Point<f64, Logical>) -> Result<(), Box<dyn Error>> {
        unimplemented!{};
    }
    fn tick (&self, state: &State) {
        unimplemented!{};
    }
}

use smithay::backend::winit::{self, Error as WinitError, WinitGraphicsBackend, WinitInputBackend};

struct Winit {
    logger:   Logger,
    running:  Arc<AtomicBool>,
    events:   EventLoop<'static, State>,
    screens:  Vec<WinitScreen>
}

struct WinitScreen {
    logger:   Logger,
    running:  Arc<AtomicBool>,
    display:  Rc<RefCell<Display>>,
    graphics: Rc<RefCell<WinitGraphicsBackend>>,
    input:    WinitInputBackend
}

impl WinitScreen {
    fn init (logger: &Logger, running: &Arc<AtomicBool>) -> Result<Self, WinitError> {
        let (graphics, input) = winit::init(logger.clone())?;
        Ok(Self {
            logger:   logger.clone(),
            running:  running.clone(),
            display:  Rc::new(RefCell::new(Display::new())),
            graphics: Rc::new(RefCell::new(graphics)),
            input,
        })
    }
    /// FIXME Describe what this does
    fn init_display_dispatch (&self, events: &EventLoop<'static, State>) -> Result<(), Box<dyn Error>> {
        let fd      = self.display.borrow().get_poll_fd();
        let source  = Generic::from_fd(fd, Interest::READ, CalloopMode::Level);
        let display = self.display.clone();
        let running = self.running.clone();
        let logger  = self.logger.clone();
        events.handle().insert_source(source, move |_, _, state: &mut State| {
            let duration = std::time::Duration::from_millis(0);
            if let Err(e) = display.borrow_mut().dispatch(duration, state) {
                error!(logger, "I/O error on the Wayland display: {}", e);
                running.store(false, Ordering::SeqCst);
                Err(e)
            } else {
                Ok(PostAction::Continue)
            }
        })?;
        Ok(())
    }
    fn init_dmabuf (&mut self) -> Result<(), Box<dyn Error>> {
        let display = self.display.clone();
        self.graphics.borrow_mut().renderer().bind_wl_display(&display.clone().borrow())?;
        let graphics = self.graphics.clone();
        init_dmabuf_global(
            &mut *display.borrow_mut(),
            self.graphics.clone().borrow_mut().renderer()
                .dmabuf_formats().cloned().collect::<Vec<_>>(),
            move |buffer, _| graphics.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
            self.logger.clone()
        );
        Ok(())
    }
}

impl Winit {
    fn new (logger: Logger) -> Result<Self, WinitError> {
        Ok(Self {
            logger,
            running:  Arc::new(AtomicBool::new(true)),
            events:   EventLoop::try_new().expect("Failed to create event loop"),
            screens:  vec![],
        })
    }
}

impl Engine for Winit {
    fn add_screen (&mut self) -> Result<(), Box<dyn Error>> {
        let screen = WinitScreen::init(&self.logger, &self.running)
            .map_err(Into::<Box<dyn Error>>::into)?;
        self.screens.push(screen);
        Ok(())
    }
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
    fn dispatch (&mut self, state: &mut State) -> Result<(), Box<dyn Error>> {
        for screen in self.screens.iter_mut() {
            screen.input
                .dispatch_new_events(|event| state.on_input(event))
                .map_err(Into::<Box<dyn Error>>::into)?;
        }
        Ok(())
    }
    fn tick (&self, state: &State) {
        unimplemented!();
    }
}

struct Udev {
    logger:  Logger,
    running: Arc<AtomicBool>,
    display: Rc<RefCell<Display>>,
    events:  EventLoop<'static, State>,
}

impl Udev {
    fn new (logger: Logger) -> Self {
        Self {
            logger,
            running: Arc::new(AtomicBool::new(true)),
            display: Rc::new(RefCell::new(Display::new())),
            events:  EventLoop::try_new().expect("Failed to create event loop"),
        }
    }
}

impl Engine for Udev {
    fn add_screen (&mut self) -> Result<(), Box<dyn Error>> {
        unimplemented!();
    }
    fn running (&self) -> &Arc<AtomicBool> {
        &self.running
    }
    fn dispatch (&mut self, state: &mut State) -> Result<(), Box<dyn Error>> {
        self.events
            .dispatch(Some(Duration::from_millis(16)), state)
            .map_err(Into::<Box<dyn Error>>::into)
    }
    fn tick (&self, state: &State) {
        unimplemented!();
    }
}

struct Screen {
    location: Point<f64, Logical>,
    size:     Size<f64, Logical>
}

impl Screen {
    fn contains_rect (&self, window: &Window) -> bool {
        false
    }
    fn contains_point (&self, point: Point<f64, Logical>) -> bool {
        false
    }
}

struct Window {
    location: Point<f64, Logical>,
    size:     Size<f64, Logical>
}

#[derive(Default)]
struct State {
    screens:      Vec<Screen>,
    windows:      Vec<Window>,
    pointer:      Point<f64, Logical>,
    pointer_last: Point<f64, Logical>
}

impl State {

    fn render (&self, engine: &mut impl Engine) {
        for screen in self.screens.iter() {
            for window in self.windows.iter() {
                if screen.contains_rect(window) {
                    engine.render_window(screen, window);
                }
            }
            if screen.contains_point(self.pointer) {
                engine.render_pointer(screen, &self.pointer);
            }
        }
    }

    fn on_input <B: InputBackend> (&mut self, event: InputEvent<B>) {
    }

}
