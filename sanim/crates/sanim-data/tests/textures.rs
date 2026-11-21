use image::{
    ExtendedColorType, ImageEncoder,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};
use sanim_core::{Type, Value};
use sanim_data::{Limits, decode};
#[test]
fn shared_png_textures_preserve_smooth_rgba_and_jpeg_loads_opaque_pixels() {
    let rgba = [255, 0, 0, 128, 0, 0, 255, 0];
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&rgba, 2, 1, ExtendedColorType::Rgba8)
        .unwrap();
    let Value::Texture(texture) =
        decode("alpha.png", &png, &Type::Texture, Limits::default()).unwrap()
    else {
        panic!("Texture")
    };
    assert_eq!(texture.pixels().as_ref(), rgba);
    assert!(texture.has_partial_alpha());
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 100)
        .encode(&[200, 90, 40, 40, 100, 220], 2, 1, ExtendedColorType::Rgb8)
        .unwrap();
    let Value::Texture(texture) =
        decode("photo.jpg", &jpeg, &Type::Texture, Limits::default()).unwrap()
    else {
        panic!("Texture")
    };
    assert_eq!([texture.width(), texture.height()], [2, 1]);
    assert!(
        texture
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 255)
    );
}
#[test]
fn encoded_copy_and_decoded_pixel_budgets_are_checked_before_decoder_buffers() {
    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg)
        .encode(&[200, 90, 40], 1, 1, ExtendedColorType::Rgb8)
        .unwrap();
    jpeg.resize(20_000, 0);
    let error = decode(
        "padded.jpg",
        &jpeg,
        &Type::Texture,
        Limits {
            max_bytes: 64_000,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("encoded decoder buffers"), "{error}");
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &vec![255; 128 * 128 * 4],
            128,
            128,
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    let error = decode(
        "large.png",
        &png,
        &Type::Texture,
        Limits {
            max_bytes: 200_000,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("byte budget"), "{error}");
}
