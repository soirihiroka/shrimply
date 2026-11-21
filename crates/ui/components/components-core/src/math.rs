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

/// Preview, inspector and full-width timeline bounds, in the same logical units as `size`.
/// Hidden panes retain their content bounds for native exit animations.
pub fn editor_pane_bounds(
    size: [f32; 2],
    inspector_width: f32,
    timeline_fraction: f32,
    inspector_visible: bool,
    timeline_visible: bool,
) -> [[f32; 4]; 3] {
    let [width, height] = size.map(|value| value.max(0.0));
    let divider = crate::layout::PANEL_DIVIDER_WIDTH;
    let inspector_width = inspector_width.clamp(0.0, width);
    let timeline_height = split_pane_extent(height, divider, timeline_fraction.clamp(0.0, 1.0));
    let top_height = if timeline_visible {
        (height - timeline_height - divider).max(0.0)
    } else {
        height
    };
    let preview_left =
        if inspector_visible && !crate::layout::inspector_overlay(width, inspector_width) {
            (inspector_width + divider).min(width)
        } else {
            0.0
        };
    [
        [preview_left, 0.0, width - preview_left, top_height],
        [0.0, 0.0, inspector_width, top_height],
        [
            0.0,
            height - timeline_height - divider,
            width,
            timeline_height + divider,
        ],
    ]
}

pub fn image_sample_point(size: [u32; 2], position: [f64; 2], delta: [i32; 2]) -> [u32; 2] {
    std::array::from_fn(|axis| {
        assert!(size[axis] > 0, "image dimensions must be positive");
        ((position[axis].clamp(0.0, 1.0) * f64::from(size[axis])) as u32)
            .min(size[axis] - 1)
            .saturating_add_signed(delta[axis])
            .min(size[axis] - 1)
    })
}

#[derive(Clone, Copy)]
pub enum ColorSelection {
    Plane,
    Hue,
    Alpha,
}

/// Match the picker axes on every renderer: value increases rightward and saturation upward.
pub fn color_selection(
    mut color: crate::color::Hsva,
    control: ColorSelection,
    position: [f32; 2],
) -> crate::color::Hsva {
    let [x, y] = position.map(|value| value.clamp(0.0, 1.0));
    match control {
        ColorSelection::Plane => {
            color.value = x;
            color.saturation = 1.0 - y;
        }
        ColorSelection::Hue => color.hue = x * 360.0,
        ColorSelection::Alpha => color.alpha = x,
    }
    color
}

pub fn color_selection_key(
    color: crate::color::Hsva,
    control: ColorSelection,
    key: &str,
) -> Option<crate::color::Hsva> {
    let byte_step = 1.0 / u8::MAX as f32;
    let position = match control {
        ColorSelection::Plane => match key {
            "left" => [color.value - byte_step, 1.0 - color.saturation],
            "right" => [color.value + byte_step, 1.0 - color.saturation],
            "up" => [color.value, 1.0 - color.saturation - byte_step],
            "down" => [color.value, 1.0 - color.saturation + byte_step],
            _ => return None,
        },
        ColorSelection::Hue => [
            (color.hue
                + match key {
                    "up" | "left" => -1.0,
                    "down" | "right" => 1.0,
                    _ => return None,
                })
                / 360.0,
            0.0,
        ],
        ColorSelection::Alpha => [
            color.alpha
                + match key {
                    "left" | "down" => -byte_step,
                    "right" | "up" => byte_step,
                    _ => return None,
                },
            0.0,
        ],
    };
    Some(color_selection(color, control, position))
}
