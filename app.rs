use crate::prelude::*;
use crate::surface::draw_surface_tree;
use crate::compositor::Compositor;
use crate::controller::Controller;

pub struct App {
    pub log:              Logger,
    pub socket_name:      Option<String>,
    pub running:          Arc<AtomicBool>,
    pub renderer:         Rc<RefCell<WinitGraphicsBackend>>,
    pub dnd_icon:         Arc<Mutex<Option<WlSurface>>>,
    pub compositor:       Rc<Compositor>,
    pub controller:       Controller,
    pub background:       Gles2Texture,
}

impl App {

    pub fn init (
        log:        Logger,
        display:    &Rc<RefCell<Display>>,
        renderer:   &Rc<RefCell<WinitGraphicsBackend>>,
        event_loop: &EventLoop<'static, Self>
    ) -> Result<Self, Box<dyn Error>> {
        init_xdg_output_manager(&mut *display.borrow_mut(), log.clone());
        init_shm_global(&mut *display.borrow_mut(), vec![], log.clone());
        Self::init_loop(&log, &display, event_loop.handle());
        let socket_name = Self::init_socket(&log, &display, true);
        let dnd_icon = Arc::new(Mutex::new(None));
        Self::init_data_device(&log, &display, &dnd_icon);
        let running    = Arc::new(AtomicBool::new(true));
        let compositor = Compositor::init(&log, display);
        let controller = Controller::init(&log, display, running.clone(), compositor.clone());
        let background = Self::import_bitmap(
            renderer.borrow_mut().renderer(),
            &image::io::Reader::open(BACKGROUND)?.with_guessed_format().unwrap()
                .decode().unwrap().to_rgba8()
        )?;
        Ok(Self {
            log,
            dnd_icon,
            renderer: renderer.clone(),
            running,
            socket_name,
            compositor,
            controller,
            background
        })
    }

    pub fn init_log () -> (slog::Logger, GlobalLoggerGuard) {
        let fuse = slog_async::Async::default(slog_term::term_full().fuse()).fuse();
        let log = slog::Logger::root(fuse, o!());
        let guard = slog_scope::set_global_logger(log.clone());
        slog_stdlog::init().expect("Could not setup log backend");
        (log, guard)
    }

    pub fn init_io (log: &Logger, display: &Rc<RefCell<Display>>)
        -> Result<(Rc<RefCell<WinitGraphicsBackend>>, WinitInputBackend), winit::Error>
    {
        match winit::init(log.clone()) {
            Ok((renderer, input)) => {
                let renderer = Rc::new(RefCell::new(renderer));
                if renderer.borrow_mut().renderer().bind_wl_display(&display.borrow()).is_ok() {
                    info!(log, "EGL hardware-acceleration enabled");
                    let dmabuf_formats = renderer.borrow_mut().renderer().dmabuf_formats().cloned()
                        .collect::<Vec<_>>();
                    let renderer = renderer.clone();
                    init_dmabuf_global(
                        &mut *display.borrow_mut(),
                        dmabuf_formats,
                        move |buffer, _| renderer.borrow_mut().renderer().import_dmabuf(buffer).is_ok(),
                        log.clone()
                    );
                };
                Ok((renderer, input))
            },
            Err(err) => {
                slog::crit!(log, "Failed to initialize Winit backend: {}", err);
                Err(err)
            }
        }
    }

    fn init_loop (
        log: &Logger,
        display: &Rc<RefCell<Display>>,
        event_loop: LoopHandle<'static, Self>,
    ) {
        let log = log.clone();
        let display = display.clone();
        let same_display = display.clone();
        event_loop.insert_source( // init the wayland connection
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

    fn init_socket (
        log: &Logger, display: &Rc<RefCell<Display>>, listen_on_socket: bool
    ) -> Option<String> {
        if listen_on_socket {
            let socket_name =
                display.borrow_mut().add_socket_auto().unwrap().into_string().unwrap();
            info!(log, "Listening on wayland socket"; "name" => socket_name.clone());
            ::std::env::set_var("WAYLAND_DISPLAY", &socket_name);
            Some(socket_name)
        } else {
            None
        }
    }

    pub fn init_data_device (
        log: &Logger, display: &Rc<RefCell<Display>>, dnd_icon: &Arc<Mutex<Option<WlSurface>>>
    ) {
        let dnd_icon = dnd_icon.clone();
        init_data_device(
            &mut display.borrow_mut(),
            move |event| match event {
                DataDeviceEvent::DnDStarted { icon, .. } => {*dnd_icon.lock().unwrap() = icon;}
                DataDeviceEvent::DnDDropped => {*dnd_icon.lock().unwrap() = None;}
                _ => {}
            },
            default_action_chooser,
            log.clone(),
        );
    }

    pub fn add_output (&self, name: &str) -> &Self {
        let size = self.renderer.borrow().window_size().physical_size;
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

    pub fn run (
        &mut self,
        display: &Rc<RefCell<Display>>,
        mut input: WinitInputBackend,
        mut event_loop: EventLoop<'static, Self>,
    ) {
        let start_time = std::time::Instant::now();
        let mut cursor_visible = true;
        info!(self.log, "Initialization completed, starting the main loop.");
        while self.running() {
            let handle = |event| self.controller.process_input_event(event);
            if input.dispatch_new_events(handle).is_err() {
                self.running.store(false, Ordering::SeqCst);
                break;
            }
            self.draw(&mut cursor_visible);
            self.send_frames(start_time.elapsed().as_millis() as u32);
            self.flush(display);
            if event_loop.dispatch(Some(Duration::from_millis(16)), self).is_err() {
                self.stop();
            } else {
                self.flush(display);
                self.refresh();
            }
        }
        self.clear();
    }

    pub fn draw (&self, cursor_visible: &mut bool) {
        let (output_geometry, output_scale) = self.compositor.output_map.borrow()
            .find_by_name(OUTPUT_NAME)
            .map(|output| (output.geometry(), output.scale()))
            .unwrap();
        // This is safe to do as with winit we are guaranteed to have exactly one output
        let result = self.renderer.borrow_mut().render(|renderer, frame| {
            frame.clear([0.8, 0.8, 0.9, 1.0])?;
            frame.render_texture_at(
                &self.background,
                self.controller.background_offset,
                1,
                1.0,
                Transform::Normal,
                1.0
            )?;
            let windows = self.compositor.window_map.borrow();
            windows.draw_windows(&self.log, renderer, frame, output_geometry, output_scale)?;
            let (x, y) = self.controller.pointer_location.into();
            let location: Point<i32, Logical> = (x as i32, y as i32).into();
            self.draw_dnd_icon(renderer, frame, output_scale, location)?;
            self.draw_cursor(renderer, frame, output_scale, cursor_visible, location)?;
            Ok(())
        }).map_err(Into::<SwapBuffersError>::into).and_then(|x| x);
        self.renderer.borrow().window().set_cursor_visible(*cursor_visible);
        if let Err(SwapBuffersError::ContextLost(err)) = result {
            error!(self.log, "Critical Rendering Error: {}", err);
            self.stop();
        }
    }

    pub fn draw_dnd_icon<R, F, E, T>(
        &self,
        renderer:     &mut R,
        frame:        &mut F,
        output_scale: f32,
        location:     Point<i32, Logical>,
    )
        -> Result<(), SwapBuffersError>
    where
        T: Texture + 'static,
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: Error + Into<SwapBuffersError>
    {
        let guard = self.dnd_icon.lock().unwrap();
        Ok(if let Some(ref surface) = *guard && surface.as_ref().is_alive() {
            if get_role(surface) != Some("dnd_icon") {
                warn!(self.log, "Trying to display as a dnd icon a surface that does not have the DndIcon role.");
            }
            draw_surface_tree(&self.log, renderer, frame, surface, location, output_scale)?
        } else {
            ()
        })
    }

    pub fn draw_cursor<R, F, E, T>(
        &self,
        renderer:       &mut R,
        frame:          &mut F,
        output_scale:   f32,
        cursor_visible: &mut bool,
        location:       Point<i32, Logical>,
    )
        -> Result<(), SwapBuffersError>
    where
        T: Texture + 'static,
        R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
        F: Frame<Error = E, TextureId = T>,
        E: Error + Into<SwapBuffersError>,
    {
        let mut guard = self.controller.cursor_status.lock().unwrap();
        let mut reset = false; // reset the cursor if the surface is no longer alive
        if let CursorImageStatus::Image(ref surface) = *guard {
            reset = !surface.as_ref().is_alive();
        }
        if reset {
            *guard = CursorImageStatus::Default;
        }
        Ok(if let CursorImageStatus::Image(ref surface) = *guard {
            *cursor_visible = false;
            let states = with_states(surface, |states|
                Some(states.data_map.get::<Mutex<CursorImageAttributes>>()
                    .unwrap().lock().unwrap().hotspot));
            let delta = if let Some(h) = states.unwrap_or(None) { h } else {
                warn!(self.log, "Trying to display as a cursor a surface that does not have the CursorImage role.");
                (0, 0).into()
            };
            draw_surface_tree(&self.log, renderer, frame, surface, location - delta, output_scale)?
        } else {
            *cursor_visible = true;
            ()
        })
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

    pub fn flush (&mut self, display: &Rc<RefCell<Display>>) {
        display.borrow_mut().flush_clients(self);
    }

    pub fn refresh (&mut self) {
        self.compositor.window_map.borrow_mut().refresh();
        self.compositor.output_map.borrow_mut().refresh();
    }

    pub fn import_bitmap<C: std::ops::Deref<Target = [u8]>>(
        renderer: &mut Gles2Renderer, image: &ImageBuffer<Rgba<u8>, C>,
    ) -> Result<Gles2Texture, Gles2Error> {
        use smithay::backend::renderer::gles2::ffi;
        renderer.with_context(|renderer, gl| unsafe {
            let mut tex = 0;
            gl.GenTextures(1, &mut tex);
            gl.BindTexture(ffi::TEXTURE_2D, tex);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
            gl.TexImage2D(
                ffi::TEXTURE_2D,
                0,
                ffi::RGBA as i32,
                image.width() as i32,
                image.height() as i32,
                0,
                ffi::RGBA,
                ffi::UNSIGNED_BYTE as u32,
                image.as_ptr() as *const _,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            Gles2Texture::from_raw(
                renderer,
                tex,
                (image.width() as i32, image.height() as i32).into(),
            )
        })
    }

}
