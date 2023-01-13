mod prelude;
pub mod desktop;
mod input;
mod xwayland;

use self::prelude::*;
use self::handle::DelegatedState;
use self::desktop::Desktop;
use self::input::{Input, Pointer, Keyboard};
use self::xwayland::{XWaylandConnection, XWaylandState};

use std::ffi::OsString;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::reexports::calloop::{PostAction, Interest, Mode, generic::Generic};
use smithay::reexports::wayland_server::backend::{ClientId, ClientData, DisconnectReason};

/// An instance of the application.
pub struct App<E: Engine, W: Widget<D>, D> {
    logger: Logger,
    events: EventLoop<'static, Self>,
    handle: DisplayHandle,
    engine: E,
    state:  W,
}

impl<E: Engine, W: Widget<D>, D> App<E, W, D> {
    /// Hook up a new Display instance between the Wayland socket
    /// and the main event loop, returning a clonable DisplayHandle
    /// and a boxed closure that calls Display::flush_clients.
    ///
    /// Each `Display` instance performs global Wayland dispatch
    /// from an event loop with state type S, to a different type U.
    ///
    /// Note that this performs type erasure, and using the handle
    /// from Display<U> for an invalid type V will cause a runtime error.
    /// On the other side of this mechanism is the Any type id downcasting
    /// of the Dispatch family of traits.
    fn register_display <U> (
        logger: &Logger,
        socket: ListeningSocketSource,
        events: EventLoop<'static, Self>,
        filter: fn(&mut Self)->&mut U,
    ) -> Result<DisplayContext, Box<dyn Error>> {
        let logger = logger.clone();
        let display = Display::<U>::new()?;
        let mut handle = display.handle();
        // Listen for clients
        events.handle().insert_source(socket, move |client, _, state| {
            handle.insert_client(client, Arc::new(ClientState)).unwrap();
        });
        // Listen for events
        let fd = display.backend().poll_fd().as_raw_fd();
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        events.handle().insert_source(source, move |_, _, mut state| {
            display.dispatch_clients(filter(state))?;
            Ok(PostAction::Continue)
        });
        Ok((display.handle(), Box::new(|_|display.flush_clients())))
    }
}

impl<E: Engine> App<E, AppState, RenderData> {

    /// Create a new application instance.
    pub fn new (logger: &Logger) -> Result<Self, Box<dyn Error>> {
        // Create the event loop and listening socket.
        let events = EventLoop::<'static, Self>::try_new()?;
        let socket = ListeningSocketSource::new_auto(logger.clone()).unwrap();
        // Export the socket name as the WAYLAND_DISPLAY environment variable.
        ::std::env::set_var("WAYLAND_DISPLAY", &socket.socket_name().to_os_string());

        // In app context, the full contents of App<E, S> are accessible.
        let (app_display, flush_app) = Self::register_display::<Self>(
            &logger, socket, events, |x|&mut x)?;
        // In engine context, only the contents of E are accessible.
        let (engine_display, flush_engine) = Self::register_display::<E>(
            &logger, socket, events, |x|&mut x.engine)?;
        // In state context only the contents of S are accessible;
        let (state_display, flush_state) = Self::register_display::<AppState>(
            &logger, socket, events, |x|&mut x.state)?;

        // Create the engine. Global dispatch in engine context allows
        // the dma buffer state to be contained in E, and buffer handling
        // to be implemented for E. The FlushClients closure must be called
        // by the Engine at the end of every `Engine::tick()` to keep the Wayland messages going.
        let engine = Engine::new(logger, &engine_display, Box::new(|state: &mut Self| {
            //events.dispatch(Some(Duration::from_millis(1)), state)?;
            //state.state.refresh().unwrap();
            state.flush().unwrap();
            flush_app()?;
            flush_engine()?;
            flush_state()?;
            Ok(())
        }))?;

        self::xwayland::init_xwayland::<Self>(
            logger,
            events.handle(),
            &state_display,
            &|app|app.state.ready()
        )?;

        Ok(Self {
            logger: logger.clone(),
            events,
            handle: app_display,
            engine,
            state: AppState::new(&mut engine, &state_display, &events.handle())?,
        })
    }

    /// Add a command to run on startup.
    /// TODO: Integrate a `systemd --user` session
    pub fn startup (&mut self, command: &str, args: &[&str]) -> &mut Self {
        self.state.startup.push((
            String::from(command),
            args.iter().map(|arg|String::from(*arg)).collect()
        ));
        self
    }

    /// Add a viewport into the workspace.
    pub fn output (&mut self, name: &str, w: i32, h: i32, x: f64, y: f64) -> Result<&mut Self, Box<dyn Error>> {
        let screen = self.state.desktop.screen_add(ScreenState::new((x, y), (w as f64, h as f64)));
        self.engine.output_add(name, screen, w, h)?;
        Ok(self)
    }

    /// Add a control seat over the workspace.
    pub fn input (&mut self, name: &str, cursor: &str) -> Result<&mut Self, Box<dyn Error>> {
        self.state.input.seat_add(name, import_bitmap(self.engine.renderer(), cursor)?)?;
        Ok(self)
    }

    pub fn start (&mut self) -> Result<(), Box<dyn Error>> {
        self.engine.start(&mut self.state)
    }

}

/// Contains the compositor state.
pub struct AppState {
    logger:        Logger,
    /// Commands to run after successful initialization
    startup:       Vec<(String, Vec<String>)>,
    /// The collection of windows and their layouts
    pub desktop:   Desktop,
    /// The collection of input devices
    pub input:     Input
}

impl AppState {

    pub fn new <E: Engine> (
        engine: &mut E,
        handle: &DisplayHandle,
        events: &LoopHandle<App<E, AppState, RenderData>>
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            logger:  engine.logger(),
            desktop: Desktop::new(engine, handle),
            input:   Input::new(engine, handle)?,
            startup: vec![],
        })
    }

    /// When the app is ready to run, this spawns the startup processes.
    pub fn ready (&self) -> Result<(), Box<dyn Error>> {
        debug!(self.logger, "DISPLAY={:?}", ::std::env::var("DISPLAY"));
        debug!(self.logger, "WAYLAND_DISPLAY={:?}", ::std::env::var("WAYLAND_DISPLAY"));
        debug!(self.logger, "{:?}", self.startup);
        for (cmd, args) in self.startup.iter() {
            debug!(self.logger, "Spawning {cmd} {args:?}");
            std::process::Command::new(cmd).args(args).spawn()?;
        }
        Ok(())
    }

}

struct ClientState;

impl ClientData for ClientState {
    fn initialized (&self, _client_id: ClientId) {
    }
    fn disconnected (&self, _client_id: ClientId, _reason: DisconnectReason) {
    }
}

type ScreenId = usize;

type RenderData = (ScreenId, Size<i32, Physical>);

impl Widget<RenderData> for AppState {

    fn render <'r> (
        &'r self, context: RenderContext<'r, RenderData>
    ) -> Result<(), Box<dyn Error>> {
        let AppState { desktop, input, .. } = &self;
        let RenderContext { renderer, output, data: (screen_id, screen_size) } = context;
        let (size, transform, scale) = (
            output.current_mode().unwrap().size,
            output.current_transform(),
            output.current_scale()
        );
        desktop.import(renderer)?;
        let mut frame = renderer.render(size, Transform::Flipped180)?;
        frame.clear([0.2,0.3,0.4,1.0], &[Rectangle::from_loc_and_size((0, 0), size)])?;
        desktop.render(&mut frame, screen_id, size)?;
        for pointer in input.pointers.iter() {
            pointer.render(&mut frame, size, &desktop.screens[screen_id])?;
        }
        frame.finish()?;
        desktop.tick(output, delegated.clock.now());
        Ok(())
    }

    fn update <B: InputBackend> (&mut self, screen_id: ScreenId, event: InputEvent<B>) {
        //let state    = &mut self.state;
        //let pointer  = &self.state.pointers[0];
        //let keyboard = &self.state.keyboards[0];
        match event {
            InputEvent::PointerMotion { event, .. }
                => Pointer::on_move_relative::<B>(self, 0, event, screen_id),
            InputEvent::PointerMotionAbsolute { event, .. }
                => Pointer::on_move_absolute::<B>(self, 0, event, screen_id),
            InputEvent::PointerButton { event, .. }
                => Pointer::on_button::<B>(self, 0, event, screen_id),
            InputEvent::PointerAxis { event, .. }
                => Pointer::on_axis::<B>(self, 0, event, screen_id),
            InputEvent::Keyboard { event, .. }
                => Keyboard::on_key::<B>(self, 0, event, screen_id),
            _ => {}
        }
    }

}
