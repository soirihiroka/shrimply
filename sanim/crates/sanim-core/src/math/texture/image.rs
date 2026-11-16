//! Raster mathematics; image supplies standard filters and shared math-color supplies color operations.
use super::*;
use ::image::{
    ImageBuffer, Rgba, Rgba32FImage, RgbaImage,
    imageops::{self, FilterType},
};
use shrimply_math_color::{Color as FloatColor, LayerBlendMode};
pub(super) const FILTERS: [(&str, FilterType); 5] = [
    ("nearest", FilterType::Nearest),
    ("triangle", FilterType::Triangle),
    ("catmull-rom", FilterType::CatmullRom),
    ("gaussian", FilterType::Gaussian),
    ("lanczos3", FilterType::Lanczos3),
];
pub(super) const FLIP_AXES: [&str; 3] = ["x", "y", "xy"];
pub(super) const MASK_CHANNELS: [&str; 2] = ["alpha", "luma"];
type View<'a> = ImageBuffer<Rgba<u8>, &'a [u8]>;
fn view(texture: &sanim_voxel::Texture) -> View<'_> {
    ImageBuffer::from_raw(texture.width(), texture.height(), texture.pixels().as_ref())
        .expect("validated texture layout")
}
fn rgba(pixel: [u8; 4]) -> FloatColor<f32> {
    FloatColor::from_rgba_u32(u32::from_le_bytes(pixel))
}
fn bytes(color: FloatColor<f32>) -> [u8; 4] {
    color.to_rgba_u32().to_le_bytes()
}
fn linear(texture: &sanim_voxel::Texture) -> Rgba32FImage {
    let source = view(texture);
    ImageBuffer::from_fn(texture.width(), texture.height(), |x, y| {
        Rgba(
            rgba(source.get_pixel(x, y).0)
                .to_linear()
                .premultiply()
                .to_array(),
        )
    })
}
fn finish(image: Rgba32FImage) -> RgbaImage {
    ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        Rgba(bytes(
            FloatColor::from(image.get_pixel(x, y).0)
                .unpremultiply()
                .linear_to_srgb::<true>(),
        ))
    })
}
pub(super) fn filter(args: &Arguments, default: &str) -> Result<FilterType, String> {
    let name = text(args, "filter", default)?;
    FILTERS
        .into_iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, filter)| filter)
        .ok_or_else(|| {
            format!(
                "unknown texture resize filter '{name}'; choose {}",
                FILTERS.map(|(name, _)| name).join(", ")
            )
        })
}
pub(super) fn rotated_size(size: [u32; 2], angle: f64, expand: bool) -> Result<[u32; 2], String> {
    if !angle.is_finite() {
        return Err("texture rotation angle must be finite".into());
    }
    if !expand {
        return Ok(size);
    }
    let angle = angle.rem_euclid(360.0);
    if angle == 0.0 || angle == 180.0 {
        return Ok(size);
    }
    if angle == 90.0 || angle == 270.0 {
        return Ok([size[1], size[0]]);
    }
    let (sin, cos) = angle.to_radians().sin_cos();
    let [width, height] = size.map(f64::from);
    let out = [
        (width * cos.abs() + height * sin.abs() - f64::EPSILON).ceil(),
        (width * sin.abs() + height * cos.abs() - f64::EPSILON).ceil(),
    ];
    if out.iter().any(|value| *value > u32::MAX as f64) {
        return Err("rotated texture dimensions exceed u32".into());
    }
    Ok(out.map(|value| value.max(1.0) as u32))
}
pub(super) fn crop(source: &sanim_voxel::Texture, min: [u32; 2], size: [u32; 2]) -> RgbaImage {
    let input = view(source);
    ImageBuffer::from_fn(size[0], size[1], |x, y| {
        *input.get_pixel(x + min[0], y + min[1])
    })
}
pub(super) fn resize(
    source: &sanim_voxel::Texture,
    size: [u32; 2],
    filter: FilterType,
) -> RgbaImage {
    if filter == FilterType::Nearest {
        let input = view(source);
        return ImageBuffer::from_fn(size[0], size[1], |x, y| {
            let sx = ((u64::from(x) * 2 + 1) * u64::from(source.width()) / (u64::from(size[0]) * 2))
                as u32;
            let sy = ((u64::from(y) * 2 + 1) * u64::from(source.height())
                / (u64::from(size[1]) * 2)) as u32;
            *input.get_pixel(sx.min(source.width() - 1), sy.min(source.height() - 1))
        });
    }
    finish(imageops::resize(&linear(source), size[0], size[1], filter))
}
pub(super) fn flip(source: &sanim_voxel::Texture, axes: &str) -> RgbaImage {
    let source = view(source);
    match axes {
        "x" => imageops::flip_horizontal(&source),
        "y" => imageops::flip_vertical(&source),
        _ => imageops::rotate180(&source),
    }
}
pub(super) fn rotate(
    source: &sanim_voxel::Texture,
    size: [u32; 2],
    angle: f64,
    nearest: bool,
    fill: [u8; 4],
) -> RgbaImage {
    let input = view(source);
    let angle = angle.rem_euclid(360.0);
    if size == [source.height(), source.width()] {
        if angle == 90.0 {
            return imageops::rotate90(&input);
        }
        if angle == 270.0 {
            return imageops::rotate270(&input);
        }
    }
    if size == [source.width(), source.height()] && angle == 180.0 {
        return imageops::rotate180(&input);
    }
    let (sin, cos) = angle.to_radians().sin_cos();
    ImageBuffer::from_fn(size[0], size[1], |x, y| {
        let dx = f64::from(x) + 0.5 - f64::from(size[0]) / 2.0;
        let dy = f64::from(y) + 0.5 - f64::from(size[1]) / 2.0;
        let at = [
            cos * dx + sin * dy + f64::from(source.width()) / 2.0 - 0.5,
            -sin * dx + cos * dy + f64::from(source.height()) / 2.0 - 0.5,
        ];
        if nearest {
            let pixel = [(at[0] + 0.5).floor() as i64, (at[1] + 0.5).floor() as i64];
            return Rgba(
                if pixel[0] < 0
                    || pixel[1] < 0
                    || pixel[0] >= i64::from(source.width())
                    || pixel[1] >= i64::from(source.height())
                {
                    fill
                } else {
                    input.get_pixel(pixel[0] as u32, pixel[1] as u32).0
                },
            );
        }
        let left = at.map(f64::floor);
        let fraction = [at[0] - left[0], at[1] - left[1]];
        let mut sample = [0.0f32; 4];
        for oy in 0..2 {
            for ox in 0..2 {
                let px = left[0] as i64 + ox;
                let py = left[1] as i64 + oy;
                let pixel = if px < 0
                    || py < 0
                    || px >= i64::from(source.width())
                    || py >= i64::from(source.height())
                {
                    fill
                } else {
                    input.get_pixel(px as u32, py as u32).0
                };
                let color = rgba(pixel).to_linear().premultiply().to_array();
                let weight = (if ox == 0 {
                    1.0 - fraction[0]
                } else {
                    fraction[0]
                }) * (if oy == 0 {
                    1.0 - fraction[1]
                } else {
                    fraction[1]
                });
                for (out, value) in sample.iter_mut().zip(color) {
                    *out += value * weight as f32;
                }
            }
        }
        Rgba(bytes(
            FloatColor::from(sample)
                .unpremultiply()
                .linear_to_srgb::<true>(),
        ))
    })
}
pub(super) fn pad(
    source: &sanim_voxel::Texture,
    size: [u32; 2],
    at: [i64; 2],
    fill: [u8; 4],
) -> RgbaImage {
    let mut output = ImageBuffer::from_pixel(size[0], size[1], Rgba(fill));
    imageops::replace(&mut output, &view(source), at[0], at[1]);
    output
}
pub(super) fn tile(source: &sanim_voxel::Texture, size: [u32; 2]) -> RgbaImage {
    let input = view(source);
    ImageBuffer::from_fn(size[0], size[1], |x, y| {
        *input.get_pixel(x % source.width(), y % source.height())
    })
}
pub(super) fn blur(
    source: &sanim_voxel::Texture,
    radius: f32,
    sharpen: Option<(f32, f32)>,
) -> RgbaImage {
    let mut input = linear(source);
    let blurred = imageops::fast_blur(&input, radius);
    if let Some((amount, threshold)) = sharpen {
        for (pixel, blurred) in input.pixels_mut().zip(blurred.pixels()) {
            for channel in 0..3 {
                let delta = pixel.0[channel] - blurred.0[channel];
                if delta.abs() > threshold {
                    pixel.0[channel] += delta * amount;
                }
            }
        }
        finish(input)
    } else {
        finish(blurred)
    }
}
pub(super) fn adjust(source: &sanim_voxel::Texture, operation: Op, factor: f32) -> RgbaImage {
    let input = view(source);
    let mean = if operation == Op::Contrast {
        let mut sum = 0.0f64;
        let mut weights = 0.0f64;
        for pixel in input.pixels() {
            let color = rgba(pixel.0).to_linear();
            sum += f64::from(color.rec709_luma() * color.a);
            weights += f64::from(color.a);
        }
        if weights > 0.0 {
            (sum / weights) as f32
        } else {
            0.0
        }
    } else {
        0.0
    };
    ImageBuffer::from_fn(source.width(), source.height(), |x, y| {
        let pixel = input.get_pixel(x, y).0;
        if operation == Op::Invert {
            return Rgba([255 - pixel[0], 255 - pixel[1], 255 - pixel[2], pixel[3]]);
        }
        let color = rgba(pixel);
        if operation == Op::Hue {
            let mut hsv = math::color::rgba_to_hsv(color.to_array());
            hsv[0] = (hsv[0] + factor).rem_euclid(360.0);
            return Rgba(bytes(FloatColor::from(math::color::hsv_to_rgba(hsv))));
        }
        let mut color = color.to_linear();
        let luma = color.rec709_luma();
        for channel in [&mut color.r, &mut color.g, &mut color.b] {
            *channel = match operation {
                Op::Grayscale => luma,
                Op::Brightness => *channel * factor,
                Op::Contrast => mean + (*channel - mean) * factor,
                Op::Saturation => luma + (*channel - luma) * factor,
                _ => unreachable!("color adjustment"),
            };
        }
        Rgba(bytes(color.linear_to_srgb::<true>()))
    })
}
pub(super) fn blend(
    source: &sanim_voxel::Texture,
    other: &sanim_voxel::Texture,
    amount: f64,
    space: math::color::MixSpace,
) -> Result<RgbaImage, String> {
    let left = view(source);
    let right = view(other);
    let mut output = ImageBuffer::new(source.width(), source.height());
    for (x, y, pixel) in output.enumerate_pixels_mut() {
        let mixed = math::color::mix_rgba(
            rgba(left.get_pixel(x, y).0).to_array(),
            rgba(right.get_pixel(x, y).0).to_array(),
            amount,
            math::color::MixOptions {
                space,
                ..Default::default()
            },
        )?;
        *pixel = Rgba(bytes(FloatColor::from(mixed)));
    }
    Ok(output)
}
pub(super) fn composite(
    source: &sanim_voxel::Texture,
    over: &sanim_voxel::Texture,
    at: [i64; 2],
    mode: LayerBlendMode,
    opacity: f32,
) -> RgbaImage {
    let mut output: RgbaImage =
        ImageBuffer::from_raw(source.width(), source.height(), source.pixels().to_vec())
            .expect("validated texture layout");
    let over = view(over);
    let x0 = at[0].max(0);
    let y0 = at[1].max(0);
    let x1 = (at[0] + i64::from(over.width())).min(i64::from(source.width()));
    let y1 = (at[1] + i64::from(over.height())).min(i64::from(source.height()));
    for y in y0..y1 {
        for x in x0..x1 {
            let pixel = over.get_pixel((x - at[0]) as u32, (y - at[1]) as u32).0;
            if pixel[3] == 0 {
                continue;
            }
            if pixel[3] == 255 && opacity == 1.0 && mode == LayerBlendMode::Normal {
                output.put_pixel(x as u32, y as u32, Rgba(pixel));
                continue;
            }
            let incoming = rgba(pixel).to_linear();
            let background = rgba(output.get_pixel(x as u32, y as u32).0).to_linear();
            let color = incoming.blend_over::<true>(background, mode, incoming.a * opacity);
            output.put_pixel(
                x as u32,
                y as u32,
                Rgba(bytes(color.linear_to_srgb::<true>())),
            );
        }
    }
    output
}
pub(super) fn alpha(
    source: &sanim_voxel::Texture,
    mask: Option<&sanim_voxel::Texture>,
    scalar: f32,
    channel: &str,
    replace: bool,
) -> RgbaImage {
    let input = view(source);
    let mask = mask.map(view);
    ImageBuffer::from_fn(source.width(), source.height(), |x, y| {
        let mut pixel = input.get_pixel(x, y).0;
        let weight = if let Some(mask) = &mask {
            let mask = rgba(mask.get_pixel(x, y).0);
            if channel == "alpha" {
                mask.a
            } else {
                mask.to_linear().rec709_luma()
            }
        } else {
            scalar
        };
        pixel[3] = ((if replace { 255.0 } else { f32::from(pixel[3]) }) * weight)
            .round()
            .clamp(0.0, 255.0) as u8;
        Rgba(pixel)
    })
}
pub(super) fn cutout(source: &sanim_voxel::Texture, threshold: f64) -> RgbaImage {
    let input = view(source);
    ImageBuffer::from_fn(source.width(), source.height(), |x, y| {
        let mut pixel = input.get_pixel(x, y).0;
        pixel[3] = if f64::from(pixel[3]) / 255.0 >= threshold {
            255
        } else {
            0
        };
        Rgba(pixel)
    })
}
