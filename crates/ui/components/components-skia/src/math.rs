pub fn cubic_bezier(progress: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..12 {
        let t = (low + high) / 2.0;
        if cubic(t, x1, x2) < progress {
            low = t;
        } else {
            high = t;
        }
    }
    cubic((low + high) / 2.0, y1, y2)
}

pub(crate) fn ease_in_out_sine(progress: f32) -> f32 {
    (1.0 - (std::f32::consts::PI * progress.clamp(0.0, 1.0)).cos()) / 2.0
}

fn cubic(t: f32, first: f32, second: f32) -> f32 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * first + 3.0 * inverse * t * t * second + t * t * t
}

/// Separate an intentional drag from finger jitter while allowing a second
/// contact to claim the gesture before any single-pointer edit begins.
pub(crate) fn touch_dragged(origin: crate::Vec2, point: crate::Vec2) -> bool {
    const TOUCH_DRAG_THRESHOLD: f32 = 8.0;
    origin.distance_squared(point) >= TOUCH_DRAG_THRESHOLD * TOUCH_DRAG_THRESHOLD
}

/// Centroid and RMS diameter keep pinch scaling independent of contact order.
pub(crate) fn touch_geometry(contacts: &[(u64, crate::Vec2)]) -> (crate::Vec2, f32) {
    let count = contacts.len() as f32;
    let center = contacts
        .iter()
        .map(|(_, point)| *point)
        .sum::<crate::Vec2>()
        / count;
    let span = (contacts
        .iter()
        .map(|(_, point)| point.distance_squared(center))
        .sum::<f32>()
        / count)
        .sqrt()
        * 2.0;
    (center, span)
}
