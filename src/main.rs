#![feature(int_roundings)]

mod prelude;
mod engine;
mod state;
//mod backend;
//mod app;
//mod compositor;
//mod controller;
//mod workspace;

use crate::prelude::*;
//use crate::app::App;
//use crate::backend::{Engine, Winit, Udev};

fn main () -> Result<(), Box<dyn Error>> {
    let (logger, _guard) = init_log();
    let mut engine = engine::winit::WinitEngine::new(&logger)?;
    engine.output_add()?;
    engine.output_add()?;
    engine.start(&mut State::new(&logger));
    Ok(())
}
