//! HEIF and AVIF share the same Rust container, grid, crop, alpha and transform decoder.
use imgalifier_heif::{CodecBackend, Colr, Decoder, Limits, Property, hevc::HevcBackend};
use skia_safe::{ColorSpace, ColorType, Image};

pub(super) fn decode(bytes: &[u8], max_pixels: usize) -> Result<(Image, [u32; 2]), String> {
    let max_pixels = max_pixels.min(super::MAX_PIXELS);
    let limits = Limits {
        max_pixels: max_pixels as u64,
        max_item_bytes: super::MAX_SOURCE_BYTES,
        ..Default::default()
    };
    let hevc = Decoder::new(bytes, HevcBackend::default(), limits).map_err(|e| e.to_string())?;
    let container = hevc.container();
    let avif = [*b"avif", *b"avis"]
        .into_iter()
        .any(|brand| container.has_brand(brand));
    let heif = [
        *b"mif1", *b"msf1", *b"mif2", *b"heic", *b"heix", *b"heim", *b"heis", *b"hevc", *b"hevx",
        *b"hevm", *b"hevs",
    ]
    .into_iter()
    .any(|brand| container.has_brand(brand));
    if !heif && !avif {
        return Err("ISO media file is not a HEIF or AVIF image".into());
    }
    if avif
        || (container
            .items
            .iter()
            .any(|item| item.item_type == *b"av01")
            && !container
                .items
                .iter()
                .any(|item| matches!(&item.item_type, b"hvc1" | b"hev1")))
    {
        drop(hevc);
        let backend = imgalifier_avif::Av1Backend {
            max_pixels: max_pixels as u32,
            ..Default::default()
        };
        render(
            Decoder::new(bytes, backend, limits).map_err(|e| e.to_string())?,
            max_pixels,
        )
    } else {
        render(hevc, max_pixels)
    }
}

fn render(
    decoder: Decoder<'_, impl CodecBackend>,
    max_pixels: usize,
) -> Result<(Image, [u32; 2]), String> {
    let info = decoder.info().map_err(|e| e.to_string())?;
    super::validate_size([info.width, info.height], max_pixels)?;
    let container = decoder.container();
    let color_space = container
        .primary()
        .ok()
        .and_then(|primary| {
            let properties = || container.item_properties(primary);
            properties()
                .find_map(|(property, _)| match property {
                    Property::Colr(Colr::Icc(bytes)) => ColorSpace::new_icc(bytes),
                    _ => None,
                })
                .or_else(|| {
                    properties().find_map(|(property, _)| {
                        let Property::Colr(Colr::Nclx {
                            primaries,
                            transfer,
                            ..
                        }) = property
                        else {
                            return None;
                        };
                        // YUV conversion is complete. Keep its RGB primaries and transfer curve.
                        let profile = moxcms::ColorProfile::new_from_cicp(moxcms::CicpProfile {
                            color_primaries: u8::try_from(*primaries).ok()?.try_into().ok()?,
                            transfer_characteristics: u8::try_from(*transfer)
                                .ok()?
                                .try_into()
                                .ok()?,
                            matrix_coefficients: moxcms::MatrixCoefficients::Identity,
                            full_range: true,
                        });
                        ColorSpace::new_icc(&profile.encode().ok()?)
                    })
                })
        })
        .unwrap_or_else(ColorSpace::new_srgb);
    let image = decoder.decode_primary().map_err(|e| e.to_string())?;
    super::validate_size([image.width, image.height], max_pixels)?;
    super::raster::from_pixels(
        [image.width, image.height],
        ColorType::RGBA8888,
        color_space,
        &image.data,
    )
}
