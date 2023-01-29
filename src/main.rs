#![feature(int_roundings, anonymous_lifetime_in_impl_trait, associated_type_defaults)]

mod prelude;
mod traits;
mod engines;
mod state;

use crate::prelude::*;
use crate::engines::winit::WinitEngine;

fn main () -> StdResult<()> {
    let (logger, _guard) = init_log();
    Charlie::<WinitEngine>::new(logger)?
        .startup("glxgears", &[])?
        .startup("wezterm", &[])?
        .output("Alice",  720, 540, 0.0, 0.0)?
        .output("Bob",    480, 720, 0.0, 0.0)?
        .input("Charlie", "data/cursor.png")?
        .run()
}
