use skia_safe::{
    AlphaType, Canvas, ColorType, CubicResampler, Data, Image, ImageInfo, Paint, Rect as SkiaRect,
    images,
};

use crate::{
    Vec2,
    canvas::{Color, Rect, Stroke, TimelinePainter, vec2},
};

pub struct SoftwareCursor {
    image: Image,
    hot_spot: Vec2,
    size: Vec2,
}

impl SoftwareCursor {
    pub fn from_rgba_premultiplied(
        pixels: &[u8],
        width: u32,
        height: u32,
        hot_spot: Vec2,
        size: Vec2,
    ) -> Option<Self> {
        let width = i32::try_from(width).ok()?;
        let height = i32::try_from(height).ok()?;
        let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
        let info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let image = images::raster_from_data(&info, Data::new_copy(pixels), row_bytes)?;
        Some(Self {
            image,
            hot_spot,
            size,
        })
    }

    pub fn draw(&self, canvas: &Canvas, position: Vec2) {
        let top_left = position.round() - self.hot_spot;
        canvas.draw_image_rect_with_sampling_options(
            &self.image,
            None,
            SkiaRect::from_xywh(top_left.x, top_left.y, self.size.x, self.size.y),
            CubicResampler::mitchell(),
            &Paint::default(),
        );
    }
}

pub struct PlayheadStyle {
    pub ruler_height: f64,
    pub frame_y: Option<f64>,
    pub handle_width: f64,
    pub handle_height: f64,
    pub handle_top: f64,
    pub triangle_height: f64,
}

pub fn draw_playhead(
    painter: &TimelinePainter,
    playhead_x: f64,
    frame_width: f64,
    height: f64,
    color: Color,
    style: PlayheadStyle,
) {
    if height <= 0.0 {
        return;
    }

    let frame_width = frame_width.max(1.0);
    let frame_y = style.frame_y.unwrap_or(style.ruler_height - 5.0);
    let handle_left = playhead_x - style.handle_width / 2.0;
    let handle_bottom = style.handle_top + style.handle_height;
    let triangle_tip_y = handle_bottom + style.triangle_height;

    painter.rect_filled(
        rect(
            handle_left,
            style.handle_top,
            style.handle_width,
            style.handle_height,
        ),
        1,
        color,
    );
    painter.convex_polygon(
        &[
            vec2(handle_left as f32, handle_bottom as f32),
            vec2(
                (handle_left + style.handle_width) as f32,
                handle_bottom as f32,
            ),
            vec2(playhead_x as f32, triangle_tip_y as f32),
        ],
        color,
        Stroke::none(),
    );
    painter.rect_filled(rect(playhead_x, frame_y, frame_width, 4.0), 0, color);
    painter.rect_filled(
        rect(
            playhead_x - 1.0,
            triangle_tip_y,
            2.0,
            height - triangle_tip_y,
        ),
        0,
        color,
    );
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_min_size(
        vec2(x as f32, y as f32),
        vec2(width.max(0.0) as f32, height.max(0.0) as f32),
    )
}
