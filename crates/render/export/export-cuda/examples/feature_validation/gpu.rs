use shrimply_preview_render_vulkan::Compositor;
use std::time::Instant;

pub fn refraction(
    mut project: shrimply_project_document::project::Project,
    output: &std::path::Path,
) -> Result<(), String> {
    use shrimply_project_document::project::VideoItemContent;
    use shrimply_scene_3d::DenoisingMethod;
    let mut compositor = Compositor::new()?;
    let mut original = Vec::new();
    let methods = std::iter::once(DenoisingMethod::Off).chain(
        shrimply_visual_cuda::gpu::denoising_methods()
            .into_iter()
            .map(|(method, _)| method)
            .filter(|method| *method != DenoisingMethod::Off),
    );
    for method in methods {
        let VideoItemContent::Obj(scene) = &mut project.video_tracks[1].items[0].content else {
            return Err("Missing refraction validation scene".into());
        };
        scene.material.denoising = method;
        compositor.invalidate();
        let rendered = compositor.render_accurate(
            &project,
            shrimply_math_core::Time::ZERO,
            0,
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        rendered.wait()?;
        let pixels = compositor.renderer.readback(&rendered.buffer)?;
        super::run::save_png(&output.join(format!("refraction-{method:?}.png")), &pixels)?;
        if method == DenoisingMethod::Off {
            original = pixels;
            continue;
        }
        if pixels == original {
            return Err(format!(
                "{method:?} did not filter transmitted background color"
            ));
        }
        if pixels
            .chunks_exact(4)
            .zip(original.chunks_exact(4))
            .any(|(filtered, original)| filtered[3] != original[3])
        {
            return Err(format!("{method:?} changed refraction output alpha"));
        }
        println!("{method:?} filtered transmitted background color and preserved alpha");
    }
    Ok(())
}

pub fn denoise() -> Result<(), String> {
    const SIZE: (u32, u32) = (160, 96);
    const BACKGROUND_ROWS: u32 = 4;
    let mut colors = Vec::new();
    let mut guides = Vec::new();
    let mut expected = Vec::new();
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            let right = x >= SIZE.0 / 2;
            let value = if right { 4.0_f32 } else { 1.0 };
            let hash =
                (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(83_492_791);
            let noise = (hash as u8 as f32 / u8::MAX as f32 - 0.5) * 0.8;
            colors.push([
                value + noise,
                value + noise,
                value + noise,
                if y < BACKGROUND_ROWS { 0.0 } else { 1.0 },
            ]);
            expected.push(value);
            guides.push(if y < BACKGROUND_ROWS {
                [0.0; 4]
            } else if right {
                [6.0, 1.0, 0.0, 0.0]
            } else {
                [2.0, 0.0, 0.0, 1.0]
            });
        }
    }
    let mut compositor = Compositor::new()?;
    let input = compositor.renderer.upload(bytes(&colors))?;
    let guide = compositor.renderer.upload(bytes(&guides))?;
    let output = compositor.renderer.allocate(input.len())?;
    let mut submissions = Vec::new();
    compositor
        .renderer
        .denoise_atrous(&input, &output, &guide, SIZE, &mut submissions)?;
    submissions.last().ok_or("No denoiser submission")?.wait()?;
    let filtered = compositor.renderer.readback(&output)?;
    let mut before_error = 0.0_f32;
    let mut after_error = 0.0_f32;
    for (index, pixel) in filtered.chunks_exact(size_of::<[f32; 4]>()).enumerate() {
        let values: [f32; 4] = std::array::from_fn(|channel| {
            f32::from_ne_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
        });
        if values.iter().any(|value| !value.is_finite()) || values[3] != colors[index][3] {
            return Err("Denoiser changed alpha or produced non-finite pixels".into());
        }
        if index < (SIZE.0 * BACKGROUND_ROWS) as usize {
            if values != colors[index] {
                return Err("Denoiser changed background pixels".into());
            }
            continue;
        }
        before_error += (colors[index][0] - expected[index]).powi(2);
        after_error += (values[0] - expected[index]).powi(2);
        if (values[0] - expected[index]).abs() > 0.4 {
            return Err("Denoiser leaked across a depth/normal edge".into());
        }
    }
    let ratio = after_error / before_error;
    println!("GPU a-trous remaining mean squared error: {ratio:.4}");
    if !ratio.is_finite() || ratio >= 0.6 {
        return Err("GPU denoiser failed to reduce noise".into());
    }

    const BENCHMARK_SIZE: (u32, u32) = (1920, 1080);
    const ITERATIONS: u32 = 5;
    let pixels = (BENCHMARK_SIZE.0 * BENCHMARK_SIZE.1) as usize;
    let input = compositor
        .renderer
        .upload(bytes(&vec![[1.0_f32, 1.0, 1.0, 1.0]; pixels]))?;
    let guide = compositor
        .renderer
        .upload(bytes(&vec![[2.0_f32, 0.0, 0.0, 1.0]; pixels]))?;
    let output = compositor.renderer.allocate(input.len())?;
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let mut submissions = Vec::new();
        compositor.renderer.denoise_atrous(
            &input,
            &output,
            &guide,
            BENCHMARK_SIZE,
            &mut submissions,
        )?;
        submissions
            .last()
            .ok_or("No denoiser benchmark submission")?
            .wait()?;
    }
    println!(
        "GPU a-trous 1920x1080 latency: {:?}",
        started.elapsed() / ITERATIONS
    );
    Ok(())
}

fn bytes(values: &[[f32; 4]]) -> &[u8] {
    // Float arrays have no padding and all float bit patterns are valid bytes.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), size_of_val(values)) }
}
