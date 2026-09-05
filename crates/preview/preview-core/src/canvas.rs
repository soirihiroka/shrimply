use crate::{Color, Rect};
use skia_safe::{Canvas, Matrix, Paint, RuntimeEffect, runtime_effect::RuntimeShaderBuilder};

pub struct Appearance {
    pub content_rect: Rect,
    pub background: Color,
    pub shadow_size: u32,
    pub pixel_scale: f32,
}

thread_local! {
    static BACKGROUND: RuntimeEffect = RuntimeEffect::make_for_shader(include_str!("../shaders/canvas.sksl"), None)
        .expect("shared preview background shader must compile");
}

pub fn draw_background(canvas: &Canvas, appearance: Appearance) {
    let rect = appearance.content_rect;
    let color = appearance.background;
    BACKGROUND.with(|effect| {
        let mut builder = RuntimeShaderBuilder::new(effect.clone());
        for (name, values) in [
            (
                "u_content_rect",
                &[rect.min.x, rect.min.y, rect.width(), rect.height()][..],
            ),
            (
                "u_background_color",
                &[color.r, color.g, color.b, color.a][..],
            ),
            ("u_shadow_size", &[appearance.shadow_size as f32][..]),
            ("u_pixel_scale", &[appearance.pixel_scale][..]),
        ] {
            builder
                .set_uniform_float(name, values)
                .expect("preview shader uniform type");
        }
        let mut paint = Paint::default();
        paint.set_blend_mode(skia_safe::BlendMode::Src);
        paint.set_shader(
            builder
                .make_shader(&Matrix::new_identity())
                .expect("create preview background shader"),
        );
        canvas.draw_paint(&paint);
    });
}
