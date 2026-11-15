use std::path::Path;

use glam::{Mat3, Vec2};
#[cfg(not(target_os = "android"))]
mod flow_klt;
use shrimply_affine_l1_stabilization::video::VideoChunk;

const MINIMUM_CROP_RATIO: f32 = 0.1;
const MAXIMUM_CROP_RATIO: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdaptiveWeights {
    Original,
    Flipped,
    ConstantHigh,
    ConstantLow,
}

#[derive(Clone, Copy, Debug)]
pub struct StabilizationOptions {
    pub crop_ratio: f32,
    pub mesh_rows: u32,
    pub mesh_columns: u32,
    pub temporal_smoothing_radius: u32,
    pub optimization_iterations: u32,
    pub adaptive_weights: AdaptiveWeights,
}

#[derive(Clone, Debug)]
pub struct StabilizationChunk {
    pub first_frame: u64,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub source_offsets: Vec<Vec<Vec2>>,
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
    let mut capture =
        VideoChunk::open(input, track_id, chunk_index, chunk_seconds, overlap_seconds)?;
    let width = capture.width;
    let height = capture.height;
    let previous = capture
        .read()?
        .ok_or("MeshFlow could not read the first frame")?;
    #[cfg(not(target_os = "android"))]
    let mut previous = previous;
    #[cfg(target_os = "android")]
    let mut estimator = shrimply_affine_l1_stabilization::flow::Estimator::new(&previous)?;
    #[cfg(not(target_os = "android"))]
    let mut tracker = shrimply_affine_l1_stabilization::sparse::Tracker::default();
    let grid_width = options.mesh_columns.clamp(2, 32) + 1;
    let grid_height = options.mesh_rows.clamp(2, 32) + 1;
    let vertices = grid_width as usize * grid_height as usize;
    let mut paths = vec![vec![Vec2::ZERO; vertices]];
    let mut transforms = vec![Mat3::IDENTITY];
    loop {
        if cancelled() {
            return Ok(None);
        }
        let Some(current) = capture.read()? else {
            break;
        };
        #[cfg(not(target_os = "android"))]
        let tracks = flow_klt::tracked_features(&mut tracker, &previous, &current)?;
        #[cfg(target_os = "android")]
        let tracks = estimator.tracks(&current)?;
        let (transform, residual, scene_cut) = shrimply_math_geometry::stabilization_mesh_motion(
            &tracks,
            grid_width as usize,
            grid_height as usize,
            width,
            height,
        );
        let global = shrimply_math_geometry::stabilization_forward_warp_offsets(
            transform,
            grid_width as usize,
            grid_height as usize,
            width,
            height,
        );
        let prior = paths.last().expect("MeshFlow path has its identity frame");
        paths.push(
            prior
                .iter()
                .zip(global.iter().zip(residual))
                .map(|(prior, (global, residual))| {
                    if scene_cut {
                        *prior
                    } else {
                        *prior + *global + residual
                    }
                })
                .collect(),
        );
        transforms.push(transform);
        #[cfg(not(target_os = "android"))]
        {
            previous = current;
        }
    }
    if paths.is_empty() {
        return Err("MeshFlow chunk contains no frames".to_string());
    }
    let weights = transforms
        .iter()
        .map(|transform| adaptive_weight(*transform, width, height, options.adaptive_weights))
        .collect::<Vec<_>>();
    let Some(smoothed) = shrimply_math_geometry::stabilization_smooth_mesh_paths(
        &paths,
        &weights,
        options.temporal_smoothing_radius as usize,
        options.optimization_iterations as usize,
        &cancelled,
    ) else {
        return Ok(None);
    };
    let central = capture.central_range(paths.len())?;
    let crop_ratio = options
        .crop_ratio
        .clamp(MINIMUM_CROP_RATIO, MAXIMUM_CROP_RATIO);
    let source_offsets = paths[central.clone()]
        .iter()
        .zip(&smoothed[central])
        .map(|(observed, smooth)| {
            observed
                .iter()
                .zip(smooth)
                .enumerate()
                .map(|(index, (observed, smooth))| {
                    let column = index % grid_width as usize;
                    let row = index / grid_width as usize;
                    let x = column as f32 * width.saturating_sub(1) as f32
                        / grid_width.saturating_sub(1) as f32;
                    let y = row as f32 * height.saturating_sub(1) as f32
                        / grid_height.saturating_sub(1) as f32;
                    let center = Vec2::new(
                        width.saturating_sub(1) as f32 * 0.5,
                        height.saturating_sub(1) as f32 * 0.5,
                    );
                    *observed - *smooth + (crop_ratio - 1.0) * (Vec2::new(x, y) - center)
                })
                .collect()
        })
        .collect();
    Ok(Some(StabilizationChunk {
        first_frame: capture.first_frame,
        frame_rate_numerator: capture.frame_rate_numerator,
        frame_rate_denominator: capture.frame_rate_denominator,
        grid_width,
        grid_height,
        source_offsets,
    }))
}

fn adaptive_weight(transform: Mat3, width: u32, height: u32, definition: AdaptiveWeights) -> f32 {
    match definition {
        AdaptiveWeights::ConstantHigh => 100.0,
        AdaptiveWeights::ConstantLow => 1.0,
        AdaptiveWeights::Original | AdaptiveWeights::Flipped => {
            let translation = (transform.z_axis.x / width.max(1) as f32)
                .hypot(transform.z_axis.y / height.max(1) as f32);
            let scale = transform.x_axis.truncate().length().max(f32::EPSILON);
            let candidate_motion = -1.93 * translation + 0.95;
            let candidate_affine = 5.83 * scale
                + if matches!(definition, AdaptiveWeights::Original) {
                    4.88
                } else {
                    -4.88
                };
            candidate_motion.min(candidate_affine).max(0.0)
        }
    }
}
