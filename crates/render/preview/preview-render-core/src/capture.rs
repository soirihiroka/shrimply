/// Readback for explicit image capture; ordinary preview presentation stays on the GPU.
pub struct RgbaFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn png(&self) -> Result<Vec<u8>, String> {
        shrimply_drawing_skia::media::encode_rgba_png(self.width, self.height, &self.pixels)
    }
}
