#![feature(int_roundings)]

mod prelude;
mod app;
mod compositor;
mod controller;
mod workspace;

use crate::prelude::*;
use crate::app::App;

fn main () -> Result<(), Box<dyn Error>> {
    let (log, _guard) = init_log();
    Ok(App::init(log)?
        .add_output(OUTPUT_NAME)
        .run(&mut Command::new("kitty"))
        .run(Command::new("chromium").arg("--ozone-platform=wayland"))
        .socket(true)
        .start())
}

fn init_log () -> (slog::Logger, GlobalLoggerGuard) {
    let fuse = slog_async::Async::default(slog_term::term_full().fuse()).fuse();
    let log = slog::Logger::root(fuse, o!());
    let guard = slog_scope::set_global_logger(log.clone());
    slog_stdlog::init().expect("Could not setup log backend");
    (log, guard)
}
