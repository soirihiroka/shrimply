use shrimply_project_document::project::{VideoItem, VideoItemContent};
use shrimply_visual_modifiers::{
    ModifierEffect,
    vectorize::{
        MAX_ANGLE_DEGREES, MAX_BINARY_THRESHOLD, MAX_COLOR_PRECISION, MAX_GRADIENT_STEP,
        MAX_ITERATIONS, MAX_PATH_PRECISION, MAX_SEGMENT_LENGTH, MAX_SPECKLE_SIZE,
        MIN_COLOR_PRECISION, MIN_SEGMENT_LENGTH, VectorizeColorMode, VectorizeHierarchy,
        VectorizeModifier, VectorizePathMode,
    },
};

const RGBA_CHANNELS: usize = 4;

#[derive(Clone, Debug)]
pub struct Plan {
    key: Vec<u8>,
    modifier: VectorizeModifier,
}

impl Plan {
    pub fn modifier_for_item(item: &VideoItem) -> Option<&VectorizeModifier> {
        if !matches!(item.content, VideoItemContent::Image) {
            return None;
        }
        item.modifiers
            .iter()
            .find(|modifier| modifier.enabled)
            .and_then(|modifier| match &modifier.effect {
                ModifierEffect::Vectorize(modifier) => Some(modifier),
                _ => None,
            })
    }

    pub fn for_item(item: &VideoItem, source_cache_key: &str) -> Result<Option<Self>, String> {
        let Some(modifier) = Self::modifier_for_item(item) else {
            return Ok(None);
        };
        Self::new(modifier.clone(), source_cache_key).map(Some)
    }

    pub fn new(modifier: VectorizeModifier, source_cache_key: &str) -> Result<Self, String> {
        let mut key = serde_json::to_vec(&modifier)
            .map_err(|error| format!("serialize Vectorize request: {error}"))?;
        key.extend_from_slice(source_cache_key.as_bytes());
        Ok(Self { key, modifier })
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn vectorize_rgba(
        &self,
        mut pixels: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<Output, String> {
        let width = usize::try_from(width).map_err(|_| "Vectorize width exceeds usize")?;
        let height = usize::try_from(height).map_err(|_| "Vectorize height exceeds usize")?;
        if width == 0 || height == 0 {
            return Err("Vectorize source dimensions must be non-zero".into());
        }
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(RGBA_CHANNELS))
            .ok_or("Vectorize source dimensions overflow")?;
        if pixels.len() != expected {
            return Err(format!(
                "Vectorize requires tightly packed RGBA pixels: expected {expected} bytes, received {}",
                pixels.len()
            ));
        }
        if self.modifier.color_mode == VectorizeColorMode::BlackAndWhite {
            for pixel in pixels.as_chunks_mut::<RGBA_CHANNELS>().0 {
                let value = if pixel[3] == 0
                    || shrimply_math_color::Color::<u8>::from([pixel[0], pixel[1], pixel[2]])
                        .rec709_luma()
                        >= self.modifier.binary_threshold.min(MAX_BINARY_THRESHOLD) as u8
                {
                    u8::MAX
                } else {
                    u8::MIN
                };
                pixel[..3].fill(value);
            }
        }
        let config = vtracer::Config {
            clustering: match self.modifier.color_mode {
                VectorizeColorMode::Color => vtracer::Clustering::ColorCluster,
                VectorizeColorMode::BlackAndWhite => vtracer::Clustering::Binary,
            },
            hierarchical: match self.modifier.hierarchy {
                VectorizeHierarchy::Stacked => vtracer::Hierarchical::Stacked,
                VectorizeHierarchy::Cutout => vtracer::Hierarchical::Cutout,
            },
            filter_speckle: self.modifier.speckle_size.min(MAX_SPECKLE_SIZE) as usize,
            color_precision: self
                .modifier
                .color_precision
                .clamp(MIN_COLOR_PRECISION, MAX_COLOR_PRECISION)
                as i32,
            layer_difference: self.modifier.gradient_step.min(MAX_GRADIENT_STEP) as i32,
            mode: match self.modifier.path_mode {
                VectorizePathMode::Pixel => vtracer::FitMode::Pixel,
                VectorizePathMode::Polygon => vtracer::FitMode::Polygon,
                VectorizePathMode::Spline => vtracer::FitMode::Spline,
            },
            corner_threshold: self
                .modifier
                .corner_threshold_degrees
                .min(MAX_ANGLE_DEGREES) as i32,
            length_threshold: self
                .modifier
                .segment_length
                .clamp(MIN_SEGMENT_LENGTH, MAX_SEGMENT_LENGTH) as f64,
            max_iterations: self.modifier.max_iterations.min(MAX_ITERATIONS) as usize,
            splice_threshold: self
                .modifier
                .splice_threshold_degrees
                .min(MAX_ANGLE_DEGREES) as i32,
            path_precision: Some(self.modifier.path_precision.min(MAX_PATH_PRECISION)),
            ..vtracer::Config::default()
        };
        let image = vtracer::ColorImage {
            pixels,
            width,
            height,
        };
        let svg = config
            .build()
            .and_then(|pipeline| pipeline.to_svg(&image))
            .map_err(|error| format!("could not vectorize image: {error}"))?;
        Ok(Output {
            key: self.key.clone(),
            svg,
            width: width as u32,
            height: height as u32,
        })
    }
}

pub struct Output {
    pub key: Vec<u8>,
    pub svg: String,
    pub width: u32,
    pub height: u32,
}
