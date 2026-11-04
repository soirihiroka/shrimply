use crate::gpu::{Buffer, Renderer, Submission};
use shrimply_render_core::effects::{BufferSlot, PixelEffect, SpatialState};

pub(super) fn apply_stabilization(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    warp: &shrimply_visual_core::stabilization::StabilizationWarp,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let (width, height) = size;
    let (source, baked) = state.materialization(size);
    let (input, submission) = renderer
        .composite_buffers(&[(source, input)], width, height, 0)?
        .into_parts();
    submissions.push(submission);
    match warp {
        shrimply_visual_core::stabilization::StabilizationWarp::Affine(source_transform) => {
            let count = usize::try_from(u64::from(width) * u64::from(height))
                .map_err(|_| "Stabilization canvas is too large")?;
            let output = renderer.allocate(
                count
                    .checked_mul(size_of::<u32>())
                    .ok_or("Stabilization output size overflow")?,
            )?;
            let mut arguments = renderer.arguments("affine_stabilization")?;
            arguments
                .set("params.input", &input.address().to_ne_bytes())?
                .set("params.width", &width.to_ne_bytes())?
                .set("params.height", &height.to_ne_bytes())?
                .set_matrix3("params.source_transform", *source_transform)?
                .set("out", &output.address().to_ne_bytes())?
                .set("out_len", &(count as u64).to_ne_bytes())?;
            submissions.push(unsafe {
                renderer.dispatch(arguments, vec![input, output.clone()], [count, 1, 1])
            }?);
            Ok((baked, output))
        }
        shrimply_visual_core::stabilization::StabilizationWarp::Mesh {
            grid_width,
            grid_height,
            source_offsets,
        } => {
            let offsets = source_offsets
                .iter()
                .map(|offset| offset.to_array())
                .collect::<Vec<_>>();
            let (output, submission) =
                renderer.mesh_flow(input, width, height, *grid_width, *grid_height, &offsets)?;
            submissions.push(submission);
            Ok((baked, output))
        }
    }
}

pub(super) fn apply_transition(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    effect: shrimply_visual_core::transition::RasterTransition,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    match effect {
        shrimply_visual_core::transition::RasterTransition::Pixel(effect) => apply(
            renderer,
            input,
            state,
            std::slice::from_ref(&effect),
            size,
            submissions,
        ),
        shrimply_visual_core::transition::RasterTransition::Origami(effect) => {
            apply_origami(renderer, input, state, effect, size, submissions)
        }
    }
}

fn apply_origami(
    renderer: &mut Renderer,
    mut input: Buffer,
    state: SpatialState,
    effect: shrimply_visual_core::transition::Origami,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let (width, height) = size;
    let state = if state.needs_materialization(size) {
        let (source, baked) = state.materialization(size);
        let (buffer, submission) = renderer
            .composite_buffers(&[(source, input)], width, height, 0)?
            .into_parts();
        submissions.push(submission);
        input = buffer;
        baked
    } else {
        state
    };
    let vertices = shrimply_math_media::origami_mesh_vertices(
        width,
        height,
        effect.grid,
        effect.visibility,
        effect.depth,
        effect.direction_degrees,
    );
    let vertex_bytes: Vec<_> = vertices
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect();
    let vertices = renderer.upload(&vertex_bytes)?;
    let count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| "Origami canvas is too large")?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or("Origami buffer size overflow")?,
    )?;
    let mut arguments = renderer.arguments("origami_transition")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("width", &width.to_ne_bytes())?
        .set("height", &height.to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &(count as u64).to_ne_bytes())?
        .set("vertices", &vertices.address().to_ne_bytes())?
        .set("grid", &effect.grid.to_ne_bytes())?
        .set("visibility", &effect.visibility.to_ne_bytes())?;
    submissions.push(unsafe {
        renderer.dispatch(
            arguments,
            vec![input, output.clone(), vertices],
            [count, 1, 1],
        )
    }?);
    Ok((state, output))
}

pub(super) fn apply_motion_blur(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    samples: &[shrimply_math_geometry::ComposedTransform2D],
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let motion = shrimply_render_core::effects::motion_materialization(state, samples, size)?;
    let (buffer, submission) = renderer
        .composite_buffers_with_transforms(
            &[(motion.source, input)],
            size.0,
            size.1,
            0,
            &motion.inverses,
        )?
        .into_parts();
    submissions.push(submission);
    Ok((motion.baked, buffer))
}

pub(super) struct ExternalMasks<'a> {
    pub frames: &'a [(
        shrimply_visual_core::raster_modifiers::ExternalDependency,
        Buffer,
    )],
    pub size: (u32, u32),
    pub output_transform: shrimply_render_core::math::Mat3,
}

pub(super) struct ModifierRequest<'a> {
    pub operations: &'a [shrimply_visual_core::raster_modifiers::Modifier],
    pub size: (u32, u32),
    pub external_masks: ExternalMasks<'a>,
    pub sam2_target: Option<&'a shrimply_visual_core::sam2::analysis::AnalysisTarget>,
    pub sam2_proxy: &'a mut Option<Buffer>,
}

pub(super) fn apply_modifiers(
    renderer: &mut Renderer,
    mut input: Buffer,
    mut state: SpatialState,
    request: ModifierRequest<'_>,
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let ModifierRequest {
        operations,
        size,
        external_masks,
        sam2_target,
        sam2_proxy,
    } = request;
    use shrimply_visual_core::raster_modifiers::Operation;
    for modifier in operations {
        let original = modifier
            .alpha_mask
            .as_ref()
            .map(|_| super::alpha_mask::Branch {
                buffer: input.clone(),
                state,
            });
        if !modifier.operation.apply_spatial(&mut state) {
            match &modifier.operation {
                Operation::Pixel(effect) => {
                    (state, input) = apply(
                        renderer,
                        input,
                        state,
                        std::slice::from_ref(effect),
                        size,
                        submissions,
                    )?;
                }
                Operation::Dithering(effect) => {
                    (state, input) =
                        apply_dithering(renderer, input, state, effect, size, submissions)?;
                }
                Operation::Sam2Mask(effect) => {
                    if sam2_target == Some(&effect.target) {
                        (state, input) =
                            materialize_mask_input(renderer, input, state, size, submissions)?;
                        if sam2_proxy.is_none() {
                            *sam2_proxy = Some(capture_sam2_proxy(
                                renderer,
                                &input,
                                size.0,
                                size.1,
                                submissions,
                            )?);
                        }
                    } else {
                        (state, input) =
                            apply_sam2_mask(renderer, input, state, effect, size, submissions)?;
                    }
                }
                Operation::TransparentFillMask(effect) => {
                    (state, input) = apply_transparent_fill_mask(
                        renderer,
                        input,
                        state,
                        effect,
                        size,
                        submissions,
                    )?;
                }
                Operation::Mask(effect) => {
                    (state, input) = apply_external_mask(
                        renderer,
                        input,
                        state,
                        effect,
                        &external_masks,
                        submissions,
                    )?;
                }
                Operation::Transform(_)
                | Operation::Opacity(_)
                | Operation::Sampling(_)
                | Operation::TextureBounds { .. }
                | Operation::CropPercentage(_)
                | Operation::CropPixels(_)
                | Operation::RasterBoundary => unreachable!("spatial operation was applied"),
            }
        }
        if let Some(original) = original {
            (state, input) = super::alpha_mask::combine(
                renderer,
                original,
                super::alpha_mask::Branch {
                    buffer: input,
                    state,
                },
                modifier.alpha_mask.as_ref().expect("masked modifier"),
                size,
                submissions,
            )?;
        }
    }
    Ok((state, input))
}

fn apply_external_mask(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    effect: &shrimply_visual_core::raster_modifiers::ExternalMask,
    masks: &ExternalMasks<'_>,
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let width = state.parameters.source_width;
    let height = state.parameters.source_height;
    let count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| "Mask input is too large")?;
    let output_count = u64::try_from(count).map_err(|_| "Mask input is too large")?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or("Mask output size overflow")?,
    )?;
    let mask = effect.source.as_ref().and_then(|source| {
        masks
            .frames
            .iter()
            .find(|(dependency, _)| dependency == source)
            .map(|(_, buffer)| buffer)
    });
    let mut arguments = renderer.arguments("mask")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set(
            "params.mask",
            &mask.map_or(0, Buffer::address).to_ne_bytes(),
        )?
        .set("params.input_width", &width.to_ne_bytes())?
        .set("params.mask_width", &masks.size.0.to_ne_bytes())?
        .set("params.mask_height", &masks.size.1.to_ne_bytes())?
        .set_matrix3("params.transform", masks.output_transform * state.transform)?
        .set("params.luminance", &[u8::from(effect.luminance)])?
        .set("params.invert", &[u8::from(effect.invert)])?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &output_count.to_ne_bytes())?;
    let mut resources = vec![input, output.clone()];
    resources.extend(mask.cloned());
    submissions.push(unsafe { renderer.dispatch(arguments, resources, [count, 1, 1]) }?);
    Ok((state, output))
}

fn capture_sam2_proxy(
    renderer: &mut Renderer,
    input: &Buffer,
    input_width: u32,
    input_height: u32,
    submissions: &mut Vec<Submission>,
) -> Result<Buffer, String> {
    if input_width == 0 || input_height == 0 {
        return Err("SAM2 proxy source dimensions must be nonzero".to_string());
    }
    let side = shrimply_visual_core::sam2::MODEL_SIZE;
    let count = usize::try_from(u64::from(side) * u64::from(side))
        .map_err(|_| "SAM2 proxy dimensions overflow")?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or("SAM2 proxy size overflow")?,
    )?;
    let mut arguments = renderer.arguments("sam2_proxy")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("params.input_width", &input_width.to_ne_bytes())?
        .set("params.input_height", &input_height.to_ne_bytes())?
        .set("params.model_size", &side.to_ne_bytes())?;
    submissions.push(unsafe {
        renderer.dispatch(
            arguments,
            vec![input.clone(), output.clone()],
            [count, 1, 1],
        )
    }?);
    Ok(output)
}

fn materialize_mask_input(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    if !state.needs_materialization(size) {
        return Ok((state, input));
    }
    let (source, baked) = state.materialization(size);
    let (input, submission) = renderer
        .composite_buffers(&[(source, input)], size.0, size.1, 0)?
        .into_parts();
    submissions.push(submission);
    Ok((baked, input))
}

fn mask_output(
    renderer: &Renderer,
    size: (u32, u32),
    effect_name: &str,
) -> Result<(usize, u64, Buffer), String> {
    let count = usize::try_from(u64::from(size.0) * u64::from(size.1))
        .map_err(|_| format!("{effect_name} canvas is too large"))?;
    let output_count =
        u64::try_from(count).map_err(|_| format!("{effect_name} canvas is too large"))?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or_else(|| format!("{effect_name} output size overflow"))?,
    )?;
    Ok((count, output_count, output))
}

fn apply_sam2_mask(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    effect: &shrimply_visual_core::sam2::ResolvedMask,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let Some(mask) = &effect.mask else {
        return Ok((state, input));
    };
    let expected = usize::try_from(shrimply_visual_core::sam2::MASK_SIZE)
        .ok()
        .and_then(|side| side.checked_mul(side))
        .ok_or("SAM2 mask dimensions overflow")?;
    if mask.len() != expected {
        return Err(format!(
            "SAM2 mask has {} samples; expected {expected}",
            mask.len()
        ));
    }
    let (state, input) = materialize_mask_input(renderer, input, state, size, submissions)?;
    let bytes = mask.iter().map(|value| *value as u8).collect::<Vec<_>>();
    let mask = renderer.upload(&bytes)?;
    let (count, output_count, output) = mask_output(renderer, size, "SAM2")?;
    let mut arguments = renderer.arguments("sam2_apply_mask")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("masks", &mask.address().to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &output_count.to_ne_bytes())?
        .set("params.output_width", &size.0.to_ne_bytes())?
        .set("params.output_height", &size.1.to_ne_bytes())?
        .set(
            "params.mask_size",
            &shrimply_visual_core::sam2::MASK_SIZE.to_ne_bytes(),
        )?
        .set("params.threshold", &effect.threshold.to_ne_bytes())?
        .set("params.softness", &effect.softness.to_ne_bytes())?
        .set(
            "params.quantization_scale",
            &shrimply_visual_core::sam2::MASK_LOGIT_QUANTIZATION_SCALE.to_ne_bytes(),
        )?
        .set("params.invert", &[u8::from(effect.invert)])?;
    submissions.push(unsafe {
        renderer.dispatch(arguments, vec![input, mask, output.clone()], [count, 1, 1])
    }?);
    Ok((state, output))
}

fn apply_transparent_fill_mask(
    renderer: &mut Renderer,
    input: Buffer,
    state: SpatialState,
    effect: &shrimply_visual_core::transparent_fill::ResolvedMask,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let Some(mask) = &effect.mask else {
        return Ok((state, input));
    };
    let stride = size.0.div_ceil(u8::BITS);
    let expected = usize::try_from(stride)
        .ok()
        .and_then(|stride| {
            usize::try_from(size.1)
                .ok()
                .and_then(|height| stride.checked_mul(height))
        })
        .ok_or("Transparent Fill mask dimensions overflow")?;
    if mask.len() != expected {
        return Err(format!(
            "Transparent Fill mask has {} bytes; expected {expected}",
            mask.len()
        ));
    }
    let (state, input) = materialize_mask_input(renderer, input, state, size, submissions)?;
    let mask = renderer.upload(mask)?;
    let (count, output_count, output) = mask_output(renderer, size, "Transparent Fill")?;
    let mut arguments = renderer.arguments("transparent_fill_apply_mask")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("mask_bits", &mask.address().to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &output_count.to_ne_bytes())?
        .set("params.width", &size.0.to_ne_bytes())?
        .set("params.height", &size.1.to_ne_bytes())?
        .set("params.stride", &stride.to_ne_bytes())?;
    submissions.push(unsafe {
        renderer.dispatch(arguments, vec![input, mask, output.clone()], [count, 1, 1])
    }?);
    Ok((state, output))
}

fn apply_dithering(
    renderer: &mut Renderer,
    mut input: Buffer,
    state: SpatialState,
    effect: &shrimply_visual_core::raster_modifiers::Dithering,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let (width, height) = size;
    let state = if state.needs_materialization(size) {
        let (source, baked) = state.materialization(size);
        let (buffer, submission) = renderer
            .composite_buffers(&[(source, input)], width, height, 0)?
            .into_parts();
        submissions.push(submission);
        input = buffer;
        baked
    } else {
        state
    };
    let count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| "Dithering canvas is too large")?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or("Dithering buffer size overflow")?,
    )?;
    let palette = (!effect.palette.is_empty())
        .then(|| {
            let bytes: Vec<_> = effect
                .palette
                .iter()
                .flat_map(|color| color.to_ne_bytes())
                .collect();
            renderer.upload(&bytes)
        })
        .transpose()?;
    let mut arguments = renderer.arguments("dithering")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("width", &width.to_ne_bytes())?
        .set("out", &output.address().to_ne_bytes())?
        .set("out_len", &(count as u64).to_ne_bytes())?
        .set(
            "params.palette",
            &palette.as_ref().map_or(0, Buffer::address).to_ne_bytes(),
        )?
        .set("params.levels", &effect.levels.to_ne_bytes())?
        .set("params.amount", &effect.amount.to_ne_bytes())?
        .set(
            "params.palette_len",
            &u32::try_from(effect.palette.len())
                .map_err(|_| "Dithering palette is too large")?
                .to_ne_bytes(),
        )?
        .set("params.pattern", &(effect.pattern as u32).to_ne_bytes())?
        .set(
            "params.color_mode",
            &(effect.color_mode as u32).to_ne_bytes(),
        )?;
    let mut resources = vec![input, output.clone()];
    resources.extend(palette);
    submissions.push(unsafe { renderer.dispatch(arguments, resources, [count, 1, 1]) }?);
    Ok((state, output))
}

pub(super) fn apply(
    renderer: &mut Renderer,
    mut input: Buffer,
    state: SpatialState,
    effects: &[PixelEffect],
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(SpatialState, Buffer), String> {
    let mut effects = effects
        .iter()
        .filter(|effect| !effect.is_identity())
        .peekable();
    if effects.peek().is_none() {
        return Ok((state, input));
    }
    let (width, height) = size;
    let baked = if state.needs_materialization(size) {
        let (source, baked) = state.materialization(size);
        let (buffer, submission) = renderer
            .composite_buffers(&[(source, input)], width, height, 0)?
            .into_parts();
        submissions.push(submission);
        input = buffer;
        baked
    } else {
        state
    };
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Effect canvas size overflow")?;
    let bytes = count
        .checked_mul(size_of::<u32>())
        .ok_or("Effect buffer size overflow")?;
    for effect in effects {
        let output = renderer.allocate(bytes)?;
        let scratch_bytes = bytes
            .checked_mul(effect.scratch_words_per_pixel())
            .ok_or("Effect scratch size overflow")?;
        let scratch = (scratch_bytes != 0)
            .then(|| renderer.allocate(scratch_bytes))
            .transpose()?;
        for pass in effect.passes(width, height) {
            let mut arguments = renderer.arguments(pass.kernel)?;
            for (name, value) in pass.arguments {
                match value {
                    shrimply_render_core::effects::Value::ChannelMixer(matrix) => {
                        arguments.set_matrix3(&format!("{name}.matrix"), matrix)?;
                        continue;
                    }
                    shrimply_render_core::effects::Value::CornerPin {
                        parameters,
                        width,
                        height,
                    } => {
                        let corners: Vec<_> = parameters
                            .corners
                            .into_iter()
                            .flat_map(|corner| corner.to_array())
                            .flat_map(f32::to_ne_bytes)
                            .collect();
                        arguments
                            .set(&format!("{name}.corners"), &corners)?
                            .set(&format!("{name}.input"), &input.address().to_ne_bytes())?
                            .set(&format!("{name}.width"), &width.to_ne_bytes())?
                            .set(&format!("{name}.height"), &height.to_ne_bytes())?
                            .set_matrix3(
                                &format!("{name}.inverse_homography"),
                                parameters.inverse_homography,
                            )?
                            .set(
                                &format!("{name}.perspective"),
                                &parameters.perspective.to_ne_bytes(),
                            )?;
                        continue;
                    }
                    _ => {}
                }
                arguments.set(
                    name,
                    &value.bytes(|slot| match slot {
                        BufferSlot::Input => input.address(),
                        BufferSlot::Output => output.address(),
                        BufferSlot::Scratch => {
                            scratch.as_ref().expect("effect requires scratch").address()
                        }
                    }),
                )?;
            }
            let mut resources = vec![input.clone(), output.clone()];
            resources.extend(scratch.iter().cloned());
            // The shared plan addresses exactly these canvas-sized RGBA buffers.
            // One Metal queue preserves materialization and pass order; submissions
            // retain every intermediate allocation through final-frame completion.
            submissions.push(unsafe { renderer.dispatch(arguments, resources, [count, 1, 1]) }?);
        }
        input = output;
    }
    Ok((baked, input))
}
