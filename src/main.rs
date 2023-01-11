#![feature(int_roundings, anonymous_lifetime_in_impl_trait)]

mod prelude;
mod traits;
mod engines;
mod state;

use crate::prelude::*;
use crate::engines::winit::WinitEngine;

fn main () -> Result<(), Box<dyn Error>> {
    let (logger, _guard) = init_log();
    App::new(WinitEngine::new(&logger)?)?
        .startup("glxgears", &[])
        .startup("wezterm", &[])
        .output("Alice",  720, 540, 0.0, 0.0)?
        .output("Bob",    480, 720, 0.0, 0.0)?
        .input("Charlie", "data/cursor.png")?
        .start()
}
