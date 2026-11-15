use crate::RgbaImage;

/// OpenCV VideoCaptureBase::applyMetadataRotation and cv::rotate quarter turns.
/// Other angles are left unchanged, as in OpenCV's metadata rotation policy.
pub fn rotate_rgba(image: RgbaImage, clockwise_degrees: i32) -> Result<RgbaImage, String> {
    image.validate()?;
    let turns = match clockwise_degrees {
        90 | -270 => 1,
        180 | -180 => 2,
        270 | -90 => 3,
        _ => return Ok(image),
    };
    let (width, height) = if turns % 2 == 1 {
        (image.height, image.width)
    } else {
        (image.width, image.height)
    };
    let mut data = vec![0; image.data.len()];
    for (index, pixel) in image.data.chunks_exact(4).enumerate() {
        let (x, y) = (index % image.width, index / image.width);
        let (x, y) = match turns {
            1 => (image.height - 1 - y, x),
            2 => (image.width - 1 - x, image.height - 1 - y),
            3 => (y, image.width - 1 - x),
            _ => unreachable!("quarter turn validated"),
        };
        data[(y * width + x) * 4..(y * width + x + 1) * 4].copy_from_slice(pixel);
    }
    Ok(RgbaImage {
        width,
        height,
        data,
    })
}
