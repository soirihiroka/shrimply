//! Shared packing and validation for the source-derived sparse LK shader.
//! Native backends only upload these buffers, dispatch, and return the output.
use crate::tracking::{pyramid_sizes, validate_flow};
use crate::{FlowOptions, GrayImage, Track};

const LEVEL_WORDS: usize = 4;
const POINT_COMPONENTS: usize = 2;
const RESULT_COMPONENTS: usize = 4;
const GRADIENT_COMPONENTS: usize = 2;
/// One cooperative workgroup tracks each feature. Keep in sync with tracking.slang.
pub const WORKGROUP_SIZE: u32 = 64;
pub const MAX_WINDOW: usize = 20;
pub const PREPARATION_GROUP_SIZE: u32 = 256;

#[derive(Clone, Copy, Debug)]
pub enum PreparationStep {
    Pyramid {
        previous: bool,
        level: u32,
        threads: usize,
    },
    Derivatives {
        level: u32,
        threads: usize,
    },
}

pub struct TrackingJob {
    pub previous: Vec<u8>,
    pub current: Vec<u8>,
    pub derivative_bytes: usize,
    pub preparation: Vec<PreparationStep>,
    pub levels: Vec<u8>,
    pub points: Vec<u8>,
    pub count: u32,
    pub level_count: u32,
    pub window: u32,
    pub iterations: u32,
    pub epsilon_squared: f32,
    pub minimum_eigenvalue: f32,
    original_points: Vec<[f32; POINT_COMPONENTS]>,
}

impl TrackingJob {
    pub fn new(
        previous: &GrayImage,
        current: &GrayImage,
        points: &[[f32; POINT_COMPONENTS]],
        options: FlowOptions,
    ) -> Result<Self, String> {
        validate_flow(previous, current, points, options)?;
        if options.window > MAX_WINDOW {
            return Err("GPU optical flow window exceeds shared memory capacity".into());
        }
        let count = u32::try_from(points.len()).map_err(|_| "too many optical flow features")?;
        count
            .checked_mul(WORKGROUP_SIZE)
            .ok_or("optical flow dispatch exceeds GPU indexing")?;
        if count == 0 {
            return Err("GPU optical flow needs at least one feature".into());
        }
        let sizes = pyramid_sizes(previous.width, previous.height, options);
        let level_count = sizes.len();
        let pixels = sizes.iter().try_fold(0_usize, |sum, &(width, height)| {
            sum.checked_add(width * height)
                .ok_or("optical flow pyramid size overflow")
        })?;
        u32::try_from(pixels).map_err(|_| "optical flow pyramid exceeds GPU indexing")?;
        let image_bytes = pixels
            .checked_mul(size_of::<u32>())
            .ok_or("optical flow pyramid byte size overflow")?;
        let derivative_bytes = image_bytes
            .checked_mul(GRADIENT_COMPONENTS)
            .ok_or("optical flow derivative byte size overflow")?;
        let mut job = Self {
            previous: Vec::with_capacity(image_bytes),
            current: Vec::with_capacity(image_bytes),
            derivative_bytes,
            preparation: Vec::with_capacity(level_count * 3),
            levels: Vec::with_capacity(level_count * LEVEL_WORDS * size_of::<u32>()),
            points: Vec::with_capacity(points.len() * POINT_COMPONENTS * size_of::<f32>()),
            count,
            level_count: level_count as u32,
            window: options.window as u32,
            iterations: options.iterations.min(100) as u32,
            epsilon_squared: options.epsilon.clamp(0.0, 10.0).powi(2) as f32,
            minimum_eigenvalue: options.minimum_eigenvalue as f32,
            original_points: points.to_vec(),
        };
        for &value in &previous.data {
            job.previous
                .extend_from_slice(&u32::from(value).to_ne_bytes());
        }
        for &value in &current.data {
            job.current
                .extend_from_slice(&u32::from(value).to_ne_bytes());
        }
        job.previous.resize(image_bytes, 0);
        job.current.resize(image_bytes, 0);
        let mut offset = 0_u32;
        for (index, (width, height)) in sizes.into_iter().enumerate() {
            for value in [width as u32, height as u32, offset, 0] {
                job.levels.extend_from_slice(&value.to_ne_bytes());
            }
            let level = index as u32;
            let threads = width * height;
            if index != 0 {
                for previous in [true, false] {
                    job.preparation.push(PreparationStep::Pyramid {
                        previous,
                        level,
                        threads,
                    });
                }
            }
            job.preparation
                .push(PreparationStep::Derivatives { level, threads });
            offset += threads as u32;
        }
        for point in points {
            for value in point {
                job.points.extend_from_slice(&value.to_ne_bytes());
            }
        }
        Ok(job)
    }

    pub fn dispatch_extent(&self) -> [usize; 3] {
        [self.count as usize * WORKGROUP_SIZE as usize, 1, 1]
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
            .zip(
                output
                    .as_chunks::<{ RESULT_COMPONENTS * size_of::<f32>() }>()
                    .0,
            )
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
