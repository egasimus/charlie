use super::prelude::*;

use smithay::input::{
    pointer::{
        CursorImageStatus as Status,
        CursorImageAttributes as Attributes
    }
};

pub struct Pointer {
    logger:        Logger,
    texture:       Gles2Texture,
    status:        Arc<Mutex<Status>>,
    position:      Point<f64, Logical>,
    last_position: Point<f64, Logical>,
}

impl Pointer {
    pub fn new  (
        engine: &mut impl Engine,
        /*seat: &Seat<State>*/
    ) -> Result<Self, Box<dyn Error>> {
        //seat.add_pointer();
        Ok(Self {
            logger:        engine.logger(),
            texture:       import_bitmap(engine.renderer(), "data/cursor.png")?,
            status:        Arc::new(Mutex::new(Status::Default)),
            position:      (100.0, 30.0).into(),
            last_position: (0.0, 0.0).into(),
        })
    }
    fn status (&self) -> (bool, Point<f64, Logical>) {
        let mut reset = false;
        let mut guard = self.status.lock().unwrap();
        if let Status::Surface(ref surface) = *guard {
            reset = !surface.alive();
        }
        if reset {
            *guard = Status::Default;
        }
        let visible = !matches!(*guard, Status::Surface(_));
        let hotspot = if let Status::Surface(ref surface) = *guard {
            with_states(surface, |states| {
                states.data_map.get::<Mutex<Attributes>>().unwrap()
                    .lock().unwrap().hotspot
            })
        } else {
            (0, 0).into()
        };
        let position = self.position - hotspot.to_f64();
        (visible, position)
    }
    pub fn render (
        &self,
        frame:  &mut Gles2Frame,
        size:   Size<i32, Physical>,
        screen: &Screen
    ) -> Result<(), Box<dyn Error>> {
        let damage = Rectangle::<i32, Physical>::from_loc_and_size(
            Point::<i32, Physical>::from((0i32, 0i32)),
            size
        );
        let x = self.position.x + screen.center().x;
        let y = self.position.y + screen.center().y;
        let position = Point::<f64, Logical>::from((x, y)).to_physical(1.0).to_i32_round();
        debug!(&self.logger, "Render pointer at {position:?} ({damage:?})");
        //let size = self.texture.size();
        Ok(frame.render_texture_at(
            &self.texture,
            position,
            1,
            1.0,
            Transform::Flipped180,
            &[damage],
            1.0
        )?)
    }
        //match *(self.status.lock().unwrap()) {
            //Status::Hidden => vec![],
            //Status::Default => {
                //if let Some(texture) = self.texture.as_ref() {
                    //frame.render_texture_from_to(
                        //&self.texture, src, dst, damage, self.transform, self.alpha)
                    //vec![
                        //PointerRenderElement::<R>::from(TextureRenderElement::from_texture_buffer(
                            //location.to_f64(),
                            //texture,
                            //None,
                            //None,
                            //None,
                        //))
                        //.into(),
                    //]
                //} else {
                    //vec![]
                //}
            //}
            //CursorImageStatus::Surface(surface) => {
                //panic!();
                ////let elements: Vec<PointerRenderElement<R>> =
                    ////smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                        ////renderer,
                        ////surface,
                        ////location,
                        ////scale,
                        ////None,
                    ////);
                ////elements.into_iter().map(E::from).collect()
            //}
        //};
}
