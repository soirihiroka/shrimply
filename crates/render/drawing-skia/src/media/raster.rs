//! Rust raster codecs produce oriented, color-managed Skia images at the source precision.
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use skia_safe::{AlphaType, ColorSpace, ColorType, Data, Image, ImageInfo};
use std::io::Cursor;

pub(super) fn format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"#?RGBE") {
        return Some(ImageFormat::Hdr);
    }
    if matches!(bytes.get(..4), Some(b"II+\0" | b"MM\0+")) {
        return Some(ImageFormat::Tiff);
    }
    image::guess_format(bytes).ok().filter(|format| {
        matches!(
            format,
            ImageFormat::Png
                | ImageFormat::Bmp
                | ImageFormat::Tiff
                | ImageFormat::WebP
                | ImageFormat::Hdr
                | ImageFormat::OpenExr
        )
    })
}

pub(super) fn decode(
    bytes: &[u8],
    format: ImageFormat,
    max_pixels: usize,
) -> Result<(Image, [u32; 2]), String> {
    let error = |e: image::ImageError| format!("Decode {format:?} image: {e}");
    let reader = Cursor::new(bytes);
    // Old RGBE uses an alternate signature for the same Radiance pixel format.
    let mut decoder: Box<dyn ImageDecoder + '_> =
        if format == ImageFormat::Hdr && bytes.starts_with(b"#?RGBE") {
            Box::new(image::codecs::hdr::HdrDecoder::new_nonstrict(reader).map_err(error)?)
        } else {
            Box::new(
                ImageReader::with_format(reader, format)
                    .into_decoder()
                    .map_err(error)?,
            )
        };
    let (width, height) = decoder.dimensions();
    super::validate_size([width, height], max_pixels)?;
    let mut narrow_depth = None;
    let color_space = if format == ImageFormat::Png {
        // image-rs stops at the decoded frame. Validate chunk framing and CRCs
        // through IEND; png's finish discards the compressed payload without decoding it.
        let mut options = png::DecodeOptions::default();
        options.set_skip_ancillary_crc_failures(false);
        let mut png = png::Decoder::new_with_options(Cursor::new(bytes), options)
            .read_info()
            .map_err(|e| format!("Read PNG header: {e}"))?;
        let info = png.info();
        let color_space = info
            .coding_independent_code_points
            .filter(|c| c.matrix_coefficients == 0)
            .and_then(|c| {
                let space = cicp_rgb_space(c.color_primaries.into(), c.transfer_function.into())?;
                if !c.is_video_full_range_image {
                    // A palette's RGB entries are always 8-bit, regardless of index depth.
                    narrow_depth = Some(if info.color_type == png::ColorType::Indexed {
                        8
                    } else {
                        info.bit_depth as u8
                    });
                }
                Some(space)
            })
            .or_else(|| info.icc_profile.as_deref().and_then(ColorSpace::new_icc))
            .or_else(|| {
                if info.srgb.is_some() || (info.gama_chunk.is_none() && info.chrm_chunk.is_none()) {
                    return None;
                }
                // Use Skia's PNG profile conversion for legacy gAMA/cHRM. Pixel decoding
                // stays in image-rs to retain the original 16-bit samples.
                skia_safe::codec::png_decoder::decode_stream(&mut &bytes[..])
                    .ok()?
                    .info()
                    .color_space()
            });
        png.finish()
            .map_err(|e| format!("Incomplete or corrupt PNG: {e}"))?;
        color_space
    } else {
        decoder
            .icc_profile()
            .map_err(error)?
            .as_deref()
            .and_then(ColorSpace::new_icc)
    };
    let orientation = decoder.orientation().map_err(error)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(error)?;
    decoded.apply_orientation(orientation);
    if let Some(depth) = narrow_depth {
        let expand = shrimply_math_color::expand_video_range(depth);
        let mut rgba = decoded.into_rgba32f();
        for pixel in rgba.pixels_mut() {
            for sample in &mut pixel.0[..3] {
                *sample = expand(*sample);
            }
        }
        decoded = DynamicImage::ImageRgba32F(rgba);
    }
    let (decoded, color_type) = match decoded.color() {
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => (
            DynamicImage::ImageRgba32F(decoded.into_rgba32f()),
            ColorType::RGBAF32,
        ),
        image::ColorType::L16
        | image::ColorType::La16
        | image::ColorType::Rgb16
        | image::ColorType::Rgba16 => (
            DynamicImage::ImageRgba16(decoded.into_rgba16()),
            ColorType::R16G16B16A16UNorm,
        ),
        _ => (
            DynamicImage::ImageRgba8(decoded.into_rgba8()),
            ColorType::RGBA8888,
        ),
    };
    let color_space = color_space.unwrap_or_else(|| {
        if color_type == ColorType::RGBAF32 {
            ColorSpace::new_srgb_linear()
        } else {
            ColorSpace::new_srgb()
        }
    });
    from_pixels(
        [decoded.width(), decoded.height()],
        color_type,
        color_space,
        decoded.as_bytes(),
    )
}

/// RGB samples after decoding use only the CICP primaries and transfer function.
pub(super) fn cicp_rgb_space(primaries: u16, transfer: u16) -> Option<ColorSpace> {
    let profile = moxcms::ColorProfile::new_from_cicp(moxcms::CicpProfile {
        color_primaries: u8::try_from(primaries).ok()?.try_into().ok()?,
        transfer_characteristics: u8::try_from(transfer).ok()?.try_into().ok()?,
        matrix_coefficients: moxcms::MatrixCoefficients::Identity,
        full_range: true,
    });
    profile.cicp?;
    ColorSpace::new_icc(&profile.encode().ok()?)
}

pub(super) fn from_pixels(
    size: [u32; 2],
    color_type: ColorType,
    color_space: ColorSpace,
    bytes: &[u8],
) -> Result<(Image, [u32; 2]), String> {
    let info = ImageInfo::new(
        (size[0] as i32, size[1] as i32),
        color_type,
        AlphaType::Unpremul,
        color_space,
    );
    let image =
        skia_safe::images::raster_from_data(&info, Data::new_copy(bytes), info.min_row_bytes())
            .ok_or("Could not prepare decoded raster image")?;
    Ok((image, size))
}
