#![feature(int_roundings, anonymous_lifetime_in_impl_trait, associated_type_defaults)]

mod prelude;
mod traits;
mod engines;
mod state;
mod app;

use crate::prelude::*;
use crate::engines::winit::WinitEngine;
use smithay::backend::winit::WinitInput;

fn main () -> StdResult<()> {
    App::<WinitEngine, AppState, _, _>::new().run()
    //let state = AppState::new(&logger)?;
    //let app = App::<WinitEngine, _,_,_,>::new(&logger, state)?;
    //App::<WinitEngine, AppState, _, (InputEvent<WinitInput>, usize)>::new(&logger)?
        //.startup("glxgears", &[])
        //.startup("wezterm", &[])
        //.output("Alice",  720, 540, 0.0, 0.0)?
        //.output("Bob",    480, 720, 0.0, 0.0)?
        //.input("Charlie", "data/cursor.png")?
        //.start()
}
