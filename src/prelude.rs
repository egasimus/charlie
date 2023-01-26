pub(crate) use crate::{
    traits::*,
    state::App,
    state::desktop::ScreenState
};

pub(crate) use std::{
    error::Error,
    rc::Rc,
    cell::{Cell, RefCell},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Instant, Duration},
    path::Path,
    collections::{HashMap, hash_map::Entry},
    os::fd::AsRawFd,
    any::TypeId,
    //marker::PhantomData
};

pub(crate) use slog::{Logger, Drain, o, info, debug, warn, error, crit};

pub(crate) use smithay::backend::{
    input::{
        InputBackend,
        InputEvent,
    },
    renderer::{
        Renderer,
        Frame,
        //damage::DamageTrackedRenderer,
        gles2::{
            Gles2Renderer, 
            Gles2Frame,
            Gles2Texture,
        },
    }
};

pub(crate) use smithay::input::Seat;

pub(crate) use smithay::output::Output;

pub(crate) use smithay::reexports::calloop::{EventLoop, LoopHandle};

pub(crate) use smithay::reexports::wayland_server::{Display, DisplayHandle};

pub(crate) use smithay::utils::{Point, Size, Rectangle, Logical, Physical};

pub(crate) fn init_log () -> (Logger, slog_scope::GlobalLoggerGuard) {
    // A logger facility, here we use the terminal here
    let log = if std::env::var("ANVIL_MUTEX_LOG").is_ok() {
        slog::Logger::root(std::sync::Mutex::new(slog_term::term_full().fuse()).fuse(), o!())
    } else {
        slog::Logger::root(
            slog_async::Async::default(slog_term::term_full().fuse()).fuse(),
            o!(),
        )
    };
    let _guard = slog_scope::set_global_logger(log.clone());
    slog_stdlog::init().expect("Could not setup log backend");
    debug!(&log, "Logger initialized");
    (log, _guard)
}

pub fn import_bitmap (renderer: &mut Gles2Renderer, path: impl AsRef<Path>)
    -> Result<Gles2Texture, Box<dyn Error>>
{
    let image = image::io::Reader::open(path)?.with_guessed_format()?.decode()?.to_rgba8();
    let size = (image.width() as i32, image.height() as i32);
    let mut tex = 0;
    renderer.with_context(|gl| unsafe {
        use smithay::backend::renderer::gles2::ffi;
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
    })?;
    Ok(unsafe {
        Gles2Texture::from_raw(renderer, tex, size.into())
    })
}

pub type ScreenId = usize;
