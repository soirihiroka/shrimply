use crate::TimelineState;
use shrimply_math_core::{Fraction, Time};
use shrimply_timeline_paint_skia::{LABEL_WIDTH, RULER_HEIGHT, TRACK_HEIGHT};

const MIN_SECONDS_PER_PIXEL: Fraction = Fraction::new_raw(1_u64, 100_000_u64);
const MAX_SECONDS_PER_PIXEL: Fraction = Fraction::new_raw(60_u64, 1_u64);

/// Input coordinates are dp; time stays rational after crossing the JNI boundary.
pub fn gesture(state: &mut TimelineState, values: &[f32], width: f64, height: f64) {
    let [kind, x, _y, dx, dy, zoom] = *values else {
        panic!("invalid timeline input batch");
    };
    assert!(
        values.iter().all(|value| value.is_finite()),
        "nonfinite timeline input"
    );
    assert!(zoom > 0.0, "timeline zoom must be positive");
    let scale = &mut state.viewport.scale;
    match kind as u32 {
        0 => {
            let anchor = Fraction::from((f64::from(x) - LABEL_WIDTH).max(0.0));
            let at = scale.scroll_seconds.saturating_add(
                Time {
                    seconds: scale.seconds_per_pixel,
                }
                .scaled(anchor),
            );
            scale.seconds_per_pixel = Time {
                seconds: scale.seconds_per_pixel,
            }
            .scaled(Fraction::from(1) / Fraction::from(zoom))
            .seconds
            .clamp(MIN_SECONDS_PER_PIXEL, MAX_SECONDS_PER_PIXEL);
            scale.scroll_seconds = at
                .signed_sub(
                    Time {
                        seconds: scale.seconds_per_pixel,
                    }
                    .scaled(anchor),
                )
                .signed_sub(
                    Time {
                        seconds: scale.seconds_per_pixel,
                    }
                    .scaled(Fraction::from(dx)),
                );
            state.viewport.scroll_y -= f64::from(dy);
        }
        1 => {
            state.playhead = scale
                .scroll_seconds
                .saturating_add(
                    Time {
                        seconds: scale.seconds_per_pixel,
                    }
                    .scaled(Fraction::from((f64::from(x) - LABEL_WIDTH).max(0.0))),
                )
                .clamp(Time::ZERO, state.duration);
        }
        _ => panic!("unknown timeline input kind"),
    }
    clamp_viewport(state, width, height);
}

pub fn clamp_viewport(state: &mut TimelineState, width: f64, height: f64) {
    let scale = &mut state.viewport.scale;
    let visible = Time {
        seconds: scale.seconds_per_pixel,
    }
    .scaled(Fraction::from((width - LABEL_WIDTH).max(0.0)));
    scale.scroll_seconds = scale
        .scroll_seconds
        .clamp(Time::ZERO, state.duration.saturating_sub(visible));
    state.viewport.scroll_y = state.viewport.scroll_y.clamp(
        0.0,
        (state.tracks.len() as f64 * TRACK_HEIGHT - (height - RULER_HEIGHT).max(0.0)).max(0.0),
    );
}

pub fn percentile(values: &[u128], percentile: usize) -> u128 {
    if values.is_empty() {
        0
    } else {
        values[(values.len() - 1) * percentile / 100]
    }
}
