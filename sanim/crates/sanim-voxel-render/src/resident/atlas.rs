//! One bounded mip plan serves byte accounting and streamed per-layer uploads.
mod math;
use std::borrow::Cow;

const CHANNELS: u64 = 4;

#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    pub(super) dimensions: [u32; 2],
    base_bytes: usize,
    pub(super) levels: u32,
    pub(super) gpu_bytes: u64,
    staging_bytes: u64,
    generation_bytes: u64,
}
impl Plan {
    pub(super) fn new(width: u32, height: u32, layers: usize) -> Result<Self, String> {
        if width == 0 || height == 0 || layers == 0 {
            return Err("voxel atlas dimensions and layer count must be positive".into());
        }
        let overflow = || "voxel atlas mip sizes exceed the addressable byte budget".to_owned();
        let layers = u64::try_from(layers).map_err(|_| overflow())?;
        let mut dimensions = [width, height];
        let mut gpu_bytes = 0u64;
        let mut staging_bytes = 0u64;
        let mut generation_bytes = 0u64;
        let mut previous_generated = 0u64;
        let mut levels = 0u32;
        let base_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|bytes| bytes.checked_mul(CHANNELS))
            .ok_or_else(overflow)?;
        let base_bytes = usize::try_from(base_bytes).map_err(|_| overflow())?;
        loop {
            let row = u64::from(dimensions[0])
                .checked_mul(CHANNELS)
                .ok_or_else(overflow)?;
            let bytes = row
                .checked_mul(u64::from(dimensions[1]))
                .ok_or_else(overflow)?;
            gpu_bytes = gpu_bytes
                .checked_add(bytes.checked_mul(layers).ok_or_else(overflow)?)
                .ok_or_else(overflow)?;
            // Queue texture writes may stage rows at the backend's copy alignment.
            let alignment = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let padded =
                row.checked_add(alignment - 1).ok_or_else(overflow)? / alignment * alignment;
            staging_bytes = staging_bytes
                .checked_add(
                    padded
                        .checked_mul(u64::from(dimensions[1]))
                        .and_then(|bytes| bytes.checked_mul(layers))
                        .ok_or_else(overflow)?,
                )
                .ok_or_else(overflow)?;
            if levels != 0 {
                generation_bytes = generation_bytes
                    .max(previous_generated.checked_add(bytes).ok_or_else(overflow)?);
                previous_generated = bytes;
            }
            levels += 1;
            if dimensions == [1, 1] {
                break;
            }
            dimensions = math::next_size(dimensions);
        }
        Ok(Self {
            dimensions: [width, height],
            base_bytes,
            levels,
            gpu_bytes,
            staging_bytes,
            generation_bytes,
        })
    }
    pub(super) fn temporary_bytes(&self, metadata_bytes: u64) -> Result<u64, String> {
        self.staging_bytes
            .checked_add(self.generation_bytes)
            .and_then(|bytes| bytes.checked_add(metadata_bytes))
            .ok_or_else(|| "voxel atlas upload temporary byte count overflowed".into())
    }
    pub(super) fn validate_temporary(&self, metadata_bytes: u64, limit: u64) -> Result<(), String> {
        let bytes = self.temporary_bytes(metadata_bytes)?;
        if bytes > limit {
            return Err(format!(
                "voxel atlas mip upload requires up to {bytes} temporary CPU bytes, exceeding the {limit} byte resource budget; reduce texture resolution or layers"
            ));
        }
        Ok(())
    }
    /// Mip zero borrows the immutable original. Only two adjacent generated
    /// levels can coexist; no CPU mip chain survives a layer's upload.
    pub(super) fn upload_levels(
        &self,
        pixels: &[u8],
        mut upload: impl FnMut(u32, [u32; 2], &[u8]),
    ) {
        assert_eq!(
            pixels.len(),
            self.base_bytes,
            "validated voxel atlas layer byte count changed"
        );
        let mut dimensions = self.dimensions;
        let mut pixels = Cow::Borrowed(pixels);
        for level in 0..self.levels {
            upload(level, dimensions, &pixels);
            if level + 1 < self.levels {
                let (next, data) = math::downsample(dimensions, &pixels);
                dimensions = next;
                pixels = Cow::Owned(data);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangular_non_power_of_two_layers_and_exact_budget_boundaries() {
        for dimensions in [[3, 5], [5, 3], [1, 7], [7, 1], [1, 1]] {
            let plan = Plan::new(dimensions[0], dimensions[1], 3).unwrap();
            let mut sizes = Vec::new();
            let pixels = vec![255; dimensions[0] as usize * dimensions[1] as usize * 4];
            let mut bytes = 0;
            plan.upload_levels(&pixels, |level, size, data| {
                assert_eq!(level as usize, sizes.len());
                assert_eq!(data.len(), size[0] as usize * size[1] as usize * 4);
                if level == 0 {
                    assert!(std::ptr::eq(data.as_ptr(), pixels.as_ptr()));
                }
                sizes.push(size);
                bytes += data.len() as u64;
            });
            assert_eq!(sizes.last(), Some(&[1, 1]));
            assert_eq!(plan.gpu_bytes, 3 * bytes);
            assert_eq!(plan.levels as usize, sizes.len());
            let peak = plan.temporary_bytes(123).unwrap();
            plan.validate_temporary(123, peak).unwrap();
            assert!(
                plan.validate_temporary(123, peak - 1)
                    .unwrap_err()
                    .contains("temporary CPU")
            );
        }
        assert_eq!(Plan::new(3, 5, 2).unwrap().gpu_bytes, 144);
        assert_eq!(
            Plan::new(1, 7, 3).unwrap().temporary_bytes(0).unwrap(),
            8464
        );
        assert!(Plan::new(0, 1, 1).is_err());
        assert!(Plan::new(1, 1, 0).is_err());
        assert!(Plan::new(u32::MAX, u32::MAX, usize::MAX).is_err());
    }

    #[test]
    fn layers_never_mix_and_odd_corner_coverage_survives_without_dark_fringes() {
        let plan = Plan::new(3, 5, 2).unwrap();
        for color in [[0, 80, 220], [230, 50, 10]] {
            let mut pixels = [255, 0, 255, 0].repeat(15);
            pixels[56..60].copy_from_slice(&[color[0], color[1], color[2], 255]);
            let original = pixels.clone();
            let mut last = Vec::new();
            plan.upload_levels(&pixels, |level, _, data| {
                if level == 0 {
                    assert_eq!(data, original);
                }
                last = data.to_vec();
            });
            assert_eq!(last, [color[0], color[1], color[2], 17]);
            assert_eq!(pixels, original);
        }
    }

    #[test]
    #[ignore = "manual cold palette mip-generation cost; no GPU required"]
    fn atlas_mip_generation_benchmark() {
        use std::{hint::black_box, time::Instant};
        for (width, height, layers) in [(16, 16, 256), (256, 128, 32)] {
            let plan = Plan::new(width, height, layers).unwrap();
            let pixels = (0..width * height)
                .flat_map(|pixel| {
                    [
                        pixel as u8,
                        (pixel / 7) as u8,
                        (pixel / 13) as u8,
                        if pixel % 3 == 0 { 0 } else { 255 },
                    ]
                })
                .collect::<Vec<_>>();
            let mut times = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                for _ in 0..layers {
                    plan.upload_levels(&pixels, |level, dimensions, pixels| {
                        black_box((level, dimensions, pixels));
                    });
                }
                times.push(start.elapsed());
            }
            times.sort();
            eprintln!(
                "atlas {width}x{height}x{layers}: cold_mip_generation_us={} mip_levels={} gpu_bytes={} conservative_cpu_temporary_bytes={}",
                times[2].as_micros(),
                plan.levels,
                plan.gpu_bytes,
                plan.temporary_bytes(0).unwrap()
            );
        }
    }
}
