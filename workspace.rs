use crate::prelude::*;

pub struct Workspace {
    pub log:        Logger,
    pub background: Gles2Texture,
    pub dragging:   bool,
    pub offset:     Point<f64, Physical>
}

impl Workspace {
    pub fn init (
        log:      &Logger,
        renderer: &Rc<RefCell<WinitGraphicsBackend>>
    ) -> Result<Self, Box<dyn Error>> {
        let log = log.clone();
        let background = &image::io::Reader::open(BACKGROUND)?.with_guessed_format().unwrap()
            .decode().unwrap().to_rgba8();
        let background = import_bitmap(renderer.borrow_mut().renderer(), background)?;
        Ok(Self {
            log,
            background,
            dragging: false,
            offset: (0.0, 0.0).into(),
        })
    }

    pub fn draw (
        &self,
        frame:        &mut Gles2Frame,
        output_size:  Size<i32, Logical>,
        output_scale: f32
    ) -> Result<(), SwapBuffersError> {
        let background_size: Size<i32, Buffer>  = self.background.size();
        let background_size: Size<i32, Logical> = background_size.to_logical(1);
        let background_tile_x = output_size.w.div_ceil(background_size.w) + 1;
        let background_tile_y = output_size.h.div_ceil(background_size.h) + 1;
        for x in 0..background_tile_x {
            for y in 0..background_tile_y {
                let offset: Point<f64, Physical> = self.offset;
                let offset: Point<i32, Logical> = offset
                    .to_logical(output_scale as f64)
                    .to_i32_round();
                let offset_x = (x * background_size.w) + offset.x;
                let offset_y = (y * background_size.h) + offset.y;
                let offset: Point<i32, Logical> = (offset_x, offset_y).into();
                let mut offset: Point<f64, Physical> = offset
                    .to_f64()
                    .to_physical(output_scale as f64);
                if offset.x > output_size.w as f64 {
                    offset.x -= output_size.w as f64 + background_size.w as f64
                }
                if offset.y > output_size.h  as f64{
                    offset.y -= output_size.h as f64 + background_size.h as f64
                }
                frame.render_texture_at(
                    &self.background,
                    offset,
                    1,
                    output_scale.into(),
                    Transform::Normal,
                    1.0
                )?;
            }
        };
        Ok(())
    }

    pub fn on_pointer_move_absolute (
        &mut self,
        pointer_location:      Point<f64, Logical>,
        last_pointer_location: Point<f64, Logical>
    ) {
        if self.dragging {
            let delta = pointer_location - last_pointer_location;
            self.offset += delta.to_physical(1.0);
        }
    }
}
