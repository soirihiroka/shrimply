use super::estimate_affine_from_points;
use glam::{Mat3, UVec2, Vec2};
use opencv::core::{Mat, MatTraitConst, Point2f, Vector};
use opencv::imgproc;
use opencv::prelude::MatTraitConstManual;
use shrimply_gpu_cuda::{CudaContext, CudaStream};
use shrimply_gpu_cuda_memory::GpuBuffer as DeviceBuffer;
use shrimply_nvidia_optical_flow::{
    FlowField, FlowVector, OpticalFlow, OutputGrid, Quality, Settings as OpticalFlowSettings,
};
use std::sync::Arc;

const NVIDIA_FLOW_SAMPLE_SPACING: usize = 16;
const NVIDIA_FLOW_COST_KEEP_RATIO: f32 = 0.7;
const NVIDIA_FLOW_FIXED_POINT_SCALE: f32 = 32.0;
const NVIDIA_FLOW_MINIMUM_TRACKS: usize = 12;
const NVIDIA_FLOW_CYCLE_ERROR_PIXELS: f32 = 1.5;
const NVIDIA_FLOW_CYCLE_ERROR_MOTION_RATIO: f32 = 0.05;

pub(super) struct NvidiaAffineEstimator {
    optical_flow: OpticalFlow,
    previous: DeviceBuffer<u32>,
    stream: Arc<CudaStream>,
    first_pair: bool,
}

impl NvidiaAffineEstimator {
    pub(super) fn new(previous: &Mat, width: u32, height: u32) -> Result<Self, String> {
        let context = CudaContext::new(shrimply_gpu_cuda::selected_device())
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

    pub(super) fn estimate(&mut self, current: &Mat) -> Result<Mat3, String> {
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
    let mut output = shrimply_gpu_cuda_memory::global().allocate_buffer(
        stream,
        pixels.len(),
        shrimply_gpu_cuda_memory::AllocationClass::Transient,
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
