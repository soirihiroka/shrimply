use glam::{Mat3, UVec2, Vec2, Vec3};
use good_lp::{
    Expression, IntoAffineExpression, ProblemVariables, Solution, SolutionStatus, SolverModel,
    Variable, highs, variable,
};
use std::path::Path;
use std::sync::Arc;

use opencv::core::{
    self, Mat, MatTraitConst, Point2f, Size, TermCriteria, TermCriteria_Type, Vector,
};
use opencv::prelude::MatTraitConstManual;
use opencv::prelude::*;
use opencv::{features, geometry, imgproc, video, videoio};
use shrimply_cuda::{CudaContext, CudaStream};
use shrimply_gpu_memory::GpuBuffer as DeviceBuffer;
use shrimply_math_core::{Time, fraction_as_u32_ratio, frame_rate_from_f64};
use shrimply_nvidia_optical_flow::{
    FlowField, FlowVector, OpticalFlow, OutputGrid, Quality, Settings as OpticalFlowSettings,
};

const MAXIMUM_FEATURES: i32 = 200;
const FEATURE_QUALITY: f64 = 0.01;
const MINIMUM_FEATURE_DISTANCE: f64 = 30.0;
const FEATURE_BLOCK_SIZE: i32 = 3;
const OPTICAL_FLOW_WINDOW: i32 = 20;
const OPTICAL_FLOW_PYRAMID_LEVELS: i32 = 3;
const OPTICAL_FLOW_ITERATIONS: i32 = 10;
const OPTICAL_FLOW_EPSILON: f64 = 0.03;
const MINIMUM_TRACKED_FEATURES: usize = 4;
const RANSAC_REPROJECTION_THRESHOLD: f64 = 3.0;
const RANSAC_MAXIMUM_ITERATIONS: usize = 2_000;
const RANSAC_CONFIDENCE: f64 = 0.99;
const RANSAC_REFINEMENT_ITERATIONS: usize = 10;
const NVIDIA_FLOW_SAMPLE_SPACING: usize = 16;
const NVIDIA_FLOW_COST_KEEP_RATIO: f32 = 0.7;
const NVIDIA_FLOW_FIXED_POINT_SCALE: f32 = 32.0;
const NVIDIA_FLOW_MINIMUM_TRACKS: usize = 12;
const NVIDIA_FLOW_CYCLE_ERROR_PIXELS: f32 = 1.5;
const NVIDIA_FLOW_CYCLE_ERROR_MOTION_RATIO: f32 = 0.05;
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

struct NvidiaAffineEstimator {
    optical_flow: OpticalFlow,
    previous: DeviceBuffer<u32>,
    stream: Arc<CudaStream>,
    first_pair: bool,
}

impl NvidiaAffineEstimator {
    fn new(previous: &Mat, width: u32, height: u32) -> Result<Self, String> {
        let context = CudaContext::new(0)
            .map_err(|error| format!("create CUDA context for optical flow: {error:?}"))?;
        let stream = context
            .new_stream()
            .map_err(|error| format!("create CUDA optical flow stream: {error:?}"))?;
        let settings = |output_grid| OpticalFlowSettings {
            quality: Quality::Quality,
            output_grid,
            temporal_hints: true,
        };
        let optical_flow = OpticalFlow::new(
            &context,
            &stream,
            width,
            height,
            settings(OutputGrid::TwoByTwo),
        )
        .or_else(|_| {
            OpticalFlow::new(
                &context,
                &stream,
                width,
                height,
                settings(OutputGrid::FourByFour),
            )
        })?;
        let previous = upload_optical_flow_frame(&stream, previous)?;
        Ok(Self {
            optical_flow,
            previous,
            stream,
            first_pair: true,
        })
    }

    fn estimate(&mut self, current: &Mat) -> Result<Mat3, String> {
        let image_size = UVec2::new(current.cols() as u32, current.rows() as u32);
        let current = upload_optical_flow_frame(&self.stream, current)?;
        let field = self.optical_flow.estimate(
            self.previous.cu_deviceptr(),
            current.cu_deviceptr(),
            self.first_pair,
        )?;
        let motion = estimate_affine_from_flow(&field, image_size)?;
        self.previous = current;
        self.first_pair = false;
        Ok(motion)
    }
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
    let mut video = open_video(input, track_id)?;
    let width = checked_property(&video, videoio::CAP_PROP_FRAME_WIDTH, "width")?;
    let height = checked_property(&video, videoio::CAP_PROP_FRAME_HEIGHT, "height")?;
    let fps = video
        .get(videoio::CAP_PROP_FPS)
        .map_err(|error| error.to_string())?;
    if !fps.is_finite() || fps <= 0.0 {
        return Err("video stabilization requires a positive frame rate".to_string());
    }
    let frame_rate = frame_rate_from_f64(fps)
        .ok_or_else(|| "video stabilization frame rate is out of range".to_string())?;
    let (frame_rate_numerator, frame_rate_denominator) = fraction_as_u32_ratio(frame_rate)
        .ok_or_else(|| "video stabilization frame rate is out of range".to_string())?;
    let central_start =
        Time::from_seconds_u64(chunk_index.saturating_mul(u64::from(chunk_seconds)))
            .as_frame(frame_rate);
    let central_end = Time::from_seconds_u64(
        chunk_index
            .saturating_add(1)
            .saturating_mul(u64::from(chunk_seconds)),
    )
    .as_frame_ceil(frame_rate);
    let overlap_frames =
        Time::from_seconds_u64(u64::from(overlap_seconds)).as_frame_ceil(frame_rate);
    let analysis_start = central_start.saturating_sub(overlap_frames);
    let analysis_end = central_end.saturating_add(overlap_frames);
    video
        .set(videoio::CAP_PROP_POS_FRAMES, analysis_start as f64)
        .map_err(|error| error.to_string())?;

    let mut previous = Mat::default();
    if !video
        .read(&mut previous)
        .map_err(|error| error.to_string())?
        || previous.empty()
    {
        return Err("video stabilization could not read the first frame".to_string());
    }
    let mut nvidia = match NvidiaAffineEstimator::new(&previous, width as u32, height as u32) {
        Ok(estimator) => {
            tracing::debug!("Using NVIDIA Optical Flow for affine stabilization motion");
            Some(estimator)
        }
        Err(error) => {
            tracing::warn!("NVIDIA Optical Flow unavailable; using KLT fallback: {error}");
            None
        }
    };
    let mut motions = vec![Mat3::IDENTITY];
    while analysis_start.saturating_add(motions.len() as u64) < analysis_end {
        if cancelled() {
            return Ok(None);
        }
        let mut current = Mat::default();
        if !video
            .read(&mut current)
            .map_err(|error| error.to_string())?
            || current.empty()
        {
            break;
        }
        let nvidia_motion = nvidia
            .as_mut()
            .map(|estimator| estimator.estimate(&current));
        let motion = match nvidia_motion {
            Some(Ok(motion)) => motion,
            Some(Err(error)) => {
                tracing::warn!("NVIDIA Optical Flow failed; using KLT fallback: {error}");
                nvidia = None;
                estimate_affine(&previous, &current)?
            }
            None => estimate_affine(&previous, &current)?,
        };
        motions.push(motion);
        previous = current;
    }
    if cancelled() {
        return Ok(None);
    }
    let corrections = solve_path(&motions, UVec2::new(width as u32, height as u32), options)?;
    if cancelled() {
        return Ok(None);
    }
    let first = central_start.saturating_sub(analysis_start) as usize;
    let end = (central_end.saturating_sub(analysis_start) as usize).min(corrections.len());
    if first >= end {
        return Err("video stabilization chunk contains no frames".to_string());
    }
    let scale = 1.0
        / options
            .crop_ratio
            .clamp(MINIMUM_CROP_RATIO, MAXIMUM_CROP_RATIO);
    let source_transforms = corrections[first..end]
        .iter()
        .copied()
        .map(|correction| source_transform(correction, width, height, scale))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(StabilizationChunk {
        first_frame: central_start,
        frame_rate_numerator,
        frame_rate_denominator,
        source_transforms,
    }))
}

pub fn estimate_affine(previous: &Mat, current: &Mat) -> Result<Mat3, String> {
    let previous_size = previous.size().map_err(|error| error.to_string())?;
    let current_size = current.size().map_err(|error| error.to_string())?;
    if previous.empty() || current.empty() || previous_size != current_size {
        return Err("affine stabilization frames do not match".to_string());
    }
    let mut previous_gray = Mat::default();
    let mut current_gray = Mat::default();
    imgproc::cvt_color_def(previous, &mut previous_gray, imgproc::COLOR_BGR2GRAY)
        .map_err(|error| error.to_string())?;
    imgproc::cvt_color_def(current, &mut current_gray, imgproc::COLOR_BGR2GRAY)
        .map_err(|error| error.to_string())?;

    let mut previous_points = Vector::<Point2f>::new();
    features::good_features_to_track(
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
    if previous_points.len() < MINIMUM_TRACKED_FEATURES {
        return Err(format!(
            "affine stabilization found only {} trackable features",
            previous_points.len()
        ));
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
            i32::from(TermCriteria_Type::COUNT) + i32::from(TermCriteria_Type::EPS),
            OPTICAL_FLOW_ITERATIONS,
            OPTICAL_FLOW_EPSILON,
        )
        .map_err(|error| error.to_string())?,
        0,
        1e-4,
    )
    .map_err(|error| error.to_string())?;

    let mut valid_previous = Vector::<Point2f>::new();
    let mut valid_current = Vector::<Point2f>::new();
    for index in 0..status.len() {
        if status.get(index).map_err(|error| error.to_string())? != 0 {
            valid_previous.push(
                previous_points
                    .get(index)
                    .map_err(|error| error.to_string())?,
            );
            valid_current.push(
                current_points
                    .get(index)
                    .map_err(|error| error.to_string())?,
            );
        }
    }
    if valid_current.len() < MINIMUM_TRACKED_FEATURES {
        return Err(format!(
            "affine stabilization tracked only {} features",
            valid_current.len()
        ));
    }

    estimate_affine_from_points(&valid_current, &valid_previous)
}

fn estimate_affine_from_points(
    current: &Vector<Point2f>,
    previous: &Vector<Point2f>,
) -> Result<Mat3, String> {
    let mut inliers = Mat::default();
    let affine = geometry::estimate_affine_2d(
        current,
        previous,
        &mut inliers,
        geometry::RANSAC,
        RANSAC_REPROJECTION_THRESHOLD,
        RANSAC_MAXIMUM_ITERATIONS,
        RANSAC_CONFIDENCE,
        RANSAC_REFINEMENT_ITERATIONS,
    )
    .map_err(|error| error.to_string())?;
    if affine.empty() || affine.rows() != 2 || affine.cols() != 3 {
        return Err("affine stabilization could not estimate frame motion".to_string());
    }
    let mut affine_f64 = Mat::default();
    affine
        .convert_to(&mut affine_f64, core::CV_64F, 1.0, 0.0)
        .map_err(|error| error.to_string())?;
    let values = affine_f64
        .data_typed::<f64>()
        .map_err(|error| error.to_string())?;
    let motion = Mat3::from_cols(
        Vec3::new(values[0] as f32, values[3] as f32, 0.0),
        Vec3::new(values[1] as f32, values[4] as f32, 0.0),
        Vec3::new(values[2] as f32, values[5] as f32, 1.0),
    );
    if !motion.is_finite() {
        return Err("affine stabilization estimated non-finite frame motion".to_string());
    }
    Ok(motion)
}

fn upload_optical_flow_frame(
    stream: &CudaStream,
    frame: &Mat,
) -> Result<DeviceBuffer<u32>, String> {
    let mut rgba = Mat::default();
    imgproc::cvt_color_def(frame, &mut rgba, imgproc::COLOR_BGR2RGBA)
        .map_err(|error| error.to_string())?;
    let bytes = rgba.data_bytes().map_err(|error| error.to_string())?;
    let expected = frame.cols() as usize * frame.rows() as usize * 4;
    if bytes.len() != expected {
        return Err("NVIDIA optical flow input is not tightly packed".to_string());
    }
    let pixels = bytes
        .chunks_exact(4)
        .map(|pixel| u32::from_le_bytes(pixel.try_into().expect("four-byte RGBA pixel")))
        .collect::<Vec<_>>();
    let mut output = shrimply_gpu_memory::global().allocate_buffer(
        stream,
        pixels.len(),
        shrimply_gpu_memory::AllocationClass::Transient,
        "stabilization optical-flow input",
    )?;
    output
        .copy_from_host(stream, &pixels)
        .map_err(|error| format!("upload NVIDIA optical flow input: {error:?}"))?;
    Ok(output)
}

fn estimate_affine_from_flow(field: &FlowField, size: UVec2) -> Result<Mat3, String> {
    let UVec2 {
        x: width,
        y: height,
    } = size;
    if field.width == 0 || field.height == 0 {
        return Err("NVIDIA optical flow returned an empty field".to_string());
    }
    let step = NVIDIA_FLOW_SAMPLE_SPACING
        .div_ceil(field.grid_size.max(1) as usize)
        .max(1);
    let forward_limit = flow_cost_limit(&field.forward_cost, field.width, field.height, step);
    let backward_limit = flow_cost_limit(&field.backward_cost, field.width, field.height, step);
    let mut previous = Vector::<Point2f>::new();
    let mut current = Vector::<Point2f>::new();
    for flow_y in (0..field.height).step_by(step) {
        for flow_x in (0..field.width).step_by(step) {
            let index = flow_y * field.width + flow_x;
            if field.forward_cost[index] > forward_limit {
                continue;
            }
            let x = (flow_x as f32 + 0.5) * field.grid_size as f32;
            let y = (flow_y as f32 + 0.5) * field.grid_size as f32;
            if x > width.saturating_sub(1) as f32 || y > height.saturating_sub(1) as f32 {
                continue;
            }
            let forward = decode_flow(field.forward[index]);
            let destination = Vec2::new(x, y) + forward;
            if destination.x < 0.0
                || destination.y < 0.0
                || destination.x > width.saturating_sub(1) as f32
                || destination.y > height.saturating_sub(1) as f32
            {
                continue;
            }
            let backward_x = ((destination.x / field.grid_size as f32) - 0.5).round() as isize;
            let backward_y = ((destination.y / field.grid_size as f32) - 0.5).round() as isize;
            if backward_x < 0
                || backward_y < 0
                || backward_x >= field.width as isize
                || backward_y >= field.height as isize
            {
                continue;
            }
            let backward_index = backward_y as usize * field.width + backward_x as usize;
            if field.backward_cost[backward_index] > backward_limit {
                continue;
            }
            let backward = decode_flow(field.backward[backward_index]);
            let cycle_error = (forward + backward).length();
            let motion = forward.length();
            if cycle_error
                > NVIDIA_FLOW_CYCLE_ERROR_PIXELS + motion * NVIDIA_FLOW_CYCLE_ERROR_MOTION_RATIO
            {
                continue;
            }
            previous.push(Point2f::new(x, y));
            current.push(Point2f::new(destination.x, destination.y));
        }
    }
    if current.len() < NVIDIA_FLOW_MINIMUM_TRACKS {
        return Err(format!(
            "NVIDIA optical flow retained only {} reliable tracks",
            current.len()
        ));
    }
    estimate_affine_from_points(&current, &previous)
}

fn flow_cost_limit(costs: &[u8], width: usize, height: usize, step: usize) -> u8 {
    let mut sampled = (0..height)
        .step_by(step)
        .flat_map(|y| (0..width).step_by(step).map(move |x| costs[y * width + x]))
        .collect::<Vec<_>>();
    sampled.sort_unstable();
    let index =
        ((sampled.len().saturating_sub(1)) as f32 * NVIDIA_FLOW_COST_KEEP_RATIO).round() as usize;
    sampled.get(index).copied().unwrap_or(u8::MAX)
}

fn decode_flow(flow: FlowVector) -> Vec2 {
    Vec2::new(
        f32::from(flow.x) / NVIDIA_FLOW_FIXED_POINT_SCALE,
        f32::from(flow.y) / NVIDIA_FLOW_FIXED_POINT_SCALE,
    )
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

fn open_video(path: &Path, track_id: u32) -> Result<videoio::VideoCapture, String> {
    let path = path
        .to_str()
        .ok_or_else(|| "video stabilization input path is not UTF-8".to_string())?;
    let video = if track_id == 0 {
        videoio::VideoCapture::from_file(path, videoio::CAP_ANY)
    } else {
        let track_id = i32::try_from(track_id)
            .map_err(|_| "video stabilization stream index is too large".to_string())?;
        videoio::VideoCapture::from_file_with_params(
            path,
            videoio::CAP_FFMPEG,
            &Vector::from_slice(&[videoio::CAP_PROP_VIDEO_STREAM, track_id]),
        )
    }
    .map_err(|error| error.to_string())?;
    if !video.is_opened().map_err(|error| error.to_string())? {
        return Err(format!("OpenCV could not open {path}"));
    }
    Ok(video)
}

fn checked_property(
    video: &videoio::VideoCapture,
    property: i32,
    name: &str,
) -> Result<i32, String> {
    let value = video.get(property).map_err(|error| error.to_string())?;
    if !value.is_finite() || value < 1.0 || value > f64::from(i32::MAX) {
        return Err(format!("video stabilization has an invalid {name}"));
    }
    Ok(value.round() as i32)
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
