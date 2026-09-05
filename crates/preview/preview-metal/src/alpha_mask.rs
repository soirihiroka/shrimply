use shrimply_render_core::math::{Mat3, Vec2};
use shrimply_render_metal::{Buffer, Renderer, Submission};
use shrimply_video_core::alpha_mask::ResolvedShapeAlphaMask;

pub(super) fn video(
    renderer: &mut Renderer,
    input: Buffer,
    input_size: (u32, u32),
    mask: Buffer,
    mask_size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<Buffer, String> {
    let count = (input_size.0 as usize)
        .checked_mul(input_size.1 as usize)
        .ok_or("Video mask raster size overflow")?;
    let output = renderer.allocate(
        count
            .checked_mul(size_of::<u32>())
            .ok_or("Video mask buffer size overflow")?,
    )?;
    let mut arguments = renderer.arguments("alpha_mask")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &(count as u64).to_ne_bytes())?
        .set("params.mask", &mask.address().to_ne_bytes())?
        .set("params.input_width", &input_size.0.to_ne_bytes())?
        .set("params.input_height", &input_size.1.to_ne_bytes())?
        .set("params.mask_width", &mask_size.0.to_ne_bytes())?
        .set("params.mask_height", &mask_size.1.to_ne_bytes())?;
    // Both RGBA rasters match their declared bounds and survive until completion.
    submissions.push(unsafe {
        renderer.dispatch(arguments, vec![input, mask, output.clone()], [count, 1, 1])
    }?);
    Ok(output)
}

/// The mask preserves the current raster dimensions and the caller's spatial state.
/// This is CUDA's preserving-pixel path: remaining transforms are sampled afterward.
pub(super) fn apply(
    renderer: &mut Renderer,
    input: Buffer,
    size: (u32, u32),
    mask: &ResolvedShapeAlphaMask,
    submissions: &mut Vec<Submission>,
) -> Result<Buffer, String> {
    dispatch(
        renderer,
        Pass {
            input,
            base: None,
            size,
            canvas_to_local: Mat3::IDENTITY,
            local_size: Vec2::new(size.0 as f32, size.1 as f32),
        },
        mask,
        submissions,
    )
}

pub(super) struct Branch {
    pub buffer: Buffer,
    pub state: super::effects::State,
}

pub(super) fn combine(
    renderer: &mut Renderer,
    original: Branch,
    affected: Branch,
    mask: &ResolvedShapeAlphaMask,
    size: (u32, u32),
    submissions: &mut Vec<Submission>,
) -> Result<(super::effects::State, Buffer), String> {
    let local_size = Vec2::new(
        original.state.parameters.source_width as f32,
        original.state.parameters.source_height as f32,
    );
    let opacity = original.state.parameters.opacity;
    let plan = shrimply_video_core::alpha_mask::branch(
        original.state.transform,
        opacity,
        affected.state.parameters.opacity,
    )?;
    let (_, mut parameters) =
        shrimply_render_core::effects::materialization(affected.state.parameters, size.0, size.1);
    parameters.opacity = opacity;
    let mut render = |branch: Branch, opacity| -> Result<Buffer, String> {
        let mut state = branch.state;
        state.parameters.opacity = opacity;
        state.parameters.blend_mode = shrimply_render_core::LayerBlendMode::Normal;
        let (buffer, submission) = renderer
            .composite_buffers(&[(state.sampled(), branch.buffer)], size.0, size.1, 0)?
            .into_parts();
        submissions.push(submission);
        Ok(buffer)
    };
    let base = render(original, 1.0)?;
    let input = render(affected, plan.affected_opacity)?;
    let output = dispatch(
        renderer,
        Pass {
            input,
            base: Some(base),
            size,
            canvas_to_local: plan.canvas_to_local,
            local_size,
        },
        mask,
        submissions,
    )?;
    Ok((
        super::effects::State {
            parameters,
            transform: Mat3::IDENTITY,
        },
        output,
    ))
}

struct Pass {
    input: Buffer,
    base: Option<Buffer>,
    size: (u32, u32),
    canvas_to_local: Mat3,
    local_size: Vec2,
}

fn dispatch(
    renderer: &mut Renderer,
    pass: Pass,
    mask: &ResolvedShapeAlphaMask,
    submissions: &mut Vec<Submission>,
) -> Result<Buffer, String> {
    let Pass {
        input,
        base,
        size,
        canvas_to_local,
        local_size,
    } = pass;
    let (width, height) = size;
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Shape mask raster size overflow")?;
    let bytes = count
        .checked_mul(size_of::<u32>())
        .ok_or("Shape mask buffer size overflow")?;
    let output = renderer.allocate(bytes)?;
    let vertices = (!mask.vertices.is_empty())
        .then(|| {
            let bytes: Vec<_> = mask
                .vertices
                .iter()
                .flat_map(|point| point.to_array())
                .flat_map(f32::to_ne_bytes)
                .collect();
            renderer.upload(&bytes)
        })
        .transpose()?;
    let vertex_count = u32::try_from(mask.vertices.len())
        .map_err(|_| "Shape mask has too many polygon vertices")?;
    let mut arguments = renderer.arguments("shape_alpha_mask")?;
    arguments
        .set("input", &input.address().to_ne_bytes())?
        .set("output", &output.address().to_ne_bytes())?
        .set("output_count", &(count as u64).to_ne_bytes())?
        .set(
            "params.base",
            &base.as_ref().map_or(0, Buffer::address).to_ne_bytes(),
        )?
        .set("params.input_width", &width.to_ne_bytes())?
        .set_matrix3("params.canvas_to_local", canvas_to_local)?
        .set("params.local_width", &local_size.x.to_ne_bytes())?
        .set("params.local_height", &local_size.y.to_ne_bytes())?
        .set(
            "params.center",
            mask.center.to_array().map(f32::to_ne_bytes).as_flattened(),
        )?
        .set(
            "params.size",
            mask.size.to_array().map(f32::to_ne_bytes).as_flattened(),
        )?
        .set(
            "params.rotation_degrees",
            &mask.rotation_degrees.to_ne_bytes(),
        )?
        .set("params.feather", &mask.feather.to_ne_bytes())?
        .set("params.rounding", &mask.rounding.to_ne_bytes())?
        .set("params.shape", &(mask.shape as u32).to_ne_bytes())?
        .set(
            "params.vertices",
            &vertices.as_ref().map_or(0, Buffer::address).to_ne_bytes(),
        )?
        .set("params.vertex_count", &vertex_count.to_ne_bytes())?
        .set("params.invert", &[u8::from(mask.invert)])?;
    let mut resources = vec![input, output.clone()];
    resources.extend(base);
    resources.extend(vertices);
    // The current RGBA raster and packed float2 vertices match the shared shader's
    // bounds. Retain all addressed storage until this dispatch completes.
    submissions.push(unsafe { renderer.dispatch(arguments, resources, [count, 1, 1]) }?);
    Ok(output)
}
