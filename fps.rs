use prelude::*;

pub static FPS_NUMBERS_PNG: &[u8] = include_bytes!("../resources/numbers.png");

pub fn draw_fps<R, E, F, T>(
    _renderer:    &mut R,
    frame:        &mut F,
    texture:      &T,
    output_scale: f64,
    value:        u32,
) -> Result<(), SwapBuffersError>
where
    R: Renderer<Error = E, TextureId = T, Frame = F> + ImportAll,
    F: Frame<Error = E, TextureId = T>,
    E: std::error::Error + Into<SwapBuffersError>,
    T: Texture + 'static,
{
    let value_str = value.to_string();
    let mut offset_x = 0f64;
    for digit in value_str.chars().map(|d| d.to_digit(10).unwrap()) {
        frame
            .render_texture_from_to(
                texture,
                match digit {
                    9 => Rectangle::from_loc_and_size((0, 0), (22, 35)),
                    6 => Rectangle::from_loc_and_size((22, 0), (22, 35)),
                    3 => Rectangle::from_loc_and_size((44, 0), (22, 35)),
                    1 => Rectangle::from_loc_and_size((66, 0), (22, 35)),
                    8 => Rectangle::from_loc_and_size((0, 35), (22, 35)),
                    0 => Rectangle::from_loc_and_size((22, 35), (22, 35)),
                    2 => Rectangle::from_loc_and_size((44, 35), (22, 35)),
                    7 => Rectangle::from_loc_and_size((0, 70), (22, 35)),
                    4 => Rectangle::from_loc_and_size((22, 70), (22, 35)),
                    5 => Rectangle::from_loc_and_size((44, 70), (22, 35)),
                    _ => unreachable!(),
                },
                Rectangle::from_loc_and_size((offset_x, 0.0), (22.0 * output_scale, 35.0 * output_scale)),
                Transform::Normal,
                1.0,
            )
            .map_err(Into::into)?;
        offset_x += 24.0 * output_scale;
    }

    Ok(())
}
