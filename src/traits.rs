use crate::prelude::*;

pub type StdResult<T> = Result<T, Box<dyn Error>>;

/// Something that respond to user input.
pub trait Update<UpdateParams> {
    /// Respond to input
    fn update (&mut self, context: UpdateParams) -> StdResult<()>;
}

/// Something that can be rendered to a display.
pub trait Render<'r, RenderParams> {
    /// Render to display
    fn render (&'r mut self, context: &'r mut RenderParams) -> StdResult<()>;
}

/// Marker trait for Render + Update
pub trait Widget<'a, U, R>: Update<U> + Render<'a, R> {}

/// All types that implement Render + Update are widgets
impl<'a, U, R, W> Widget<'a, U, R> for W where W: Render<'a, R> + Update<U> {}

pub trait Engine<'r, U, R, W: Widget<'r, U, R>>: Widget<'r, W, W> + Outputs + Inputs + 'static {
    /// Create a new instance of this engine
    fn new (logger: &Logger, display: &DisplayHandle)
        -> Result<Self, Box<dyn Error>> where Self: Sized;
    /// Obtain a copy of the logger.
    fn logger (&self)
        -> Logger;
    /// Obtain a mutable reference to the renderer.
    fn renderer (&mut self)
        -> &mut Gles2Renderer;
}

pub trait Outputs {
    /// Called when an output is added
    fn output_added (&mut self, name: &str, screen: usize, width: i32, height: i32)
        -> Result<(), Box<dyn Error>> { unimplemented!(); }
    /// Called when an output's properties change
    fn output_changed (&mut self) -> Result<(), Box<dyn Error>> { unimplemented!(); }
    /// Called when an output is removed
    fn output_removed (&mut self) -> Result<(), Box<dyn Error>> { unimplemented!(); }
}

pub trait Inputs {
    /// Called when an input is added
    fn input_added (&mut self, name: &str) -> Result<(), Box<dyn Error>> { unimplemented!(); }
    /// Called when an input's properties change
    fn input_changed (&mut self) -> Result<(), Box<dyn Error>> { unimplemented!(); }
    /// Called when an input is removed
    fn input_removed (&mut self) -> Result<(), Box<dyn Error>> { unimplemented!(); }
}

// TODO:
// fn render (self, renderer, area) -> RenderResult
// struct RenderResult { used, damages }
