use crate::{Buffer, Renderer, Submission};

const MAX_PYRAMID_LEVELS: usize = 5;
const MIN_PYRAMID_EDGE: u32 = 8;

pub struct OpticalFlow {
    pub forward: Buffer,
    pub backward: Buffer,
    pub grid: (u32, u32),
}

impl Renderer {
    pub fn optical_flow(
        &mut self,
        source: Buffer,
        target: Buffer,
        size: (u32, u32),
        submissions: &mut Vec<Submission>,
    ) -> Result<OpticalFlow, String> {
        let length = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|n| n.checked_mul(4));
        if size.0 < 2 || size.1 < 2 || length != Some(source.len()) || length != Some(target.len())
        {
            return Err("Invalid optical-flow endpoint dimensions".into());
        }
        let mut pyramid = vec![(source, target, size)];
        while pyramid.len() < MAX_PYRAMID_LEVELS {
            let (source, target, size) = pyramid.last().expect("flow pyramid has endpoints");
            if size.0.min(size.1) <= MIN_PYRAMID_EDGE {
                break;
            }
            let next_size = (size.0.div_ceil(2), size.1.div_ceil(2));
            let mut downsample = |input: &Buffer| -> Result<Buffer, String> {
                let count = next_size.0 as usize * next_size.1 as usize;
                let output = self.allocate(count * 4)?;
                let mut args = self.arguments("flow_downsample")?;
                args.set("input", &input.address().to_ne_bytes())?
                    .set("output", &output.address().to_ne_bytes())?
                    .set("width", &size.0.to_ne_bytes())?
                    .set("height", &size.1.to_ne_bytes())?
                    .set("output_width", &next_size.0.to_ne_bytes())?
                    .set("output_height", &next_size.1.to_ne_bytes())?;
                submissions.push(unsafe {
                    self.dispatch(args, vec![input.clone(), output.clone()], [count, 1, 1])
                }?);
                Ok(output)
            };
            let source = downsample(source)?;
            let target = downsample(target)?;
            pyramid.push((source, target, next_size));
        }
        let mut previous: Option<OpticalFlow> = None;
        for (source, target, size) in pyramid.into_iter().rev() {
            let grid_size = shrimply_math_geometry::optical_flow_grid_size(size.0, size.1)?;
            let grid = (grid_size.x, grid_size.y);
            let count = grid.0 as usize * grid.1 as usize;
            let mut estimate = |source: &Buffer,
                                target: &Buffer,
                                previous_buffer: Option<&Buffer>|
             -> Result<Buffer, String> {
                let output = self.allocate(count * size_of::<[f32; 2]>())?;
                // This address is not dereferenced when has_previous is false.
                let prior = previous_buffer.unwrap_or(source);
                let previous_grid = previous.as_ref().map_or((0, 0), |previous| previous.grid);
                let mut args = self.arguments("estimate_flow")?;
                args.set("source", &source.address().to_ne_bytes())?
                    .set("target", &target.address().to_ne_bytes())?
                    .set("previous", &prior.address().to_ne_bytes())?
                    .set("output", &output.address().to_ne_bytes())?
                    .set("width", &size.0.to_ne_bytes())?
                    .set("height", &size.1.to_ne_bytes())?
                    .set("grid_width", &grid.0.to_ne_bytes())?
                    .set("grid_height", &grid.1.to_ne_bytes())?
                    .set("previous_width", &previous_grid.0.to_ne_bytes())?
                    .set("previous_height", &previous_grid.1.to_ne_bytes())?
                    .set("has_previous", &[u8::from(previous_buffer.is_some())])?;
                submissions.push(unsafe {
                    self.dispatch(
                        args,
                        vec![
                            source.clone(),
                            target.clone(),
                            prior.clone(),
                            output.clone(),
                        ],
                        [count, 1, 1],
                    )
                }?);
                Ok(output)
            };
            let forward = estimate(
                &source,
                &target,
                previous.as_ref().map(|flow| &flow.forward),
            )?;
            let backward = estimate(
                &target,
                &source,
                previous.as_ref().map(|flow| &flow.backward),
            )?;
            previous = Some(OpticalFlow {
                forward,
                backward,
                grid,
            });
        }
        previous.ok_or_else(|| "Optical-flow pyramid is empty".into())
    }
}
