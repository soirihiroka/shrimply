//! A seekable GIF uses exact frame delays and retains only the last decoded bitmap.
use super::animation::Animation;
use shrimply_math_core::Time;
use skia_safe::{AlphaType, Codec, ColorType, Data, Image, ImageInfo, codec};

pub struct Gif {
    codec: Codec<'static>,
    info: ImageInfo,
}

impl Gif {
    pub(super) fn open(bytes: &[u8]) -> Result<Animation, String> {
        if bytes.len() > super::MAX_SOURCE_BYTES {
            return Err("GIF exceeds the image source byte budget".into());
        }
        let mut codec = Codec::from_data_with_decoders(
            Data::new_copy(bytes),
            &[skia_safe::codec::gif_decoder::decoder()],
        )
        .ok_or("Invalid GIF")?;
        let info = codec
            .info()
            .with_color_type(ColorType::RGBA8888)
            .with_alpha_type(AlphaType::Unpremul);
        super::validate_size(
            [info.width() as u32, info.height() as u32],
            super::MAX_PIXELS,
        )?;
        let count = codec.get_frame_count();
        if count == 0 {
            return Err("GIF contains no frames".into());
        }
        let delays = (0..count)
            .map(|index| {
                let frame = codec
                    .get_frame_info(index)
                    .ok_or("Missing GIF frame header")?;
                if !frame.fully_received {
                    return Err("Incomplete GIF frame".into());
                }
                // Preserve FFmpeg's GIF timing for missing delays and single-tick frames.
                const MIN_DELAY_MS: i32 = 20;
                const DEFAULT_DELAY_MS: i32 = 100;
                let milliseconds = if frame.duration < MIN_DELAY_MS {
                    DEFAULT_DELAY_MS
                } else {
                    frame.duration
                };
                Ok(Time::from_fraction(i64::from(milliseconds), 1000))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Animation::new(super::animation::Decoder::Gif(Self { codec, info }), delays)
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.info.width() as u32, self.info.height() as u32]
    }

    pub(super) fn frame(
        &mut self,
        index: usize,
        previous: Option<&(usize, Image)>,
    ) -> Result<Image, String> {
        let mut bitmap = skia_safe::Bitmap::new();
        if !bitmap.try_alloc_pixels_flags(&self.info) {
            return Err("Could not allocate GIF frame".into());
        }
        {
            let mut pixmap = bitmap
                .peek_pixels()
                .ok_or("Allocated GIF frame has no pixels")?;
            let stride = pixmap.row_bytes();
            let pixels = pixmap
                .bytes_mut()
                .ok_or("Allocated GIF frame is read-only")?;
            let mut options = codec::Options {
                frame_index: index,
                ..Default::default()
            };
            if let Some((last, image)) = previous
                && *last < index
                && let Some(frame) = self.codec.get_frame_info(index)
                && frame.required_frame >= 0
                && *last >= frame.required_frame as usize
                && self.codec.get_frame_info(*last).is_some_and(|frame| {
                    frame.disposal_method != codec::codec_animation::DisposalMethod::RestorePrevious
                })
                && image.read_pixels(
                    &self.info,
                    pixels,
                    stride,
                    (0, 0),
                    skia_safe::image::CachingHint::Allow,
                )
            {
                options.prior_frame = Some(*last);
            }
            let result =
                self.codec
                    .get_pixels_with_options(&self.info, pixels, stride, Some(&options));
            if result != codec::Result::Success {
                return Err(format!("GIF frame {index} is incomplete: {result:?}"));
            }
        }
        bitmap.set_immutable();
        Ok(bitmap.as_image())
    }
}
