use ffmpeg_next::{color::Range, format::Pixel, frame::Video, sys};

/// Converts decoded, CPU-addressable planes to full-range RGB. Frame-based
/// swscale reads the matrix, range and chroma position from each input frame.
pub struct RgbFrameConverter {
    context: *mut sys::SwsContext,
    format: Pixel,
}

impl RgbFrameConverter {
    pub fn new(format: Pixel) -> Result<Self, String> {
        let context = unsafe { sys::sws_alloc_context() };
        if context.is_null() {
            return Err("Allocate video color converter".into());
        }
        Ok(Self { context, format })
    }

    pub fn convert(&mut self, source: &Video) -> Result<Video, String> {
        let mut rgb = Video::empty();
        rgb.set_format(self.format);
        rgb.set_width(source.width());
        rgb.set_height(source.height());
        rgb.set_color_range(Range::JPEG);
        let result =
            unsafe { sys::sws_scale_frame(self.context, rgb.as_mut_ptr(), source.as_ptr()) };
        if result < 0 {
            return Err(format!(
                "Convert decoded video to RGB: {}",
                ffmpeg_next::Error::from(result)
            ));
        }
        Ok(rgb)
    }
}

impl Drop for RgbFrameConverter {
    fn drop(&mut self) {
        unsafe { sys::sws_free_context(&mut self.context) };
    }
}
