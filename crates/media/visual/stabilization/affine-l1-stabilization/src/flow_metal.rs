use objc2::{
    AnyThread,
    rc::Retained,
    runtime::{AnyObject, NSObjectProtocol, ProtocolObject},
};
use objc2_core_ml::{MLComputeDeviceProtocol, MLGPUComputeDevice};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_TwoComponent32Float, kCVReturnSuccess,
};
use objc2_foundation::{NSArray, NSDictionary};
use objc2_vision::{
    VNGenerateOpticalFlowRequest, VNGenerateOpticalFlowRequestComputationAccuracy,
    VNGenerateOpticalFlowRequestRevision2, VNImageOption, VNImageRequestHandler, VNRequest,
};
use shrimply_math_geometry::Vec2;
use shrimply_math_geometry::optical_flow::{OpticalFlowField, regular_grid_size};
use std::ptr::NonNull;

pub fn estimate_rgba(
    source: &[u8],
    target: &[u8],
    width: u32,
    height: u32,
) -> Result<OpticalFlowField, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|count| usize::try_from(count).ok())
        .and_then(|count| count.checked_mul(4))
        .ok_or("Morph endpoint size overflow")?;
    if source.len() != expected || target.len() != expected {
        return Err("Morph endpoint buffer size does not match the canvas".into());
    }
    objc2::rc::autoreleasepool(|_| {
        let source = input_buffer(source, width, height)?;
        let target = input_buffer(target, width, height)?;
        // Vision reports handler-image -> targeted-image motion, matching the
        // source -> target convention used by the shared Morph presentation.
        let forward = estimate_direction(&source, &target, width, height)?;
        let backward = estimate_direction(&target, &source, width, height)?;
        let grid_size = regular_grid_size(width, height)?;
        OpticalFlowField::new(grid_size, forward, backward)
    })
}

fn input_buffer(bytes: &[u8], width: u32, height: u32) -> Result<Retained<CVPixelBuffer>, String> {
    let mut raw = std::ptr::null_mut();
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width as usize,
            height as usize,
            kCVPixelFormatType_32BGRA,
            None,
            NonNull::from(&mut raw),
        )
    };
    if status != kCVReturnSuccess {
        return Err(format!(
            "Apple optical flow could not allocate an input buffer ({status})"
        ));
    }
    let buffer =
        unsafe { Retained::from_raw(raw) }.ok_or("Apple optical flow returned no input buffer")?;
    let flags = CVPixelBufferLockFlags::empty();
    let status = unsafe { CVPixelBufferLockBaseAddress(&buffer, flags) };
    if status != kCVReturnSuccess {
        return Err(format!(
            "Apple optical flow could not lock an input buffer ({status})"
        ));
    }
    let row_bytes = CVPixelBufferGetBytesPerRow(&buffer);
    let packed_row_bytes = width as usize * 4;
    let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
    if base.is_null() || row_bytes < packed_row_bytes {
        unsafe { CVPixelBufferUnlockBaseAddress(&buffer, flags) };
        return Err("Apple optical-flow input buffer has invalid storage".into());
    }
    for row in 0..height as usize {
        // Core Video allocates BGRA input buffers; renderer readback is RGBA.
        // Convert only the packed pixels, leaving Core Video's row padding alone.
        let source = &bytes[row * packed_row_bytes..(row + 1) * packed_row_bytes];
        let destination =
            unsafe { std::slice::from_raw_parts_mut(base.add(row * row_bytes), packed_row_bytes) };
        for (rgba, bgra) in source.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
            bgra.copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
    }
    let status = unsafe { CVPixelBufferUnlockBaseAddress(&buffer, flags) };
    if status != kCVReturnSuccess {
        return Err(format!(
            "Apple optical flow could not unlock an input buffer ({status})"
        ));
    }
    Ok(buffer)
}

fn estimate_direction(
    source: &CVPixelBuffer,
    target: &CVPixelBuffer,
    width: u32,
    height: u32,
) -> Result<Vec<Vec2>, String> {
    let options = NSDictionary::<VNImageOption, AnyObject>::new();
    let request = unsafe {
        VNGenerateOpticalFlowRequest::initWithTargetedCVPixelBuffer_options(
            VNGenerateOpticalFlowRequest::alloc(),
            target,
            &options,
        )
    };
    unsafe {
        request.setRevision(VNGenerateOpticalFlowRequestRevision2);
        request.setComputationAccuracy(VNGenerateOpticalFlowRequestComputationAccuracy::High);
        request.setOutputPixelFormat(kCVPixelFormatType_TwoComponent32Float);
    }
    require_gpu(&request)?;
    let handler = unsafe {
        VNImageRequestHandler::initWithCVPixelBuffer_options(
            VNImageRequestHandler::alloc(),
            source,
            &options,
        )
    };
    let requests = NSArray::<VNRequest>::from_slice(&[&request]);
    handler
        .performRequests_error(&requests)
        .map_err(|error| format!("Apple hardware optical flow failed: {error}"))?;
    let results = unsafe { request.results() }
        .ok_or("Apple hardware optical flow returned no result list")?;
    let observation = results
        .firstObject()
        .ok_or("Apple hardware optical flow returned no result")?;
    let flow = unsafe { observation.pixelBuffer() };
    sample_flow(&flow, width, height)
}

fn require_gpu(request: &VNGenerateOpticalFlowRequest) -> Result<(), String> {
    if !request.respondsToSelector(objc2::sel!(supportedComputeStageDevicesAndReturnError:)) {
        return Err("Apple GPU optical flow is unavailable on this macOS version".into());
    }
    let supported = unsafe { request.supportedComputeStageDevicesAndReturnError() }
        .map_err(|error| format!("Could not query Apple optical-flow hardware: {error}"))?;
    if supported.is_empty() {
        return Err("Apple optical flow reported no compute stages".into());
    }
    for stage in supported.keys() {
        let devices = supported
            .objectForKey(&stage)
            .ok_or("Apple optical flow omitted devices for a compute stage")?;
        let gpu = devices
            .iter()
            .find(|device| {
                let protocol: &ProtocolObject<dyn MLComputeDeviceProtocol> = device;
                let object: &AnyObject = protocol.as_ref();
                object.downcast_ref::<MLGPUComputeDevice>().is_some()
            })
            .ok_or("Apple optical flow has a compute stage without a GPU implementation")?;
        unsafe { request.setComputeDevice_forComputeStage(Some(&gpu), &stage) };
    }
    Ok(())
}

fn sample_flow(buffer: &CVPixelBuffer, width: u32, height: u32) -> Result<Vec<Vec2>, String> {
    if CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_TwoComponent32Float
        || CVPixelBufferGetWidth(buffer) != width as usize
        || CVPixelBufferGetHeight(buffer) != height as usize
    {
        return Err("Apple optical flow returned an unexpected pixel-buffer layout".into());
    }
    let flags = CVPixelBufferLockFlags::ReadOnly;
    let status = unsafe { CVPixelBufferLockBaseAddress(buffer, flags) };
    if status != kCVReturnSuccess {
        return Err(format!(
            "Could not lock Apple optical-flow output ({status})"
        ));
    }
    let result = (|| {
        let row_bytes = CVPixelBufferGetBytesPerRow(buffer);
        let component_bytes = std::mem::size_of::<[f32; 2]>();
        if row_bytes < width as usize * component_bytes {
            return Err("Apple optical-flow output rows are truncated".into());
        }
        let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
        if base.is_null() {
            return Err("Apple optical-flow output has no storage".into());
        }
        let grid = regular_grid_size(width, height)?;
        let mut values = Vec::with_capacity((grid.x * grid.y) as usize);
        for row in 0..grid.y {
            let y = row * (height - 1) / (grid.y - 1);
            for column in 0..grid.x {
                let x = column * (width - 1) / (grid.x - 1);
                let address = unsafe {
                    base.add(y as usize * row_bytes + x as usize * component_bytes)
                        .cast::<[f32; 2]>()
                };
                let value = unsafe { address.read_unaligned() };
                // Revision 2 reports components in its half-resolution analysis
                // coordinates even though Vision expands the result buffer to the
                // full input dimensions. Shared motion uses source-pixel offsets.
                values.push(Vec2::from_array(value) * 2.0);
            }
        }
        Ok(values)
    })();
    let unlock = unsafe { CVPixelBufferUnlockBaseAddress(buffer, flags) };
    if unlock != kCVReturnSuccess {
        return Err(format!(
            "Could not unlock Apple optical-flow output ({unlock})"
        ));
    }
    result
}
