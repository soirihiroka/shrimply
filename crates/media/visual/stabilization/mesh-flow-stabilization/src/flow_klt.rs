use shrimply_vision::{FeatureOptions, FlowOptions, RgbaImage, good_features};

const MAXIMUM_FEATURES: usize = 1_000;
const FEATURE_QUALITY: f64 = 0.01;
const MINIMUM_FEATURE_DISTANCE: f64 = 12.0;
const FEATURE_BLOCK_SIZE: usize = 3;
const OPTICAL_FLOW_WINDOW: usize = 20;
const OPTICAL_FLOW_PYRAMID_LEVELS: usize = 3;
const OPTICAL_FLOW_ITERATIONS: usize = 20;
const OPTICAL_FLOW_EPSILON: f64 = 0.01;
pub(super) fn tracked_features(
    tracker: &mut shrimply_affine_l1_stabilization::sparse::Tracker,
    previous: &RgbaImage,
    current: &RgbaImage,
) -> Result<Vec<[f32; 6]>, String> {
    let previous_gray = previous.gray()?;
    let current_gray = current.gray()?;
    let points = good_features(
        &previous_gray,
        FeatureOptions {
            maximum: MAXIMUM_FEATURES,
            quality: FEATURE_QUALITY,
            minimum_distance: MINIMUM_FEATURE_DISTANCE,
            block_size: FEATURE_BLOCK_SIZE,
        },
    )?;
    if points.len() < 4 {
        return Ok(Vec::new());
    }
    Ok(tracker
        .track(
            &previous_gray,
            &current_gray,
            &points,
            FlowOptions {
                window: OPTICAL_FLOW_WINDOW,
                maximum_level: OPTICAL_FLOW_PYRAMID_LEVELS,
                iterations: OPTICAL_FLOW_ITERATIONS,
                epsilon: OPTICAL_FLOW_EPSILON,
                minimum_eigenvalue: 0.0001,
            },
        )?
        .into_iter()
        .filter_map(|track| {
            (track.valid && track.error.is_finite()).then_some([
                track.previous[0],
                track.previous[1],
                track.current[0],
                track.current[1],
                1.0 / (1.0 + track.error.max(0.0)),
                1.0,
            ])
        })
        .collect())
}
