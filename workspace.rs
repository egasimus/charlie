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
        let size:     Size<i32, Buffer>    = self.background.size();
        let size:     Size<i32, Logical>   = size.to_logical(1);
        let tiles_x:  i32                  = output_size.w.div_ceil(size.w);
        let tiles_y:  i32                  = output_size.h.div_ceil(size.h);
        let offset:   Point<f64, Physical> = self.offset;
        let offset:   Point<i32, Logical>  = offset.to_logical(output_scale as f64).to_i32_round();
        let offset_x: i32                  = offset.x % size.w;
        let offset_y: i32                  = offset.y % size.h;
        for x in -1..tiles_x + 1 {
            for y in -1..tiles_y + 1 {
                let offset: Point<i32, Physical> =
                    ((x * size.w) + offset_x, (y * size.h) + offset_y).into();
                let offset: Point<f64, Physical> = offset.to_f64();
                frame.render_texture_at(
                    &self.background,
                    offset.into(),
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
