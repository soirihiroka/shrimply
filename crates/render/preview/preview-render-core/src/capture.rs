/// Readback for explicit image capture; ordinary preview presentation stays on the GPU.
pub struct RgbaFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let info = skia_safe::ImageInfo::new(
            (self.width, self.height),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let image = skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(&self.pixels),
            usize::try_from(self.width).map_err(|_| "Invalid captured image width")?
                * std::mem::size_of::<u32>(),
        )
        .ok_or("Could not construct the captured image")?;
        image
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .map(|data| data.as_bytes().to_vec())
            .ok_or_else(|| "Could not encode the captured image".into())
    }
}
