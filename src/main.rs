#![feature(int_roundings, anonymous_lifetime_in_impl_trait, associated_type_defaults)]

mod prelude;
mod traits;
mod engines;
mod state;
mod app;

use crate::prelude::*;
use crate::engines::winit::WinitEngine;

fn main () -> StdResult<()> {

    App::<WinitEngine, AppState>::new()?.with(|mut app|{
        app.state
            .startup("glxgears", &[])?
            .startup("wezterm", &[])?
            .output("Alice",  720, 540, 0.0, 0.0)?
            .output("Bob",    480, 720, 0.0, 0.0)?
            .input("Charlie", "data/cursor.png")?;
        Ok(app)
    })?.run()

}
