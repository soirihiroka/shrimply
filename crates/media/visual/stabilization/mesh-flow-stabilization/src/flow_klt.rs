use opencv::core::{self, Mat, Point2f, Size, TermCriteria, TermCriteria_Type, Vector};
use opencv::{imgproc, video};

opencv::opencv_branch_5! {
    use opencv::features::good_features_to_track;
}

opencv::not_opencv_branch_5! {
    use opencv::imgproc::good_features_to_track;
}

const MAXIMUM_FEATURES: i32 = 1_000;
const FEATURE_QUALITY: f64 = 0.01;
const MINIMUM_FEATURE_DISTANCE: f64 = 12.0;
const FEATURE_BLOCK_SIZE: i32 = 3;
const OPTICAL_FLOW_WINDOW: i32 = 20;
const OPTICAL_FLOW_PYRAMID_LEVELS: i32 = 3;
const OPTICAL_FLOW_ITERATIONS: i32 = 20;
const OPTICAL_FLOW_EPSILON: f64 = 0.01;
pub(super) fn tracked_features(previous: &Mat, current: &Mat) -> Result<Vec<[f32; 6]>, String> {
    let mut previous_gray = Mat::default();
    let mut current_gray = Mat::default();
    imgproc::cvt_color_def(previous, &mut previous_gray, imgproc::COLOR_BGR2GRAY)
        .map_err(|error| error.to_string())?;
    imgproc::cvt_color_def(current, &mut current_gray, imgproc::COLOR_BGR2GRAY)
        .map_err(|error| error.to_string())?;
    let mut previous_points = Vector::<Point2f>::new();
    good_features_to_track(
        &previous_gray,
        &mut previous_points,
        MAXIMUM_FEATURES,
        FEATURE_QUALITY,
        MINIMUM_FEATURE_DISTANCE,
        &core::no_array(),
        FEATURE_BLOCK_SIZE,
        false,
        0.04,
    )
    .map_err(|error| error.to_string())?;
    if previous_points.len() < 4 {
        return Ok(Vec::new());
    }
    let mut current_points = Vector::<Point2f>::new();
    let mut status = Vector::<u8>::new();
    let mut errors = Vector::<f32>::new();
    video::calc_optical_flow_pyr_lk(
        &previous_gray,
        &current_gray,
        &previous_points,
        &mut current_points,
        &mut status,
        &mut errors,
        Size::new(OPTICAL_FLOW_WINDOW, OPTICAL_FLOW_WINDOW),
        OPTICAL_FLOW_PYRAMID_LEVELS,
        TermCriteria::new(
            TermCriteria_Type::COUNT as i32 | TermCriteria_Type::EPS as i32,
            OPTICAL_FLOW_ITERATIONS,
            OPTICAL_FLOW_EPSILON,
        )
        .map_err(|error| error.to_string())?,
        0,
        0.0001,
    )
    .map_err(|error| error.to_string())?;
    Ok(previous_points
        .iter()
        .zip(current_points.iter())
        .zip(status.iter().zip(errors.iter()))
        .filter_map(|((early, late), (valid, error))| {
            (valid != 0 && error.is_finite()).then_some([
                early.x,
                early.y,
                late.x,
                late.y,
                1.0 / (1.0 + error.max(0.0)),
                1.0,
            ])
        })
        .collect())
}
