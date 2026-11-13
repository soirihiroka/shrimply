//! Overlay scrollbar geometry in logical points. No rendering or element traversal.
use crate::Overflow;

pub const GUTTER: f64 = 12.;
pub const THICKNESS: f64 = 6.;
const EDGE: f64 = 3.;
const MIN_THUMB: f64 = 24.;

#[derive(Clone, Debug, PartialEq)]
pub struct Scrollbar {
    pub track: [f64; 4],
    pub thumb: [f64; 4],
}

pub fn bars(
    viewport: [f64; 4],
    range: [f64; 2],
    offset: [f64; 2],
    overflow: [Overflow; 2],
) -> Vec<Scrollbar> {
    let shown = std::array::from_fn::<_, 2, _>(|axis| {
        overflow[axis] == Overflow::Scroll || overflow[axis] == Overflow::Auto && range[axis] > 0.
    });
    if !shown.into_iter().any(|value| value) {
        return Vec::new();
    }
    let mut bars = Vec::with_capacity(2);
    for axis in 0..2 {
        if !shown[axis] {
            continue;
        }
        let cross = 1 - axis;
        let length = viewport[axis + 2] - viewport[axis];
        let start = viewport[axis] + EDGE;
        let end = viewport[axis + 2] - EDGE - if shown[cross] { GUTTER } else { 0. };
        let track_length = (end - start).max(0.);
        if track_length == 0. || viewport[cross + 2] - viewport[cross] < GUTTER {
            continue;
        }
        let extent = (length + range[axis]).max(length);
        let thumb_length = (track_length * length / extent)
            .max(MIN_THUMB)
            .min(track_length);
        let progress = if range[axis] == 0. {
            0.
        } else {
            (offset[axis] / range[axis]).clamp(0., 1.)
        };
        let thumb_start = start + (track_length - thumb_length) * progress;
        let mut track = viewport;
        track[axis] = start;
        track[axis + 2] = end;
        track[cross] = viewport[cross + 2] - EDGE - THICKNESS;
        track[cross + 2] = viewport[cross + 2] - EDGE;
        let mut thumb = track;
        thumb[axis] = thumb_start;
        thumb[axis + 2] = thumb_start + thumb_length;
        bars.push(Scrollbar { track, thumb });
    }
    bars
}
