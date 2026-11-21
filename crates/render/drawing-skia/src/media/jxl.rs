//! JPEG XL codestreams and containers share the same color-managed Rust decoder.
use super::animation::{Animation, Decoder};
use jxl_oxide::{
    AllocTracker, EnumColourEncoding, FrameBufferSample, ImageStream, InitializeResult, JxlImage,
    RenderingIntent, image::BitDepth,
};
use shrimply_math_core::Time;
use skia_safe::{AlphaType, Bitmap, ColorSpace, ColorType, Image, ImageInfo};

pub(super) fn recognizes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\xff\x0a") || bytes.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n")
}

pub(super) struct JpegXl(JxlImage);

impl JpegXl {
    pub(super) fn open(bytes: &[u8], max_pixels: usize) -> Result<Self, String> {
        let (mut decoder, remaining) = Self::header(bytes, max_pixels)?;
        decoder.load(remaining)?;
        Ok(decoder)
    }

    fn header(bytes: &[u8], max_pixels: usize) -> Result<(Self, &[u8]), String> {
        if bytes.len() > super::MAX_SOURCE_BYTES {
            return Err("JPEG XL exceeds the image source byte budget".into());
        }
        // Allow working planes for decoding and color conversion; cap retained animation
        // references as well as a single frame's dimensions.
        const WORKING_BYTES_PER_PIXEL: usize = 8 * size_of::<f32>();
        let budget = max_pixels.min(super::MAX_PIXELS) * WORKING_BYTES_PER_PIXEL;
        let tracker = AllocTracker::with_limit(budget.max(super::MAX_SOURCE_BYTES));
        let mut state = JxlImage::builder().alloc_tracker(tracker).build_uninit();
        let mut offset = 0;
        // Stop after the header when probing a still image for animation.
        const HEADER_CHUNK: usize = 4096;
        for start in (0..bytes.len()).step_by(HEADER_CHUNK) {
            let end = (start + HEADER_CHUNK).min(bytes.len());
            offset += state
                .feed_bytes(&bytes[offset..end])
                .map_err(|e| format!("Open JPEG XL: {e}"))?;
            match state.try_init().map_err(|e| format!("Open JPEG XL: {e}"))? {
                InitializeResult::NeedMoreData(next) => state = next,
                InitializeResult::Initialized(mut image) => {
                    super::validate_size([image.width(), image.height()], max_pixels)?;
                    // Moxcms converts embedded ICC profiles before quantization.
                    image.request_color_encoding(EnumColourEncoding::srgb(
                        RenderingIntent::Relative,
                    ));
                    return Ok((Self(image), &bytes[offset..]));
                }
            }
        }
        Err("Incomplete JPEG XL header".into())
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), String> {
        let consumed = self
            .0
            .feed_bytes(bytes)
            .map_err(|e| format!("Read JPEG XL: {e}"))?;
        self.0
            .finalize()
            .map_err(|e| format!("Finish JPEG XL: {e}"))?;
        if consumed != bytes.len()
            || !self.0.is_loading_done()
            || self.0.num_loaded_keyframes() == 0
        {
            return Err("Incomplete JPEG XL image".into());
        }
        Ok(())
    }

    pub(super) fn dimensions(&self) -> [u32; 2] {
        [self.0.width(), self.0.height()]
    }

    pub(super) fn probe_animation(bytes: &[u8]) -> Result<Option<Animation>, String> {
        let (mut image, remaining) = Self::header(bytes, super::MAX_PIXELS)?;
        let Some(timing) = &image.0.image_header().metadata.animation else {
            return Ok(None);
        };
        let (numerator, denominator) = (
            i64::from(timing.tps_numerator),
            i64::from(timing.tps_denominator),
        );
        image.load(remaining)?;
        let mut delays = Vec::with_capacity(image.0.num_loaded_keyframes());
        for index in 0..image.0.num_loaded_keyframes() {
            let frame = image
                .0
                .frame_header(index)
                .ok_or("Missing JPEG XL frame header")?;
            delays.push(Time::from_fraction(
                i64::from(frame.duration) * denominator,
                numerator,
            ));
        }
        Animation::new(Decoder::JpegXl(Box::new(image)), delays).map(Some)
    }

    pub(super) fn frame(&self, index: usize) -> Result<Image, String> {
        let render = self
            .0
            .render_frame(index)
            .map_err(|e| format!("Decode JPEG XL frame {index}: {e}"))?;
        let mut stream = render.stream(); // Applies codestream orientation and frame blending.
        let [width, height] = self.dimensions();
        if (stream.width(), stream.height()) != (width, height) {
            return Err("JPEG XL frame dimensions differ from the canvas".into());
        }
        let metadata = &self.0.image_header().metadata;
        let alpha = metadata.ec_info.iter().find(|ec| ec.is_alpha());
        let alpha_type = match alpha.and_then(|ec| ec.alpha_associated()) {
            None => AlphaType::Opaque,
            Some(false) => AlphaType::Unpremul,
            Some(true) => AlphaType::Premul,
        };
        let mut depths = std::iter::once(metadata.bit_depth).chain(alpha.map(|ec| ec.bit_depth));
        let color_type = if depths.clone().any(|depth| {
            matches!(depth, BitDepth::FloatSample { .. }) || depth.bits_per_sample() > 16
        }) {
            ColorType::RGBAF32
        } else if depths.any(|depth| depth.bits_per_sample() > 8) {
            ColorType::R16G16B16A16UNorm
        } else {
            ColorType::RGBA8888
        };
        let info = ImageInfo::new(
            (width as i32, height as i32),
            color_type,
            alpha_type,
            ColorSpace::new_srgb(),
        );
        let mut bitmap = Bitmap::new();
        if !bitmap.try_alloc_pixels_flags(&info) {
            return Err("Could not allocate JPEG XL frame".into());
        }
        {
            let mut map = bitmap.peek_pixels().ok_or("Missing JPEG XL frame pixels")?;
            let stride = map.row_bytes();
            let pixels = map
                .bytes_mut()
                .ok_or("JPEG XL frame pixels are read-only")?;
            match color_type {
                ColorType::RGBAF32 => write_pixels(&mut stream, pixels, stride, 1.0f32)?,
                ColorType::R16G16B16A16UNorm => {
                    write_pixels(&mut stream, pixels, stride, u16::MAX)?
                }
                _ => write_pixels(&mut stream, pixels, stride, u8::MAX)?,
            }
        }
        Ok(super::bitmap_image(bitmap).0)
    }
}

fn write_pixels<T: FrameBufferSample + bytemuck::Pod>(
    stream: &mut ImageStream<'_>,
    pixels: &mut [u8],
    stride: usize,
    opaque: T,
) -> Result<(), String> {
    let width = stream.width() as usize;
    let channels = stream.channels() as usize;
    if !matches!(channels, 3 | 4) {
        return Err("JPEG XL could not be converted to sRGB".into());
    }
    let mut rgb = if channels == 3 {
        vec![opaque; width * 3]
    } else {
        Vec::new()
    };
    for row in pixels.chunks_mut(stride) {
        let row = bytemuck::try_cast_slice_mut::<_, T>(&mut row[..width * 4 * size_of::<T>()])
            .map_err(|e| format!("Invalid JPEG XL bitmap stride: {e}"))?;
        let samples = if channels == 3 {
            &mut rgb[..]
        } else {
            &mut *row
        };
        if stream.write_to_buffer(samples) != samples.len() {
            return Err("Incomplete JPEG XL frame pixels".into());
        }
        if channels == 3 {
            for (rgba, rgb) in row.chunks_exact_mut(4).zip(rgb.chunks_exact(3)) {
                rgba.copy_from_slice(&[rgb[0], rgb[1], rgb[2], opaque]);
            }
        }
    }
    Ok(())
}
