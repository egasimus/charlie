#![feature(int_roundings)]

mod prelude;
mod backend;
mod app;
mod compositor;
mod controller;
mod workspace;

use crate::prelude::*;
use crate::app::App;
use crate::backend::{Engine, Winit, Udev};

fn main () -> Result<(), Box<dyn Error>> {
    let (log, _guard) = init_log();
    Winit::init(&log)?.run(|app|app
        .run(Command::new("glxgears"))
        .run(Command::new("kitty")))
}

fn init_log () -> (slog::Logger, GlobalLoggerGuard) {
    let fuse = slog_async::Async::default(slog_term::term_full().fuse()).fuse();
    let log = slog::Logger::root(fuse, o!());
    let guard = slog_scope::set_global_logger(log.clone());
    slog_stdlog::init().expect("Could not setup log backend");
    (log, guard)
}
