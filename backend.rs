use crate::prelude::*;
use crate::app::App;
use crate::controller::Controller;

pub trait Backend: Sized {
    type Renderer;
    type Input;
    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        unimplemented!()
    }
    fn post_init (app: &mut App<Self>, event_loop: &EventLoop<'static, App<Self>>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
    fn renderer (&mut self) -> &mut Gles2Renderer {
        unimplemented!()
    }
    fn draw (app: &mut App<Self>) -> Result<(), SwapBuffersError> {
        unimplemented!()
    }
    fn load_texture (&mut self, path: impl AsRef<Path>) -> Result<Gles2Texture, Box<dyn Error>> {
        unimplemented!()
    }
    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
    fn add_output (app: &mut App<Self>, name: impl AsRef<str>) {
        unimplemented!()
    }
}

pub struct Winit {
    pub renderer: <Self as Backend>::Renderer,
    pub input:    <Self as Backend>::Input,
}

impl Backend for Winit {

    type Renderer = WinitGraphicsBackend;
    type Input    = WinitInputBackend;

    fn init (log: &Logger) -> Result<Self, Box<dyn Error>> {
        let (renderer, input) = winit::init(log.clone())?;
        Ok(Self { renderer, input })
    }

    fn post_init (app: &mut App<Self>, event_loop: &EventLoop<'static, App<Self>>) -> Result<(), Box<dyn Error>> {

        let log = app.log.clone();

        let display = app.display.clone();

        event_loop.handle().insert_source(Generic::from_fd(
            app.display.borrow().get_poll_fd(),
            Interest::READ,
            CalloopMode::Level
        ), move |_, _, state: &mut App<Self>| {
            let mut display = display.borrow_mut();
            match display.dispatch(std::time::Duration::from_millis(0), state) {
                Ok(_) => {
                    Ok(PostAction::Continue)
                },
                Err(e) => {
                    error!(log, "I/O error on the Wayland display: {}", e);
                    state.running.store(false, Ordering::SeqCst);
                    Err(e)
                }
            }
        })?;

        app.backend.renderer().bind_wl_display(&app.display.borrow())?;

        init_dmabuf_global(
            &mut *app.display.borrow_mut(),
            app.backend.renderer().dmabuf_formats().cloned().collect::<Vec<_>>(),
            move |buffer, mut state| state.get::<App<Self>>().unwrap()
                .backend.renderer()
                .import_dmabuf(buffer).is_ok(),
            app.log.clone()
        );

        Ok(())

    }

    fn renderer (&mut self) -> &mut Gles2Renderer {
        self.renderer.renderer()
    }

    fn load_texture (&mut self, path: impl AsRef<Path>) -> Result<Gles2Texture, Box<dyn Error>> {
        import_bitmap(
            self.renderer(),
            &image::io::Reader::open(path)?
                .with_guessed_format().unwrap()
                .decode().unwrap()
                .to_rgba8()
        )
            .map_err(Into::<Box<dyn Error>>::into)
    }

    fn dispatch_input (&mut self, controller: &mut Controller<Self>) -> Result<(), Box<dyn Error>> {
        self.input
            .dispatch_new_events(|event| controller.process_input_event(event))
            .map_err(Into::<Box<dyn Error>>::into)
            //.map_err(|e|Box::new(e) as Box<dyn Error>)
    }

    fn draw (app: &mut App<Self>) -> Result<(), SwapBuffersError> {
        let workspace = app.workspace.borrow();
        let result = app.backend.renderer.render(|mut renderer, mut frame| {
            // This is safe to do as with winit we are guaranteed to have exactly one output
            frame.clear([0.8, 0.8, 0.8, 1.0])?;
            app.compositor.borrow().draw(&mut renderer, &mut frame, &workspace)?;
            app.controller.draw(&mut renderer, &mut frame, 1.0)?;
            Ok(())
        })?;
        app.backend.renderer.window().set_cursor_visible(app.controller.cursor_visible.get());
        result
    }

    fn add_output (app: &mut App<Self>, name: impl AsRef<str>) {
        app.compositor.borrow_mut().add_output(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: wl_output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
            },
            OutputMode {
                size:    app.backend.renderer.window_size().physical_size,
                refresh: 60_000
            }
        );
    }

}

pub struct Udev;

impl Backend for Udev {
    type Renderer = ();
    type Input    = ();
}
