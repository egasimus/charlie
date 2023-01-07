use crate::prelude::*;
use crate::app::{App, Backend};

pub struct Winit {
    render_backend: Rc<RefCell<WinitGraphicsBackend>>,
    input_backend:  WinitInputBackend,
}

impl Backend for Winit {

    type Render = WinitGraphicsBackend;

    type Input  = WinitInputBackend;

    fn init (
        log:     &Logger,
        display: &Rc<RefCell<Display>>,
        events:  &EventLoop<'static, App<Self>>
    ) -> Result<Self, Box<dyn Error>> where Self: Sized {
        let (render_backend, input_backend) = winit::init(log.clone())?;
        let render_backend = Rc::new(RefCell::new(render_backend));
        if render_backend.borrow_mut().renderer().bind_wl_display(&display.borrow()).is_ok() {
            info!(log, "EGL hardware-acceleration enabled");
            let dmabuf_formats = render_backend.borrow_mut()
                .renderer().dmabuf_formats().cloned().collect::<Vec<_>>();
            let renderer = render_backend.clone();
            init_dmabuf_global(
                &mut *display.borrow_mut(),
                dmabuf_formats,
                move |buffer, _| renderer.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
                log.clone(),
            );
        };
        let size = render_backend.borrow().window_size().physical_size;
        let backend = Self { render_backend, input_backend };
        let same_display = display.clone();
        // init the wayland connection
        events.handle().insert_source(
            Generic::from_fd(display.borrow().get_poll_fd(), Interest::READ, CalloopMode::Level),
            move |_, _, state: &mut App<Self>| {
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
        Ok(backend)
    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        self.render_backend.borrow_mut().renderer()
    }

    fn input (&self) -> Self::Input {
        self.input_backend
    }

    fn input_dispatched (&self, app: &mut App<Self>) -> bool {
        self.input()
            .dispatch_new_events(|event| app.controller.process_input_event(event))
            .is_ok()
    }

    fn draw (&self, app: &App<Self>, elapsed: u32) {
        let workspace = app.workspace.borrow();
        // This is safe to do as with winit we are guaranteed to have exactly one output
        let result = self.render_backend.borrow_mut().render(|mut renderer, mut frame| {
            frame.clear([0.8, 0.8, 0.8, 1.0])?;
            let (_, output_scale) = app.compositor.draw(&mut renderer, &mut frame, &workspace)?;
            app.controller.draw(&mut renderer, &mut frame, output_scale)?;
            Ok(())
        }).map_err(Into::<SwapBuffersError>::into).and_then(|x| x);
        //app.backend.renderer().window().set_cursor_visible(app.controller.cursor_visible.get());
        if let Err(SwapBuffersError::ContextLost(err)) = result {
            error!(app.log, "Critical Rendering Error: {}", err);
            app.stop();
        };
        app.send_frames(elapsed);
    }

}

impl App<Winit> {
    pub fn add_output (&self, name: &str) -> &Self {
        let size = self.backend.render_backend.borrow().window_size().physical_size;
        self.compositor.output_map.borrow_mut().add(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode { size, refresh: 60_000 }
        );
        self
    }
}
