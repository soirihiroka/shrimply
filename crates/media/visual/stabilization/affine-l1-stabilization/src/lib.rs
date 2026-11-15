#[cfg(not(feature = "nvidia"))]
pub mod flow;
#[cfg(target_os = "macos")]
pub mod flow_metal;
#[cfg(feature = "nvidia")]
mod flow_nvidia;
pub mod sparse;
pub mod video;

use glam::{Mat3, UVec2, Vec2, Vec3};
use good_lp::{
    Expression, IntoAffineExpression, ProblemVariables, Solution, SolutionStatus, SolverModel,
    Variable, highs, variable,
};
use std::path::Path;

use shrimply_vision::math::{AffineOptions, estimate_affine as fit_affine};
use shrimply_vision::{FeatureOptions, FlowOptions, RgbaImage, good_features};

const MAXIMUM_FEATURES: usize = 200;
const FEATURE_QUALITY: f64 = 0.01;
const MINIMUM_FEATURE_DISTANCE: f64 = 30.0;
const FEATURE_BLOCK_SIZE: usize = 3;
const OPTICAL_FLOW_WINDOW: usize = 20;
const OPTICAL_FLOW_PYRAMID_LEVELS: usize = 3;
const OPTICAL_FLOW_ITERATIONS: usize = 10;
const OPTICAL_FLOW_EPSILON: f64 = 0.03;
const MINIMUM_TRACKED_FEATURES: usize = 4;
const RANSAC_REPROJECTION_THRESHOLD: f64 = 3.0;
const RANSAC_MAXIMUM_ITERATIONS: usize = 2_000;
const RANSAC_CONFIDENCE: f64 = 0.99;
const RANSAC_REFINEMENT_ITERATIONS: usize = 10;
const PARAMETER_WEIGHTS: [f64; 6] = [1.0, 1.0, 100.0, 100.0, 100.0, 100.0];
const MINIMUM_CROP_RATIO: f64 = 0.1;
const MAXIMUM_CROP_RATIO: f64 = 1.0;
const MINIMUM_DETERMINANT: f32 = 1e-12;
const MAXIMUM_SHEAR: f64 = 0.1;
const MAXIMUM_ASPECT_CHANGE: f64 = 0.05;

const DX: usize = 0;
const DY: usize = 1;
const A: usize = 2;
const B: usize = 3;
const C: usize = 4;
const D: usize = 5;

#[derive(Clone, Copy, Debug)]
pub struct StabilizationOptions {
    pub crop_ratio: f64,
    pub derivative_weights: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct StabilizationChunk {
    pub first_frame: u64,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub source_transforms: Vec<Mat3>,
}

pub fn analyze_chunk(
    input: &Path,
    track_id: u32,
    chunk_index: u64,
    chunk_seconds: u32,
    overlap_seconds: u32,
    options: StabilizationOptions,
    cancelled: impl Fn() -> bool,
) -> Result<Option<StabilizationChunk>, String> {
    if cancelled() {
        return Ok(None);
    }
    let mut video =
        video::VideoChunk::open(input, track_id, chunk_index, chunk_seconds, overlap_seconds)?;
    let width = video.width as i32;
    let height = video.height as i32;
    let previous = video
        .read()?
        .ok_or("video stabilization could not read the first frame")?;
    #[cfg(feature = "nvidia")]
    let mut previous = previous;
    #[cfg(feature = "nvidia")]
    let mut tracker = None;
    #[cfg(feature = "nvidia")]
    let mut nvidia =
        match flow_nvidia::NvidiaAffineEstimator::new(&previous, width as u32, height as u32) {
            Ok(estimator) => {
                tracing::debug!("Using NVIDIA Optical Flow for affine stabilization motion");
                Some(estimator)
            }
            Err(error) => {
                tracing::warn!("NVIDIA Optical Flow unavailable; using KLT fallback: {error}");
                None
            }
        };
    #[cfg(not(feature = "nvidia"))]
    let mut estimator = flow::Estimator::new(&previous)?;
    let mut motions = vec![Mat3::IDENTITY];
    loop {
        if cancelled() {
            return Ok(None);
        }
        let Some(current) = video.read()? else {
            break;
        };
        #[cfg(feature = "nvidia")]
        let nvidia_motion = nvidia
            .as_mut()
            .map(|estimator| estimator.estimate(&current));
        #[cfg(feature = "nvidia")]
        let motion = match nvidia_motion {
            Some(Ok(motion)) => motion,
            Some(Err(error)) => {
                tracing::warn!("NVIDIA Optical Flow failed; using KLT fallback: {error}");
                nvidia = None;
                estimate_affine(
                    tracker.get_or_insert_with(sparse::Tracker::default),
                    &previous,
                    &current,
                )?
            }
            None => estimate_affine(
                tracker.get_or_insert_with(sparse::Tracker::default),
                &previous,
                &current,
            )?,
        };
        #[cfg(not(feature = "nvidia"))]
        let motion = estimator.estimate(&current)?;
        motions.push(motion);
        #[cfg(feature = "nvidia")]
        {
            previous = current;
        }
    }
    if cancelled() {
        return Ok(None);
    }
    let corrections = solve_path(&motions, UVec2::new(width as u32, height as u32), options)?;
    if cancelled() {
        return Ok(None);
    }
    let central = video.central_range(corrections.len())?;
    let scale = 1.0
        / options
            .crop_ratio
            .clamp(MINIMUM_CROP_RATIO, MAXIMUM_CROP_RATIO);
    let source_transforms = corrections[central]
        .iter()
        .copied()
        .map(|correction| source_transform(correction, width, height, scale))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(StabilizationChunk {
        first_frame: video.first_frame,
        frame_rate_numerator: video.frame_rate_numerator,
        frame_rate_denominator: video.frame_rate_denominator,
        source_transforms,
    }))
}

pub fn estimate_affine(
    tracker: &mut sparse::Tracker,
    previous: &RgbaImage,
    current: &RgbaImage,
) -> Result<Mat3, String> {
    if previous.width != current.width || previous.height != current.height {
        return Err("affine stabilization frames do not match".to_string());
    }
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
    if points.len() < MINIMUM_TRACKED_FEATURES {
        return Err(format!(
            "affine stabilization found only {} trackable features",
            points.len()
        ));
    }
    let tracks = tracker.track(
        &previous_gray,
        &current_gray,
        &points,
        FlowOptions {
            window: OPTICAL_FLOW_WINDOW,
            maximum_level: OPTICAL_FLOW_PYRAMID_LEVELS,
            iterations: OPTICAL_FLOW_ITERATIONS,
            epsilon: OPTICAL_FLOW_EPSILON,
            minimum_eigenvalue: 1e-4,
        },
    )?;
    let (previous, current): (Vec<_>, Vec<_>) = tracks
        .into_iter()
        .filter(|track| track.valid)
        .map(|track| (track.previous, track.current))
        .unzip();
    if current.len() < MINIMUM_TRACKED_FEATURES {
        return Err(format!(
            "affine stabilization tracked only {} features",
            current.len()
        ));
    }
    estimate_affine_from_points(&current, &previous)
}

fn estimate_affine_from_points(
    current: &[[f32; 2]],
    previous: &[[f32; 2]],
) -> Result<Mat3, String> {
    let affine = fit_affine(
        current,
        previous,
        AffineOptions {
            reprojection_threshold: RANSAC_REPROJECTION_THRESHOLD,
            maximum_iterations: RANSAC_MAXIMUM_ITERATIONS,
            confidence: RANSAC_CONFIDENCE,
            refinement_iterations: RANSAC_REFINEMENT_ITERATIONS,
        },
    )?;
    let motion = Mat3::from_cols(
        Vec3::new(affine[0][0] as f32, affine[1][0] as f32, 0.0),
        Vec3::new(affine[0][1] as f32, affine[1][1] as f32, 0.0),
        Vec3::new(affine[0][2] as f32, affine[1][2] as f32, 1.0),
    );
    if !motion.is_finite() {
        return Err("affine stabilization estimated non-finite frame motion".to_string());
    }
    Ok(motion)
}

pub fn solve_path(
    motions: &[Mat3],
    size: UVec2,
    options: StabilizationOptions,
) -> Result<Vec<Mat3>, String> {
    let UVec2 {
        x: width,
        y: height,
    } = size;
    if motions.is_empty() {
        return Err("affine stabilization path has no frames".to_string());
    }
    if width == 0 || height == 0 || !motions.iter().all(|motion| motion.is_finite()) {
        return Err("affine stabilization path input is invalid".to_string());
    }
    if motions.len() == 1 {
        return Ok(vec![Mat3::IDENTITY]);
    }
    let crop_ratio = options
        .crop_ratio
        .clamp(MINIMUM_CROP_RATIO, MAXIMUM_CROP_RATIO);
    if !options
        .derivative_weights
        .iter()
        .all(|weight| weight.is_finite() && *weight >= 0.0)
    {
        return Err("affine stabilization derivative weights are invalid".to_string());
    }
    let mut variables = ProblemVariables::new();
    let parameters = (0..motions.len())
        .map(|_| {
            std::array::from_fn(|component| {
                variables.add(match component {
                    A | D => variable().min(0.9).max(1.1),
                    B | C => variable().min(-0.1).max(0.1),
                    _ => variable(),
                })
            })
        })
        .collect::<Vec<[Variable; 6]>>();
    let first_slack = add_slack(&mut variables, motions.len() - 1);
    let second_slack = add_slack(&mut variables, motions.len().saturating_sub(2));
    let third_slack = add_slack(&mut variables, motions.len().saturating_sub(3));
    let mut objective = Expression::from(0.0);
    for (derivative, slack) in [&first_slack, &second_slack, &third_slack]
        .into_iter()
        .enumerate()
    {
        for frame in slack {
            for component in 0..6 {
                objective += frame[component]
                    * options.derivative_weights[derivative]
                    * PARAMETER_WEIGHTS[component];
            }
        }
    }
    let mut model = variables.minimise(objective).using(highs);

    let residuals = (0..motions.len() - 1)
        .map(|frame| {
            let product = parameterized_product(motions[frame + 1], parameters[frame + 1]);
            std::array::from_fn(|component| {
                product[component].clone() - parameters[frame][component]
            })
        })
        .collect::<Vec<[Expression; 6]>>();
    for frame in 0..first_slack.len() {
        for component in 0..6 {
            let residual = residuals[frame][component].clone();
            model.add_constraint(residual.clone().leq(first_slack[frame][component]));
            model.add_constraint((-residual).leq(first_slack[frame][component]));
        }
    }
    for frame in 0..second_slack.len() {
        for component in 0..6 {
            let residual =
                residuals[frame + 1][component].clone() - residuals[frame][component].clone();
            model.add_constraint(residual.clone().leq(second_slack[frame][component]));
            model.add_constraint((-residual).leq(second_slack[frame][component]));
        }
    }
    for frame in 0..third_slack.len() {
        for component in 0..6 {
            let residual = residuals[frame + 2][component].clone()
                - residuals[frame + 1][component].clone() * 2.0
                + residuals[frame][component].clone();
            model.add_constraint(residual.clone().leq(third_slack[frame][component]));
            model.add_constraint((-residual).leq(third_slack[frame][component]));
        }
    }

    let crop_width = (f64::from(width) * crop_ratio).round();
    let crop_height = (f64::from(height) * crop_ratio).round();
    let origin_x = ((f64::from(width) - crop_width) * 0.5).round();
    let origin_y = ((f64::from(height) - crop_height) * 0.5).round();
    let crop_corners = [
        [origin_x, origin_y],
        [origin_x + crop_width, origin_y],
        [origin_x, origin_y + crop_height],
        [origin_x + crop_width, origin_y + crop_height],
    ];
    for p in &parameters {
        model.add_constraint((p[B] + p[C]).geq(-MAXIMUM_SHEAR));
        model.add_constraint((p[B] + p[C]).leq(MAXIMUM_SHEAR));
        model.add_constraint((p[A] - p[D]).geq(-MAXIMUM_ASPECT_CHANGE));
        model.add_constraint((p[A] - p[D]).leq(MAXIMUM_ASPECT_CHANGE));
        for [x, y] in crop_corners {
            let projected_x = p[DX] + p[A] * x + p[B] * y;
            let projected_y = p[DY] + p[C] * x + p[D] * y;
            model.add_constraint(projected_x.clone().geq(0.0));
            model.add_constraint(projected_x.leq(f64::from(width)));
            model.add_constraint(projected_y.clone().geq(0.0));
            model.add_constraint(projected_y.leq(f64::from(height)));
        }
    }

    let solution = model
        .solve()
        .map_err(|error| format!("solve affine stabilization path: {error}"))?;
    if !matches!(solution.status(), SolutionStatus::Optimal) {
        return Err(format!(
            "affine stabilization solver stopped with {:?}",
            solution.status()
        ));
    }
    parameters
        .iter()
        .map(|p| {
            inverse_affine([
                solution.value(p[A]),
                solution.value(p[B]),
                solution.value(p[DX]),
                solution.value(p[C]),
                solution.value(p[D]),
                solution.value(p[DY]),
            ])
        })
        .collect()
}

fn add_slack(variables: &mut ProblemVariables, frames: usize) -> Vec<[Variable; 6]> {
    (0..frames)
        .map(|_| std::array::from_fn(|_| variables.add(variable().min(0.0))))
        .collect()
}

fn parameterized_product(motion: Mat3, p: [Variable; 6]) -> [Expression; 6] {
    let a = f64::from(motion.x_axis.x);
    let b = f64::from(motion.y_axis.x);
    let dx = f64::from(motion.z_axis.x);
    let c = f64::from(motion.x_axis.y);
    let d = f64::from(motion.y_axis.y);
    let dy = f64::from(motion.z_axis.y);
    [
        dx * p[A] + dy * p[B] + p[DX],
        dx * p[C] + dy * p[D] + p[DY],
        a * p[A] + c * p[B],
        b * p[A] + d * p[B],
        a * p[C] + c * p[D],
        b * p[C] + d * p[D],
    ]
    .map(IntoAffineExpression::into_expression)
}

fn inverse_affine([a, b, dx, c, d, dy]: [f64; 6]) -> Result<Mat3, String> {
    let correction = Mat3::from_cols(
        Vec3::new(a as f32, c as f32, 0.0),
        Vec3::new(b as f32, d as f32, 0.0),
        Vec3::new(dx as f32, dy as f32, 1.0),
    );
    let determinant = correction.determinant();
    if !determinant.is_finite() || determinant.abs() <= MINIMUM_DETERMINANT {
        return Err("affine stabilization produced a singular correction".to_string());
    }
    let transform = correction.inverse();
    if !transform.is_finite() {
        return Err("affine stabilization produced a non-finite correction".to_string());
    }
    Ok(transform)
}

fn source_transform(correction: Mat3, width: i32, height: i32, scale: f64) -> Result<Mat3, String> {
    let center = Vec2::new(width as f32, height as f32) * 0.5;
    let forward = Mat3::from_translation(center)
        * Mat3::from_scale(Vec2::splat(scale as f32))
        * Mat3::from_translation(-center)
        * correction;
    let determinant = forward.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return Err("affine stabilization produced a singular source transform".to_string());
    }
    Ok(forward.inverse())
}
