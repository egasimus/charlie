#![feature(int_roundings, anonymous_lifetime_in_impl_trait)]

mod prelude;
mod traits;
mod engines;
mod state;

use crate::prelude::*;
use crate::engines::winit::WinitEngine;
use crate::state::Screen;

fn main () -> Result<(), Box<dyn Error>> {
    let (logger, _guard) = init_log();
    let mut engine = WinitEngine::new(&logger)?;
    let mut state  = State::new(&mut engine)?;
    state.startup_add("glxgears", &[]);
    state.startup_add("wezterm",  &[]);
    engine.output_add("Alice", state.screen_add(Screen::new((-100.0, 0.0), (0.0, 0.0))))?;
    engine.output_add("Bob",   state.screen_add(Screen::new(( 100.0, 0.0), (0.0, 0.0))))?;
    engine.start(&mut state)
}
