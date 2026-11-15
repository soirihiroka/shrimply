use glam::{UVec2, Vec2};

pub use crate::{FLOW_GRID_SPACING, optical_flow_grid_size as regular_grid_size};

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

    /// Keep bidirectionally consistent image correspondences for camera motion.
    pub fn reliable_tracks(&self, size: UVec2) -> Vec<[f32; 6]> {
        const SAMPLE_SPACING: u32 = 16;
        const BORDER: f32 = 12.0;
        const CYCLE_ERROR_PIXELS: f32 = 1.5;
        const CYCLE_ERROR_MOTION_RATIO: f32 = 0.05;
        if size.min_element() < 2 {
            return Vec::new();
        }
        let extent = (size - UVec2::ONE).as_vec2();
        let grid_extent = (self.grid_size - UVec2::ONE).as_vec2();
        let stride = (SAMPLE_SPACING / FLOW_GRID_SPACING).max(1) as usize;
        let mut tracks = Vec::new();
        for row in (0..self.grid_size.y).step_by(stride) {
            for column in (0..self.grid_size.x).step_by(stride) {
                let position = UVec2::new(column, row).as_vec2() / grid_extent * extent;
                if position.min_element() < BORDER || (extent - position).min_element() < BORDER {
                    continue;
                }
                let forward = self.forward[(row * self.grid_size.x + column) as usize];
                let destination = position + forward;
                if destination.min_element() < 0.0 || !destination.cmple(extent).all() {
                    continue;
                }
                let backward_grid = (destination / extent * grid_extent).round().as_uvec2();
                let backward =
                    self.backward[(backward_grid.y * self.grid_size.x + backward_grid.x) as usize];
                if (forward + backward).length()
                    > CYCLE_ERROR_PIXELS + forward.length() * CYCLE_ERROR_MOTION_RATIO
                {
                    continue;
                }
                tracks.push([
                    position.x,
                    position.y,
                    destination.x,
                    destination.y,
                    1.0,
                    1.0,
                ]);
            }
        }
        tracks
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
