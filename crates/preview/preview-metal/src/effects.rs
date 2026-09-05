use shrimply_render_core::{
    Nv12LayerParams,
    effects::{BufferSlot, PixelEffect},
};
use shrimply_render_metal::{Buffer, Renderer, Submission};

// Keep forward spatial state until sampling, matching CUDA. A near-singular
// transform can become renderable after another transform or a shutter sample.
#[derive(Clone, Copy)]
pub(super) struct State {
    pub parameters: Nv12LayerParams,
    pub transform: shrimply_render_core::math::Mat3,
}

impl State {
    pub fn sampled(&self) -> Nv12LayerParams {
        let mut parameters = self.parameters;
        if let Some(inverse) = shrimply_render_core::math::inverse_affine(self.transform) {
            parameters.inverse = inverse;
        } else {
            parameters.opacity = 0.0;
        }
        parameters
    }
}

pub(super) fn apply_motion_blur(
    renderer: &mut Renderer,
    input: Buffer,
    state: State,
    samples: &[shrimply_math_geometry::ComposedTransform2D],
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(State, Buffer), String> {
    let inverses = shrimply_math_geometry::motion_sample_inverses(state.transform, samples);
    let (mut source, baked) =
        shrimply_render_core::effects::materialization(state.parameters, size.0, size.1);
    // CUDA falls back to a valid sample inverse when the current state is singular.
    source.inverse = shrimply_render_core::math::inverse_affine(state.transform)
        .or_else(|| inverses.first().copied())
        .unwrap_or(shrimply_render_core::math::Mat3::IDENTITY);
    source.motion_transform_offset = 0;
    source.motion_transform_count = inverses
        .len()
        .try_into()
        .map_err(|_| "Motion transform count overflow")?;
    // Singular samples contribute transparency rather than increasing the weight of valid samples.
    source.motion_sample_count = samples
        .len()
        .try_into()
        .map_err(|_| "Motion sample count overflow")?;
    let (buffer, submission) = renderer
        .composite_buffers_with_transforms(&[(source, input)], size.0, size.1, 0, &inverses)?
        .into_parts();
    submissions.push(submission);
    Ok((
        State {
            parameters: baked,
            transform: shrimply_render_core::math::Mat3::IDENTITY,
        },
        buffer,
    ))
}

pub(super) fn apply_modifiers(
    renderer: &mut Renderer,
    mut input: Buffer,
    mut state: State,
    operations: &[shrimply_video_core::raster_modifiers::Modifier],
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(State, Buffer), String> {
    use shrimply_video_core::raster_modifiers::Operation;
    for modifier in operations {
        let original = modifier
            .alpha_mask
            .as_ref()
            .map(|_| super::alpha_mask::Branch {
                buffer: input.clone(),
                state,
            });
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
            Operation::Transform(transform) => {
                state.transform = transform.matrix * state.transform;
            }
            Operation::Opacity(opacity) => state.parameters.opacity *= opacity,
            Operation::Sampling(method) => state.parameters.sample_method = *method,
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

pub(super) fn apply(
    renderer: &mut Renderer,
    mut input: Buffer,
    state: State,
    effects: &[PixelEffect],
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(State, Buffer), String> {
    let mut effects = effects
        .iter()
        .filter(|effect| !effect.is_identity())
        .peekable();
    if effects.peek().is_none() {
        return Ok((state, input));
    }
    let (width, height) = size;
    let parameters = state.parameters;
    let spatial_identity = parameters.kind == shrimply_render_core::LayerKind::Rgba
        && state.transform == shrimply_render_core::math::Mat3::IDENTITY
        && parameters.crop == [0.0; 4]
        && parameters.padding == [0.0; 4]
        && parameters.address_mode == shrimply_render_core::TextureAddressMode::Transparent
        && parameters.motion_transform_count == 0;
    let baked = if shrimply_render_core::effects::needs_canvas_materialization(
        (parameters.source_width, parameters.source_height),
        size,
        spatial_identity,
    ) {
        let (source, baked) =
            shrimply_render_core::effects::materialization(parameters, width, height);
        let source = State {
            parameters: source,
            transform: state.transform,
        }
        .sampled();
        let (buffer, submission) = renderer
            .composite_buffers(&[(source, input)], width, height, 0)?
            .into_parts();
        submissions.push(submission);
        input = buffer;
        State {
            parameters: baked,
            transform: shrimply_render_core::math::Mat3::IDENTITY,
        }
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
