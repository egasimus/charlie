use crate::prelude::*;

use super::{WinitEngine, WinitHostWindow};

use smithay::backend::renderer::Bind;

pub type WinitRenderContext<'a> = &'a mut (
    &'a mut Gles2Renderer,
    &'a Output,
    Size<i32, Physical>,
    ScreenId
);

/// Render a compositor output into all host windows
impl<'r, W> Render<'r, W> for WinitEngine where W: Render<'r, W> {

    /// Render the app state on each output
    fn render (&'r mut self, state: &'r mut W) -> StdResult<()> {
        for (_, output) in self.outputs.iter() {
            output.render((self, state))?;
        }
        Ok(())
    }

}

/// Render a compositor output into this host window
impl<'r, W> Render<'r, (&'r mut WinitEngine, W)> for WinitHostWindow {

    /// Render the app state on this output
    fn render (&'r mut self, params: &'r mut (&'r mut WinitEngine, W)) -> StdResult<()> {
        let (engine, state) = params;
        if let Some(size) = self.resized.take() {
            self.surface.resize(size.w, size.h, 0, 0);
        }
        engine.renderer.bind(self.surface.clone())?;
        let size = self.surface.get_size().unwrap();
        state.render((&mut engine.renderer, &self.output, size, self.screen))?;
        self.surface.swap_buffers(None)?;
        Ok(())
    }

}

