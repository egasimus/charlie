#![feature(int_roundings)]

mod prelude;
mod engine;
mod state;
mod xwayland;
//mod backend;
//mod app;
//mod compositor;
//mod controller;
//mod workspace;

use crate::prelude::*;
use crate::engine::winit::WinitEngine;
use crate::xwayland::XWaylandState;
//use crate::app::App;
//use crate::backend::{Engine, Winit, Udev};

fn main () -> Result<(), Box<dyn Error>> {
    let (logger, _guard) = init_log();
    let mut engine = WinitEngine::new(&logger)?;
    let xwayland = XWaylandState::new(&logger, engine.event_handle(), &engine.display_handle())?;
    engine.output_add("Alice")?;
    engine.output_add("Bob")?;
    engine.start(&mut State::new(&logger, xwayland));
    Ok(())
}
