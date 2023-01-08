pub(crate) use std::error::Error;

pub(crate) use std::rc::Rc;

pub(crate) use std::cell::{Cell, RefCell};

pub(crate) use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub(crate) use std::time::Instant;

pub(crate) use slog::{Logger, Drain, o, info, debug, warn, trace, error};

pub(crate) use smithay::reexports::calloop::{EventLoop, LoopHandle};

pub(crate) use smithay::reexports::wayland_server::{Display, DisplayHandle};

pub(crate) use smithay::utils::{Size, Point, Logical, Physical};

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

pub(crate) use crate::engine::{Engine, Stoppable};

pub(crate) use crate::state::{State, Screen, Window};
