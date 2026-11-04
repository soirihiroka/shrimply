use shrimply_denoise_oidn::{Device, Filter, Image, Images, Quality};
use shrimply_gpu_cuda::{CudaContext, DeviceBuffer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const WIDTH: u32 = 1920;
    const HEIGHT: u32 = 1080;
    const ITERATIONS: u32 = 5;
    let context = CudaContext::new(0)?;
    let stream = context.new_stream()?;
    let count = (WIDTH * HEIGHT) as usize;
    let mut seed = 1u32;
    let reference = (0..count)
        .map(|pixel| {
            let value = if pixel % (WIDTH as usize) < WIDTH as usize / 2 {
                0.5
            } else {
                2.0
            };
            [value, value, value, 0.375]
        })
        .collect::<Vec<_>>();
    let noisy = reference
        .iter()
        .map(|pixel| {
            let mut output = *pixel;
            for channel in &mut output[..3] {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *channel += (seed as f32 / u32::MAX as f32 - 0.5) * 0.5;
            }
            output
        })
        .collect::<Vec<_>>();
    let input = DeviceBuffer::from_host(&stream, &noisy)?;
    // Initialize alpha too: FLOAT3 filtering must preserve it.
    let output = DeviceBuffer::from_host(&stream, &noisy)?;
    let albedo = DeviceBuffer::from_host(&stream, &reference)?;
    let normal = DeviceBuffer::from_host(&stream, &vec![[0.0f32, 0.0, 1.0, 1.0]; count])?;
    stream.synchronize()?;
    let device = unsafe { Device::for_cuda(context.ordinal(), stream.cu_stream().cast()) }?;
    let mut filter = Filter::new(device.clone())?;
    for (name, quality) in [
        ("interactive", Quality::Interactive),
        ("final", Quality::Final),
    ] {
        let mut run = || unsafe {
            filter.execute(
                Images {
                    color: Image::Cuda(input.cu_deviceptr()),
                    output: Image::Cuda(output.cu_deviceptr()),
                    albedo: Some(Image::Cuda(albedo.cu_deviceptr())),
                    normal: Some(Image::Cuda(normal.cu_deviceptr())),
                    size: (WIDTH, HEIGHT),
                },
                quality,
            )
        };
        run()?;
        device.synchronize()?;
        let start = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            run()?;
        }
        device.synchronize()?;
        let elapsed = start.elapsed() / ITERATIONS;
        let pixels = output.to_host_vec(&stream)?;
        let mut original_error = 0.0f32;
        let mut filtered_error = 0.0f32;
        for ((output, noisy), reference) in pixels.iter().zip(&noisy).zip(&reference) {
            if !output.iter().all(|value| value.is_finite()) || output[3] != reference[3] {
                return Err("OIDN corrupted alpha or produced non-finite pixels".into());
            }
            for channel in 0..3 {
                original_error += (noisy[channel] - reference[channel]).powi(2);
                filtered_error += (output[channel] - reference[channel]).powi(2);
            }
        }
        let ratio = filtered_error / original_error;
        if ratio >= 0.3 {
            return Err(format!("OIDN failed to remove fixture noise: ratio={ratio}").into());
        }
        println!(
            "OIDN CUDA {name}: 1080p {elapsed:?}, noise MSE ratio={ratio:.4}, alpha preserved"
        );
    }
    Ok(())
}
