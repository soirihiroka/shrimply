use crate::{Buffer, Renderer, Submission};

impl Renderer {
    /// Filter HDR float4 color with float4(depth, normal) guides, preserving alpha.
    /// The input is also used as scratch; the final result is written to output.
    /// Three GPU passes cover progressively wider neighborhoods without readback.
    pub fn denoise_atrous(
        &mut self,
        input: &Buffer,
        output: &Buffer,
        depth_normal: &Buffer,
        size: (u32, u32),
        submissions: &mut Vec<Submission>,
    ) -> Result<(), String> {
        let length = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|count| count.checked_mul(size_of::<[f32; 4]>()));
        if size.0 == 0
            || size.1 == 0
            || length != Some(input.len())
            || length != Some(output.len())
            || length != Some(depth_normal.len())
            || input.address() == output.address()
            || depth_normal.address() == input.address()
            || depth_normal.address() == output.address()
        {
            return Err("Invalid GPU denoiser buffers".into());
        }
        for (pass, step) in [1_u32, 2, 4].into_iter().enumerate() {
            let (source, target) = if pass % 2 == 0 {
                (input, output)
            } else {
                (output, input)
            };
            let mut args = self.arguments("denoise_atrous")?;
            args.set("input", &source.address().to_ne_bytes())?
                .set("output", &target.address().to_ne_bytes())?
                .set("depth_normal", &depth_normal.address().to_ne_bytes())?
                .set("width", &size.0.to_ne_bytes())?
                .set("height", &size.1.to_ne_bytes())?
                .set("step", &step.to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    vec![source.clone(), target.clone(), depth_normal.clone()],
                    [size.0 as usize, size.1 as usize, 1],
                )
            }?);
        }
        Ok(())
    }
}
