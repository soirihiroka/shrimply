//! Surface selection shares one bounded height table with scatter and surface paint.
use super::checked_work;
use crate::{
    Volume,
    procedural::{Field, MAX_SURFACE_POINTS, math},
};

#[derive(Clone, Copy, Debug)]
pub struct SurfacePoints<'a> {
    pub step: u32,
    pub seed: u64,
    /// Fraction of each sampling tile covered by deterministic integer-column jitter.
    pub jitter: f64,
    pub density: Option<&'a Field>,
    pub mask: Option<&'a Field>,
    /// Maximum local height rise per cell, measured against four adjacent columns.
    pub max_slope: Option<f64>,
}
impl Default for SurfacePoints<'_> {
    fn default() -> Self {
        Self {
            step: 4,
            seed: 0,
            jitter: 0.0,
            density: None,
            mask: None,
            max_slope: None,
        }
    }
}
impl SurfacePoints<'_> {
    fn candidates(&self, dimensions: [u32; 3]) -> Result<usize, String> {
        if self.step == 0 {
            return Err("surface point spacing must be positive".into());
        }
        if !self.jitter.is_finite() || !(0.0..=1.0).contains(&self.jitter) {
            return Err("surface point jitter must be finite within 0..=1".into());
        }
        if self
            .max_slope
            .is_some_and(|slope| !slope.is_finite() || slope < 0.0)
        {
            return Err("surface point max_slope must be finite and nonnegative".into());
        }
        Ok(dimensions[0].div_ceil(self.step) as usize * dimensions[2].div_ceil(self.step) as usize)
    }
}
impl Volume {
    pub(super) fn surface_sampling_work(
        &self,
        options: &SurfacePoints<'_>,
    ) -> Result<usize, String> {
        let count = options.candidates(self.dimensions())?;
        let per_point = 1
            + usize::from(options.jitter > 0.0) * 2
            + usize::from(options.max_slope.is_some()) * 5
            + options.mask.map_or(0, Field::sample_work)
            + options.density.map_or(0, Field::sample_work);
        checked_work(
            self.surface_work()
                .saturating_add(count.saturating_mul(per_point)),
        )
    }
    pub fn surface_points_work(&self, options: &SurfacePoints<'_>) -> Result<usize, String> {
        let count = options.candidates(self.dimensions())?;
        if count > MAX_SURFACE_POINTS {
            return Err(format!(
                "surface sampling may produce {count} points, exceeding {MAX_SURFACE_POINTS}; increase step"
            ));
        }
        self.surface_sampling_work(options)
    }
    pub(super) fn visit_surface_points(
        &self,
        options: &SurfacePoints<'_>,
        max_height: u32,
        mut visit: impl FnMut([u32; 3]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.surface_sampling_work(options)?;
        let [width, _, depth] = self.dimensions();
        let heights = self.surface_heights();
        let mut mask = options.mask.map(Field::sampler);
        let mut density = options.density.map(Field::sampler);
        for z in (0..depth).step_by(options.step as usize) {
            for x in (0..width).step_by(options.step as usize) {
                let [x, z] = math::surface_point(
                    options.seed,
                    [x, z],
                    [width, depth],
                    options.step,
                    options.jitter,
                );
                let y = u32::from(heights[(x + width * z) as usize]);
                if y == 0 || y > max_height {
                    continue;
                }
                if options.max_slope.is_some_and(|limit| {
                    math::surface_slope(&heights, [width, depth], [x, z]) > limit
                }) {
                    continue;
                }
                let point = [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5];
                if let Some(mask) = &mut mask
                    && mask.sample(point)? <= 0.0
                {
                    continue;
                }
                let probability = match &mut density {
                    Some(field) => field.sample(point)?,
                    None => 1.0,
                };
                if math::scatter(options.seed, [x, z], probability) {
                    visit([x, y, z])?;
                }
            }
        }
        Ok(())
    }
    pub fn surface_points(&self, options: &SurfacePoints<'_>) -> Result<Vec<[u32; 3]>, String> {
        self.surface_points_work(options)?;
        let mut points = Vec::new();
        self.visit_surface_points(options, self.dimensions()[1], |point| {
            points.push(point);
            Ok(())
        })?;
        Ok(points)
    }
}
