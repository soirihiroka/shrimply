//! Shared packing and validation for the source-derived sparse LK shader.
//! Native backends only upload these buffers, dispatch, and return the output.
use crate::tracking::{pyramid, scharr, validate_flow};
use crate::{FlowOptions, GrayImage, Track};

const LEVEL_WORDS: usize = 4;
const POINT_COMPONENTS: usize = 2;
const RESULT_COMPONENTS: usize = 4;
const GRADIENT_COMPONENTS: usize = 2;

pub struct TrackingJob {
    pub previous: Vec<u8>,
    pub current: Vec<u8>,
    pub derivatives: Vec<u8>,
    pub levels: Vec<u8>,
    pub points: Vec<u8>,
    pub count: u32,
    pub level_count: u32,
    pub window: u32,
    pub iterations: u32,
    pub epsilon_squared: f32,
    pub minimum_eigenvalue: f32,
    original_points: Vec<[f32; POINT_COMPONENTS]>,
    patch_length: usize,
}

impl TrackingJob {
    pub fn new(
        previous: &GrayImage,
        current: &GrayImage,
        points: &[[f32; POINT_COMPONENTS]],
        options: FlowOptions,
    ) -> Result<Self, String> {
        validate_flow(previous, current, points, options)?;
        let count = u32::try_from(points.len()).map_err(|_| "too many optical flow features")?;
        if count == 0 {
            return Err("GPU optical flow needs at least one feature".into());
        }
        let early = pyramid(previous, options);
        let late = pyramid(current, options);
        let level_count = early.len().min(late.len());
        let pixels = early
            .iter()
            .take(level_count)
            .try_fold(0_usize, |sum, image| {
                sum.checked_add(image.data.len())
                    .ok_or("optical flow pyramid size overflow")
            })?;
        u32::try_from(pixels).map_err(|_| "optical flow pyramid exceeds GPU indexing")?;
        let patch_length = points
            .len()
            .checked_mul(options.window)
            .and_then(|value| value.checked_mul(options.window))
            .ok_or("optical flow scratch size overflow")?;
        u32::try_from(patch_length).map_err(|_| "optical flow scratch exceeds GPU indexing")?;
        let image_bytes = pixels
            .checked_mul(size_of::<u32>())
            .ok_or("optical flow pyramid byte size overflow")?;
        let derivative_bytes = image_bytes
            .checked_mul(GRADIENT_COMPONENTS)
            .ok_or("optical flow derivative byte size overflow")?;
        let mut job = Self {
            previous: Vec::with_capacity(image_bytes),
            current: Vec::with_capacity(image_bytes),
            derivatives: Vec::with_capacity(derivative_bytes),
            levels: Vec::with_capacity(level_count * LEVEL_WORDS * size_of::<u32>()),
            points: Vec::with_capacity(points.len() * POINT_COMPONENTS * size_of::<f32>()),
            count,
            level_count: level_count as u32,
            window: options.window as u32,
            iterations: options.iterations.min(100) as u32,
            epsilon_squared: options.epsilon.clamp(0.0, 10.0).powi(2) as f32,
            minimum_eigenvalue: options.minimum_eigenvalue as f32,
            original_points: points.to_vec(),
            patch_length,
        };
        let mut offset = 0_u32;
        for (source, target) in early.iter().zip(&late).take(level_count) {
            for value in [source.width as u32, source.height as u32, offset, 0] {
                job.levels.extend_from_slice(&value.to_ne_bytes());
            }
            for &value in &source.data {
                job.previous
                    .extend_from_slice(&u32::from(value).to_ne_bytes());
            }
            for &value in &target.data {
                job.current
                    .extend_from_slice(&u32::from(value).to_ne_bytes());
            }
            for derivative in scharr(source) {
                for value in derivative {
                    job.derivatives
                        .extend_from_slice(&i32::from(value).to_ne_bytes());
                }
            }
            offset += source.data.len() as u32;
        }
        for point in points {
            for value in point {
                job.points.extend_from_slice(&value.to_ne_bytes());
            }
        }
        Ok(job)
    }

    pub fn patch_bytes(&self) -> usize {
        self.patch_length * size_of::<i32>()
    }

    pub fn gradient_bytes(&self) -> usize {
        self.patch_length * GRADIENT_COMPONENTS * size_of::<i32>()
    }

    pub fn output_bytes(&self) -> usize {
        self.count as usize * RESULT_COMPONENTS * size_of::<f32>()
    }

    pub fn finish(&self, output: &[u8]) -> Result<Vec<Track>, String> {
        if output.len() != self.output_bytes() {
            return Err("GPU optical flow returned an invalid result size".into());
        }
        self.original_points
            .iter()
            .zip(output.chunks_exact(RESULT_COMPONENTS * size_of::<f32>()))
            .map(|(&previous, bytes)| {
                let values = std::array::from_fn::<_, RESULT_COMPONENTS, _>(|i| {
                    f32::from_ne_bytes(
                        bytes[i * size_of::<f32>()..(i + 1) * size_of::<f32>()]
                            .try_into()
                            .expect("fixed-size optical flow component"),
                    )
                });
                if values[3] != 0.0 && values[3] != 1.0 {
                    return Err("GPU optical flow returned an invalid feature status".into());
                }
                Ok(Track {
                    previous,
                    current: [values[0], values[1]],
                    valid: values[3] == 1.0,
                    error: values[2],
                })
            })
            .collect()
    }
}
