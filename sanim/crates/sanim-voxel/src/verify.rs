//! Pure checks on resolved voxel layout snapshots. Cutout textures do not erase occupied cells.
mod math;
pub use math::{assert_disjoint, assert_framed, assert_within};

/// Bounds CPU work independently of the size of the containing empty volume.
pub const MAX_VERIFY_WORK: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    pub work: usize,
    pub pairs: usize,
    pub tested_cells: usize,
}

struct Budget {
    statistics: Statistics,
    maximum: usize,
}
impl Budget {
    fn new(maximum: usize) -> Self {
        Self {
            statistics: Statistics::default(),
            maximum: maximum.min(MAX_VERIFY_WORK),
        }
    }
    fn charge(&mut self) -> Result<(), String> {
        if self.statistics.work >= self.maximum {
            return Err(format!(
                "voxel assertion exceeds its {}-operation verification budget; select smaller parts or simplify their occupied bricks",
                self.maximum
            ));
        }
        self.statistics.work += 1;
        Ok(())
    }
}
