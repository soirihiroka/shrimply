use glam::{UVec2, Vec2};

pub use shrimply_math_geometry::{FLOW_GRID_SPACING, optical_flow_grid_size as regular_grid_size};

/// Backend-neutral dense motion represented as source-pixel displacements on a
/// regular grid. Hardware backends own estimation and resource execution.
#[derive(Clone, Debug)]
pub struct OpticalFlowField {
    pub grid_size: UVec2,
    pub forward: Vec<Vec2>,
    pub backward: Vec<Vec2>,
}

#[derive(Clone, Debug)]
pub struct Presentation {
    pub grid_size: UVec2,
    pub source_offsets: Vec<Vec2>,
    pub target_offsets: Vec<Vec2>,
    pub target_opacity: f32,
}

impl OpticalFlowField {
    pub fn new(grid_size: UVec2, forward: Vec<Vec2>, backward: Vec<Vec2>) -> Result<Self, String> {
        let count = grid_size
            .x
            .checked_mul(grid_size.y)
            .and_then(|count| usize::try_from(count).ok())
            .ok_or("Morph optical-flow grid size overflow")?;
        if grid_size.x < 2
            || grid_size.y < 2
            || forward.len() != count
            || backward.len() != count
            || forward
                .iter()
                .chain(&backward)
                .any(|value| !value.is_finite())
        {
            return Err("Morph optical-flow grid is invalid".into());
        }
        Ok(Self {
            grid_size,
            forward,
            backward,
        })
    }

    pub fn presentation(&self, progress: f32) -> Presentation {
        let progress = progress.clamp(0.0, 1.0);
        Presentation {
            grid_size: self.grid_size,
            source_offsets: scaled_source_offsets(&self.forward, progress),
            target_offsets: scaled_source_offsets(&self.backward, 1.0 - progress),
            target_opacity: progress,
        }
    }
}

pub fn scaled_source_offsets(flow: &[Vec2], amount: f32) -> Vec<Vec2> {
    let scale = -amount.clamp(0.0, 1.0);
    flow.iter().map(|value| *value * scale).collect()
}
