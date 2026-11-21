use crate::Rect;

pub fn to_skia_matrix(matrix: glam::Mat3) -> skia_safe::Matrix {
    let values = matrix.to_cols_array();
    skia_safe::Matrix::new_all(
        values[0], values[3], values[6], values[1], values[4], values[7], values[2], values[5],
        values[8],
    )
}

impl From<Rect> for skia_safe::Rect {
    fn from(rect: Rect) -> Self {
        Self::from_ltrb(rect.min.x, rect.min.y, rect.max.x, rect.max.y)
    }
}

/// CSS physical units use 96 pixels per inch, independently of Skia's render DPI.
#[cfg(feature = "svg")]
pub fn svg_length_px(length: skia_safe::svg::Length) -> Option<f32> {
    use skia_safe::svg::LengthUnit;
    const CSS_PIXELS_PER_INCH: f32 = 96.0;
    let pixels_per_unit = match length.unit {
        LengthUnit::Number | LengthUnit::PX => 1.0,
        LengthUnit::IN => CSS_PIXELS_PER_INCH,
        LengthUnit::CM => CSS_PIXELS_PER_INCH / 2.54,
        LengthUnit::MM => CSS_PIXELS_PER_INCH / 25.4,
        LengthUnit::PT => CSS_PIXELS_PER_INCH / 72.0,
        LengthUnit::PC => CSS_PIXELS_PER_INCH / 6.0,
        _ => return None,
    };
    Some(length.value * pixels_per_unit)
}
