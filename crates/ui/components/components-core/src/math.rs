pub fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

pub fn number_rotation(value: f64, offset_degrees: f64) -> u16 {
    const FULL_ROTATION_DEGREES: i64 = 360;
    ((value + offset_degrees).round() as i64).rem_euclid(FULL_ROTATION_DEGREES) as u16
}

pub fn split_pane_extent(total: f32, divider: f32, fraction: f32) -> f32 {
    (total - divider).max(0.0) * fraction
}

pub fn split_pane_fraction(leading: f32, trailing: f32) -> Option<f32> {
    let content = leading + trailing;
    (leading > 0.0 && trailing > 0.0 && content.is_finite()).then(|| trailing / content)
}
