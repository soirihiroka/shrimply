//! Independent voxel outputs share the language/runtime and exact timeline sampler.
use crate::Args;
use sanim_core::{Fraction, Geometry, math};
use sanim_lang::Prepared;
use sanim_voxel::{Camera, View};
use sanim_voxel_render::{RenderMode, RenderSettings, RenderStats, Renderer};
use serde_json::json;
use std::{fs, io::BufWriter, path::Path, sync::Arc};

struct Sample {
    scene: Arc<sanim_core::voxel::Scene>,
    id: String,
    opacity: f32,
}

fn draw(
    renderer: &mut Renderer,
    snapshot: &Sample,
    camera: &Camera,
    settings: &RenderSettings,
) -> Result<sanim_voxel_render::Frame, String> {
    match &snapshot.scene.source {
        sanim_voxel::SceneSource::Volume(volume) => renderer.render_rgba(volume, camera, settings),
        sanim_voxel::SceneSource::Instances(instances) => {
            renderer.render_instances_rgba(instances, camera, settings)
        }
    }
}

pub(super) fn validate(prepared: &Prepared, args: &Args) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let viewport = args.viewport();
    for layer in &prepared.animation.layers {
        let Geometry::Voxel(scene) = layer.element.geometry.as_ref() else {
            return Err("the independent voxel output cannot contain 2D layers".into());
        };
        ids.insert(&layer.element.id);
        sanim_voxel_render::validate_materials(scene.materials())?;
        scene.resolve_camera([viewport.width, viewport.height])?;
    }
    if ids.len() != 1 {
        return Err("the independent voxel output requires exactly one VoxelScene".into());
    }
    Ok(())
}

fn sample(prepared: &Prepared, time: Fraction) -> Result<Sample, String> {
    let mut elements = prepared.animation.sample(time)?.into_iter();
    let element = elements.next().ok_or(
        "no VoxelScene is mounted at this time; mount it with scene.fade_in() before inspecting",
    )?;
    if elements.next().is_some() {
        return Err("the independent voxel renderer accepts exactly one VoxelScene; 2D composition and multiple voxel views are separate outputs".into());
    }
    let Geometry::Voxel(scene) = element.geometry.as_ref() else {
        return Err(format!(
            "element '{}' is not a VoxelScene; use the ordinary render command for 2D scenes",
            element.id
        ));
    };
    Ok(Sample {
        scene: scene.clone(),
        id: element.id,
        opacity: math::to_f64(element.opacity)? as f32,
    })
}

fn camera(sample: &Sample, args: &Args, view: Option<View>) -> Result<Camera, String> {
    let viewport = args.viewport();
    let size = [viewport.width, viewport.height];
    let camera = if let Some(view) = view {
        Camera::fit(view, sample.scene.dimensions(), size)
    } else {
        sample.scene.resolve_camera(size)
    }?;
    if sample.scene.check_framing {
        sanim_voxel::verify::assert_framed(
            &camera,
            sample.scene.dimensions(),
            size,
            math::to_f64(sample.scene.frame_margin)?,
        )?;
    }
    Ok(camera)
}

fn settings(args: &Args, sample: &Sample, time: Fraction) -> Result<RenderSettings, String> {
    let viewport = args.viewport();
    let defaults = RenderSettings::default();
    let mut options = RenderSettings {
        width: viewport.width,
        height: viewport.height,
        opacity: if args.command == "voxel-inspect" {
            1.0
        } else {
            sample.opacity
        },
        time,
        lights: sample.scene.lights.clone(),
        environment: sample.scene.environment,
        mode: if args.voxel_mode.as_deref() == Some("path-traced") {
            RenderMode::PathTraced
        } else {
            RenderMode::Flat
        },
        samples: args.samples.unwrap_or(defaults.samples),
        bounces: args.bounces.unwrap_or(defaults.bounces),
        ..defaults
    };
    if let Some(value) = args.sun_diameter {
        options.lighting.sun_diameter_degrees = value;
    }
    if let Some(value) = args.sun_strength {
        options.lighting.key_strength = value;
    }
    if let Some(value) = args.environment_strength {
        options.lighting.environment_strength = value;
    }
    if let Some(value) = args.shadows {
        options.lighting.shadows.mode = value;
    }
    if let Some(value) = args.shadow_strength {
        options.lighting.shadows.strength = value;
    }
    if let Some(value) = args.shadow_samples {
        options.lighting.shadows.samples = value;
    }
    if let Some(value) = args.ao {
        options.ambient_occlusion.mode = value;
    }
    if let Some(value) = args.ao_strength {
        options.ambient_occlusion.strength = value;
    }
    if let Some(value) = args.ao_distance {
        options.ambient_occlusion.distance = value;
    }
    if let Some(value) = args.ao_samples {
        options.ambient_occlusion.samples = value;
    }
    if let Some(value) = args.exposure {
        options.exposure_ev = value;
    }
    if let Some(value) = args.reconstruction {
        options.reconstruction.mode = value;
    }
    if let Some(value) = args.spatial_passes {
        options.reconstruction.spatial_passes = value;
    }
    if let Some(value) = args.history_frames {
        options.reconstruction.history_frames = value;
    }
    if let Some(value) = args.antialias {
        options.reconstruction.antialias_samples = value;
    }
    options.validate()?;
    Ok(options)
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let directory = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("PNG output: {error}"))?;
    {
        let mut encoder = png::Encoder::new(BufWriter::new(temporary.as_file()), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        // GPU targets use premultiplied alpha; PNG stores straight RGBA.
        let straight: Vec<u8> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| {
                shrimply_math_color::Color::<f32>::from_rgba8(
                    pixel[0], pixel[1], pixel[2], pixel[3],
                )
                .unpremultiply()
                .to_rgba_u32()
                .to_le_bytes()
            })
            .collect();
        writer
            .write_image_data(&straight)
            .map_err(|error| error.to_string())?;
        writer.finish().map_err(|error| error.to_string())?;
    }
    temporary
        .persist(path)
        .map_err(|error| format!("publish PNG: {error}"))?;
    Ok(())
}

fn render_frame(
    snapshot: &Sample,
    args: &Args,
    options: &RenderSettings,
    renderer: &mut Renderer,
) -> Result<(Camera, RenderStats), String> {
    let camera = camera(snapshot, args, args.views.first().copied())?;
    let frame = draw(renderer, snapshot, &camera, options)?;
    let output = args
        .output
        .as_ref()
        .ok_or("voxel rendering requires --output PATH.png")?;
    write_png(output, frame.width, frame.height, &frame.pixels)?;
    Ok((camera, frame.stats))
}

pub(super) fn watch_render(
    prepared: &Prepared,
    args: &Args,
    renderer: &mut Option<Renderer>,
) -> Result<u64, String> {
    let started = std::time::Instant::now();
    let time = args
        .time
        .unwrap_or(prepared.animation.frame_time(args.frame)?);
    let snapshot = sample(prepared, time)?;
    let options = settings(args, &snapshot, time)?;
    if renderer.is_none() {
        *renderer = Some(Renderer::headless(Default::default())?);
    }
    render_frame(
        &snapshot,
        args,
        &options,
        renderer.as_mut().expect("initialized voxel renderer"),
    )?;
    Ok(started.elapsed().as_micros() as u64)
}

pub(super) fn run(prepared: &Prepared, args: &Args) -> Result<(), String> {
    let time = args
        .time
        .unwrap_or(prepared.animation.frame_time(args.frame)?);
    let snapshot = sample(prepared, time)?;
    sanim_voxel_render::validate_materials(snapshot.scene.materials())?;
    let options = settings(args, &snapshot, time)?;
    let selected = args.views.first().copied();
    let (shadows, ao) = options.effective_modes();
    let mut result = json!({
        "ok":true,"source":prepared.source,"element":snapshot.id,"time":format!("{time}s"),
        "dimensions":snapshot.scene.dimensions(),"occupied_voxels":snapshot.scene.occupied_voxels(),
        "occupied_voxels_upper_bound":snapshot.scene.occupied_voxels_upper_bound(),
        "resident_bytes":snapshot.scene.resident_bytes(),"revision":snapshot.scene.revision(),
        "width":options.width,"height":options.height,"mode":args.voxel_mode.as_deref().unwrap_or("flat"),
        "presentation_opacity":snapshot.opacity,
        "lighting":options.lighting,
        "ambient_occlusion":options.ambient_occlusion,
        "exposure_ev":options.exposure_ev,
        "reconstruction":options.reconstruction,
        "effective_modes":{"shadows":shadows,"ambient_occlusion":ao},
        "coordinate_system":{"axes":"+X right, +Y up, +Z toward the front camera", "voxel_bounds":"minimum inclusive, maximum exclusive", "pixel_origin":"top-left"},
        "source_location":prepared.element_sources.get(&snapshot.id),
        "layout":snapshot.scene.placements,
    });
    match args.command.as_str() {
        "voxel-pick" => {
            let camera = camera(&snapshot, args, selected)?;
            let pixel = args.pixel.ok_or("voxel pick requires --pixel X,Y")?;
            let ray = camera.ray_for_pixel([options.width, options.height], pixel)?;
            let hit = snapshot.scene.raycast(ray.origin, ray.direction)?;
            result["camera"] = serde_json::to_value(camera).map_err(|error| error.to_string())?;
            result["pixel"] = json!(pixel);
            result["hit"] = serde_json::to_value(hit).map_err(|error| error.to_string())?;
        }
        "voxel-inspect" => {
            let views: Vec<_> = if args.views.is_empty() {
                View::ALL.to_vec()
            } else {
                args.views.clone()
            };
            let mut renderer = args
                .output
                .as_ref()
                .map(|_| Renderer::headless(Default::default()))
                .transpose()?;
            if let Some(directory) = &args.output {
                fs::create_dir_all(directory)
                    .map_err(|error| format!("inspection output: {error}"))?;
            }
            let mut outputs = Vec::with_capacity(views.len());
            for view in views {
                let camera = camera(&snapshot, args, Some(view))?;
                let mut output = json!({"view":view,"camera":camera});
                if let Some(renderer) = &mut renderer {
                    let frame = draw(renderer, &snapshot, &camera, &options)?;
                    let path = args
                        .output
                        .as_ref()
                        .expect("renderer requires output")
                        .join(format!("{view}.png"));
                    write_png(&path, frame.width, frame.height, &frame.pixels)?;
                    output["image"] = json!(path);
                    output["statistics"] =
                        serde_json::to_value(frame.stats).map_err(|error| error.to_string())?;
                }
                outputs.push(output);
            }
            result["views"] = json!(outputs);
            if let Some(directory) = &args.output {
                let bytes =
                    serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?;
                fs::write(directory.join("scene.json"), bytes)
                    .map_err(|error| error.to_string())?;
            }
        }
        "voxel-render" => {
            let mut renderer = Renderer::headless(Default::default())?;
            let (camera, statistics) = render_frame(&snapshot, args, &options, &mut renderer)?;
            result["camera"] = serde_json::to_value(camera).map_err(|error| error.to_string())?;
            result["output"] = json!(args.output);
            result["statistics"] =
                serde_json::to_value(statistics).map_err(|error| error.to_string())?;
        }
        "voxel-video" => {
            let duration = args
                .duration
                .or_else(|| (!prepared.animation.indefinite).then_some(prepared.animation.duration))
                .ok_or("an indefinite voxel animation requires --duration, for example 10s")?;
            let mut renderer = Renderer::headless(Default::default())?;
            result = crate::video::encode(
                args,
                prepared.animation.rate,
                duration,
                options.width,
                options.height,
                |index, output| {
                    let time = prepared.animation.frame_time(index)?;
                    let frame = sample(prepared, time)?;
                    let camera = camera(&frame, args, selected)?;
                    let settings = settings(args, &frame, time)?;
                    let frame = draw(&mut renderer, &frame, &camera, &settings)?;
                    output
                        .write_all(&frame.pixels)
                        .map_err(|error| error.to_string())?;
                    Ok(frame.pixels.len())
                },
            )?;
        }
        _ => return Err("unknown voxel command".into()),
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_export_converts_premultiplied_pixels_to_straight_alpha() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("alpha.png");
        super::write_png(&path, 1, 1, &[64, 32, 16, 128]).unwrap();
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut bytes).unwrap();
        assert_eq!(bytes, [128, 64, 32, 128]);
    }
}
