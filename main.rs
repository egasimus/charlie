mod prelude;
mod app;
mod surface;
mod compositor;
mod controller;

use crate::prelude::*;
use crate::app::App;

fn main () -> Result<(), Box<dyn Error>> {
    let (log, _guard) = App::init_log();
    let display = Rc::new(RefCell::new(Display::new()));
    let (renderer, input) = App::init_io(&log, &display)?;
    let event_loop = EventLoop::try_new().unwrap();
    let mut charlie = App::init(log, &display, &renderer, &event_loop)?;
    charlie.add_output(OUTPUT_NAME);
    std::process::Command::new("kitty").spawn()?;
    Ok(charlie.run(&display, input, event_loop))
}
