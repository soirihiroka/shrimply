use crate::gpu::{Buffer, Renderer, Submission};
use shrimply_visual_core::layered_image::Prepared;
use std::collections::{HashMap, HashSet};

pub fn composite(
    renderer: &mut Renderer,
    plan: &Prepared,
    sources: &mut HashMap<(String, usize), Buffer>,
    used_sources: &mut HashSet<(String, usize)>,
    submissions: &mut Vec<Submission>,
) -> Result<Buffer, String> {
    let width = plan.document.width.max(1);
    let height = plan.document.height.max(1);
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or("Layered image dimensions overflow GPU address space")?;
    let byte_count = pixel_count
        .checked_mul(size_of::<u32>())
        .ok_or("Layered image byte size overflow")?;
    let pitch = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(size_of::<u32>()))
        .and_then(|pitch| u64::try_from(pitch).ok())
        .ok_or("Layered image row pitch overflow")?;
    let output_count = u64::try_from(pixel_count)
        .map_err(|_| "Layered image pixel count exceeds the shared Slang ABI")?;

    for (index, layer) in plan.document.layers.iter().enumerate() {
        if layer.rgba.len() != byte_count {
            return Err(format!(
                "Layered image layer {} has {} RGBA bytes, expected {byte_count}",
                layer.name,
                layer.rgba.len()
            ));
        }
        let key = (plan.source_key.clone(), index);
        used_sources.insert(key.clone());
        if let std::collections::hash_map::Entry::Vacant(entry) = sources.entry(key) {
            let source = renderer.upload(&layer.rgba)?;
            entry.insert(source);
        }
    }

    // Initialize the blend destination on the GPU; no canvas-sized CPU upload.
    let (output, clear) = renderer
        .composite_buffers(&[], width, height, 0)?
        .into_parts();
    submissions.push(clear);
    for layer in &plan.layers {
        let source = sources
            .get(&(plan.source_key.clone(), layer.source))
            .cloned()
            .ok_or("Shared LayeredImage plan references a missing source layer")?;
        let clipping = layer
            .clipping_base
            .map(|(index, opacity)| {
                sources
                    .get(&(plan.source_key.clone(), index))
                    .cloned()
                    .map(|buffer| (buffer, opacity))
                    .ok_or("Shared LayeredImage plan references a missing clipping base")
            })
            .transpose()?;
        let mut arguments = renderer.arguments("composite_layered_image_layer")?;
        arguments
            .set("params.source", &source.address().to_ne_bytes())?
            .set(
                "params.clipping_base",
                &clipping
                    .as_ref()
                    .map_or(0, |(buffer, _)| buffer.address())
                    .to_ne_bytes(),
            )?
            .set("params.source_pitch", &pitch.to_ne_bytes())?
            .set(
                "params.clipping_base_pitch",
                &clipping.as_ref().map_or(0, |_| pitch).to_ne_bytes(),
            )?
            .set("params.width", &width.to_ne_bytes())?
            .set("params.opacity", &layer.opacity.to_ne_bytes())?
            .set(
                "params.clipping_base_opacity",
                &clipping
                    .as_ref()
                    .map_or(1.0, |(_, opacity)| *opacity)
                    .to_ne_bytes(),
            )?
            .set("params.noise_seed", &layer.noise_seed.to_ne_bytes())?
            .set("params.mode", &[layer.mode as u8])?
            .set("output", &output.address().to_ne_bytes())?
            .set("output_count", &output_count.to_ne_bytes())?;
        let mut resources = vec![source, output.clone()];
        resources.extend(clipping.map(|(buffer, _)| buffer));
        submissions.push(unsafe { renderer.dispatch(arguments, resources, [pixel_count, 1, 1]) }?);
    }
    Ok(output)
}
