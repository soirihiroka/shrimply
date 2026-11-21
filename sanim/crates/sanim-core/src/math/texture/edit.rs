use super::*;
use ::image::imageops::FilterType;
use shrimply_math_color::LayerBlendMode;
fn source_size(source: &sanim_voxel::Texture) -> [u32; 2] {
    [source.width(), source.height()]
}
fn size(args: &Arguments) -> Result<[u32; 2], String> {
    pair(required(args, "size")?)
}
fn offset(args: &Arguments, default: [i64; 2]) -> Result<[i64; 2], String> {
    let Some(value) = args.get("at") else {
        return Ok(default);
    };
    let values = value.list()?;
    if values.len() != 2 {
        return Err("texture at requires two signed integer offsets".into());
    }
    let mut result = [0; 2];
    for (out, value) in result.iter_mut().zip(values) {
        let value = math::to_f64(value.number()?.value)?;
        if value.fract() != 0.0 || value.abs() > f64::from(i32::MAX) {
            return Err("texture offsets require integers within the signed 32-bit range".into());
        }
        *out = value as i64;
    }
    Ok(result)
}
fn unit(args: &Arguments, name: &str, default: f64) -> Result<f64, String> {
    let value = number(args, name, default)?;
    if !(0.0..=1.0).contains(&value) {
        Err(format!("texture {name} must be within 0..=1"))
    } else {
        Ok(value)
    }
}
fn nonnegative(args: &Arguments, name: &str, default: f64) -> Result<f64, String> {
    let value = number(args, name, default)?;
    if value < 0.0 || value > f64::from(f32::MAX) {
        Err(format!(
            "texture {name} must be finite, nonnegative and representable as f32"
        ))
    } else {
        Ok(value)
    }
}
fn matching<'a>(
    source: &sanim_voxel::Texture,
    args: &'a Arguments,
    name: &str,
) -> Result<&'a Arc<sanim_voxel::Texture>, String> {
    let other = texture(args, name)?;
    if source_size(source) != source_size(other) {
        return Err(format!(
            "texture {name} requires equal sizes; source is {}×{}, {name} is {}×{}",
            source.width(),
            source.height(),
            other.width(),
            other.height()
        ));
    }
    Ok(other)
}
fn mode(args: &Arguments) -> Result<LayerBlendMode, String> {
    serde_json::from_value(serde_json::Value::String(
        text(args, "mode", "normal")?.into(),
    ))
    .map_err(|error| format!("texture composite mode: {error}"))
}
fn crop(args: &Arguments, source: &sanim_voxel::Texture) -> Result<([u32; 2], [u32; 2]), String> {
    let min = pair(required(args, "min")?)?;
    let size = size(args)?;
    sanim_voxel::texture_bytes(size[0], size[1])?;
    if (0..2).any(|axis| {
        u64::from(min[axis]) + u64::from(size[axis]) > u64::from(source_size(source)[axis])
    }) {
        return Err(format!(
            "texture crop min {min:?} and size {size:?} exceed source size {:?}",
            source_size(source)
        ));
    }
    Ok((min, size))
}
fn result_cost(size: [u32; 2], work: usize, scratch: usize) -> Result<Cost, String> {
    let bytes = sanim_voxel::texture_bytes(size[0], size[1])?;
    Ok(Cost {
        work,
        output_bytes: bytes,
        bytes: checked_product(bytes, 2)?
            .checked_add(scratch)
            .ok_or("texture scratch byte count overflow")?,
    })
}
fn radius(args: &Arguments, source: &sanim_voxel::Texture) -> Result<f64, String> {
    let radius = nonnegative(args, "radius", 2.0)?;
    if radius > usize::MAX as f64 || !radius.is_normal() && radius != 0.0 {
        return Err("texture blur radius is outside the supported numeric range".into());
    }
    let work = radius.ceil() as usize;
    if work
        .checked_mul((source.width() as usize + source.height() as usize) * 8)
        .is_none()
    {
        return Err("texture blur radius exceeds its work budget".into());
    }
    Ok(radius)
}
fn valid_axes(args: &Arguments) -> Result<&str, String> {
    let axes = text(args, "axes", "x")?;
    if !image::FLIP_AXES.contains(&axes) {
        return Err("texture flip axes must be x, y or xy".into());
    }
    Ok(axes)
}
fn channel(args: &Arguments) -> Result<&str, String> {
    let name = text(args, "channel", "alpha")?;
    if !image::MASK_CHANNELS.contains(&name) {
        return Err("texture mask channel must be alpha or luma".into());
    }
    Ok(name)
}

fn identity(source: &sanim_voxel::Texture) -> Cost {
    Cost {
        work: 0,
        bytes: 0,
        output_bytes: source.pixels().len(),
    }
}
pub(super) fn cost(operation: Op, args: &Arguments) -> Result<Cost, String> {
    let source = texture(args, "texture")?;
    let original = source_size(source);
    let count = source.pixels().len() / 4;
    let mut out = original;
    let mut work = count;
    let mut scratch = 0;
    match operation {
        Op::Size => {
            return Ok(Cost {
                work: 1,
                bytes: 0,
                output_bytes: 0,
            });
        }
        Op::Pixel => {
            let at = pair(required(args, "at")?)?;
            if at[0] >= original[0] || at[1] >= original[1] {
                return Err(format!(
                    "texture pixel {at:?} is outside source size {original:?}"
                ));
            }
            return Ok(Cost {
                work: 1,
                bytes: 0,
                output_bytes: 0,
            });
        }
        Op::Crop => {
            let (min, size) = crop(args, source)?;
            out = size;
            if min == [0, 0] && out == original {
                return Ok(identity(source));
            }
            work = out[0] as usize * out[1] as usize;
        }
        Op::Resize => {
            out = size(args)?;
            sanim_voxel::texture_bytes(out[0], out[1])?;
            let filter = image::filter(args, "lanczos3")?;
            if out == original {
                return Ok(identity(source));
            }
            let pixels = checked_product(out[0] as usize, out[1] as usize)?;
            work = pixels;
            if filter != FilterType::Nearest {
                let cross = checked_product(original[0] as usize, out[1] as usize)?;
                scratch = checked_product(
                    count
                        .checked_add(cross)
                        .and_then(|v| v.checked_add(pixels))
                        .ok_or("texture resampling scratch overflow")?,
                    16,
                )?;
                scratch = scratch
                    .checked_add(checked_product(original[0].max(original[1]) as usize, 16)?)
                    .ok_or("texture resampling weights overflow")?;
                let support = match filter {
                    FilterType::Triangle => 1,
                    FilterType::CatmullRom => 2,
                    _ => 3,
                };
                let vertical =
                    2 * support * (original[1] as usize).div_ceil(out[1] as usize).max(1) + 1;
                let horizontal =
                    2 * support * (original[0] as usize).div_ceil(out[0] as usize).max(1) + 1;
                work = checked_product(cross, vertical)?
                    .checked_add(checked_product(pixels, horizontal)?)
                    .ok_or("texture resampling work overflow")?;
            }
        }
        Op::Flip => {
            valid_axes(args)?;
        }
        Op::Rotate => {
            let angle = number(args, "angle", 90.0)?;
            let expand = boolean(args, "expand", true)?;
            let filter = image::filter(args, "triangle")?;
            if !matches!(filter, FilterType::Nearest | FilterType::Triangle) {
                return Err("texture rotation supports nearest or triangle filtering".into());
            }
            color(args, "fill")?;
            out = image::rotated_size(original, angle, expand)?;
            if angle.rem_euclid(360.0) == 0.0 {
                return Ok(identity(source));
            }
            work = checked_product(
                checked_product(out[0] as usize, out[1] as usize)?,
                if filter == FilterType::Nearest { 1 } else { 16 },
            )?;
        }
        Op::Pad => {
            out = size(args)?;
            let at = offset(
                args,
                [
                    (i64::from(out[0]) - i64::from(original[0])) / 2,
                    (i64::from(out[1]) - i64::from(original[1])) / 2,
                ],
            )?;
            color(args, "fill")?;
            if out == original && at == [0, 0] {
                return Ok(identity(source));
            }
            work = checked_product(out[0] as usize, out[1] as usize)?
                .checked_add(count)
                .ok_or("texture padding work overflow")?;
        }
        Op::Tile => {
            out = size(args)?;
            if out == original {
                return Ok(identity(source));
            }
            work = checked_product(out[0] as usize, out[1] as usize)?;
        }
        Op::Blur | Op::Sharpen => {
            let radius = radius(args, source)?;
            let amount = if operation == Op::Sharpen {
                nonnegative(args, "amount", 1.0)?
            } else {
                1.0
            };
            let threshold = number(args, "threshold", 0.0)?;
            if !(0.0..=255.0).contains(&threshold) {
                return Err("texture sharpen threshold must be within 0..=255".into());
            }
            if radius == 0.0 || amount == 0.0 {
                return Ok(identity(source));
            }
            scratch = checked_product(count, 48)?;
            let edges = checked_product(
                radius.ceil() as usize,
                (original[0] as usize + original[1] as usize) * 8,
            )?;
            work = checked_product(count, if operation == Op::Sharpen { 32 } else { 24 })?
                .checked_add(edges)
                .ok_or("texture filtering work overflow")?;
        }
        Op::Brightness | Op::Contrast | Op::Saturation => {
            let factor = nonnegative(args, "factor", 1.0)?;
            if factor == 1.0 {
                return Ok(identity(source));
            }
            work = checked_product(count, if operation == Op::Contrast { 8 } else { 4 })?;
        }
        Op::Hue => {
            let angle = number(args, "angle", 0.0)?;
            if angle.rem_euclid(360.0) == 0.0 {
                return Ok(identity(source));
            }
            work = checked_product(count, 4)?;
        }
        Op::Blend => {
            matching(source, args, "other")?;
            let amount = unit(args, "amount", 0.5)?;
            let _: math::color::MixSpace = text(args, "space", "oklab")?.parse()?;
            if amount == 0.0 || amount == 1.0 {
                return Ok(identity(source));
            }
            work = checked_product(count, math::color::Ramp::MAX_SAMPLE_WORK + 2)?;
        }
        Op::Composite => {
            let over = texture(args, "over")?;
            let at = offset(args, [0, 0])?;
            mode(args)?;
            let opacity = unit(args, "opacity", 1.0)?;
            if opacity == 0.0
                || at[0] >= i64::from(source.width())
                || at[1] >= i64::from(source.height())
                || at[0] + i64::from(over.width()) <= 0
                || at[1] + i64::from(over.height()) <= 0
            {
                return Ok(identity(source));
            }
            work = checked_product(count, 16)?;
        }
        Op::Mask => {
            matching(source, args, "mask")?;
            channel(args)?;
            work = checked_product(count, 4)?;
        }
        Op::WithAlpha => {
            match required(args, "alpha")? {
                Value::Texture(_) => {
                    matching(source, args, "alpha")?;
                }
                Value::Number(_) => {
                    unit(args, "alpha", 1.0)?;
                }
                _ => return Err("texture alpha requires Number or Texture".into()),
            }
            channel(args)?;
            work = checked_product(count, 4)?;
        }
        Op::Cutout => {
            let threshold = unit(args, "threshold", 0.5)?;
            if !source.has_partial_alpha() && threshold > 0.0 {
                return Ok(identity(source));
            }
        }
        Op::Grayscale => work = checked_product(count, 4)?,
        Op::Invert => {}
        Op::Texture | Op::Draw => unreachable!("texture specialized cost"),
    }
    result_cost(out, work, scratch)
}

pub(super) fn execute(operation: Op, args: &Arguments) -> Result<Value, String> {
    let source = texture(args, "texture")?;
    let original = source_size(source);
    let output = match operation {
        Op::Size => {
            return Ok(Value::List(crate::List::typed(
                Type::Numeric(Unit::Number),
                original
                    .into_iter()
                    .map(|value| {
                        Value::Number(crate::Number {
                            value: crate::Fraction::from(value),
                            unit: Unit::Number,
                        })
                    })
                    .collect(),
            )));
        }
        Op::Pixel => {
            let [x, y] = pair(required(args, "at")?)?;
            let start = (y as usize * source.width() as usize + x as usize) * 4;
            let pixel = source.pixels()[start..start + 4]
                .try_into()
                .expect("validated pixel");
            return math::color::from_rgba(
                shrimply_math_color::Color::<f32>::from_rgba_u32(u32::from_le_bytes(pixel))
                    .to_array(),
            )
            .map(Value::Color);
        }
        Op::Crop => {
            let (min, size) = crop(args, source)?;
            if min == [0, 0] && size == original {
                return Ok(Value::Texture(source.clone()));
            }
            image::crop(source, min, size)
        }
        Op::Resize => {
            let size = size(args)?;
            if size == original {
                return Ok(Value::Texture(source.clone()));
            }
            image::resize(source, size, image::filter(args, "lanczos3")?)
        }
        Op::Flip => image::flip(source, valid_axes(args)?),
        Op::Rotate => {
            let angle = number(args, "angle", 90.0)?;
            if angle.rem_euclid(360.0) == 0.0 {
                return Ok(Value::Texture(source.clone()));
            }
            let size = image::rotated_size(original, angle, boolean(args, "expand", true)?)?;
            image::rotate(
                source,
                size,
                angle,
                image::filter(args, "triangle")? == FilterType::Nearest,
                color(args, "fill")?,
            )
        }
        Op::Pad => {
            let size = size(args)?;
            let at = offset(
                args,
                [
                    (i64::from(size[0]) - i64::from(original[0])) / 2,
                    (i64::from(size[1]) - i64::from(original[1])) / 2,
                ],
            )?;
            if size == original && at == [0, 0] {
                return Ok(Value::Texture(source.clone()));
            }
            image::pad(source, size, at, color(args, "fill")?)
        }
        Op::Tile => {
            let size = size(args)?;
            if size == original {
                return Ok(Value::Texture(source.clone()));
            }
            image::tile(source, size)
        }
        Op::Blur | Op::Sharpen => {
            let radius = radius(args, source)?;
            let amount = nonnegative(args, "amount", 1.0)?;
            if radius == 0.0 || operation == Op::Sharpen && amount == 0.0 {
                return Ok(Value::Texture(source.clone()));
            }
            image::blur(
                source,
                radius as f32,
                (operation == Op::Sharpen).then_some((
                    amount as f32,
                    (number(args, "threshold", 0.0)? / 255.0) as f32,
                )),
            )
        }
        Op::Grayscale | Op::Invert => image::adjust(source, operation, 1.0),
        Op::Brightness | Op::Contrast | Op::Saturation => {
            let factor = nonnegative(args, "factor", 1.0)?;
            if factor == 1.0 {
                return Ok(Value::Texture(source.clone()));
            }
            image::adjust(source, operation, factor as f32)
        }
        Op::Hue => {
            let angle = number(args, "angle", 0.0)?.rem_euclid(360.0);
            if angle == 0.0 {
                return Ok(Value::Texture(source.clone()));
            }
            image::adjust(source, operation, angle as f32)
        }
        Op::Blend => {
            let other = matching(source, args, "other")?;
            let amount = unit(args, "amount", 0.5)?;
            if amount == 0.0 {
                return Ok(Value::Texture(source.clone()));
            }
            if amount == 1.0 {
                return Ok(Value::Texture(other.clone()));
            }
            image::blend(
                source,
                other,
                amount,
                text(args, "space", "oklab")?.parse()?,
            )?
        }
        Op::Composite => {
            let over = texture(args, "over")?;
            let at = offset(args, [0, 0])?;
            let opacity = unit(args, "opacity", 1.0)?;
            if opacity == 0.0
                || at[0] >= i64::from(source.width())
                || at[1] >= i64::from(source.height())
                || at[0] + i64::from(over.width()) <= 0
                || at[1] + i64::from(over.height()) <= 0
            {
                return Ok(Value::Texture(source.clone()));
            }
            image::composite(source, over, at, mode(args)?, opacity as f32)
        }
        Op::Mask => image::alpha(
            source,
            Some(matching(source, args, "mask")?),
            1.0,
            channel(args)?,
            false,
        ),
        Op::WithAlpha => match required(args, "alpha")? {
            Value::Texture(_) => image::alpha(
                source,
                Some(matching(source, args, "alpha")?),
                1.0,
                channel(args)?,
                true,
            ),
            _ => image::alpha(
                source,
                None,
                unit(args, "alpha", 1.0)? as f32,
                channel(args)?,
                true,
            ),
        },
        Op::Cutout => {
            let threshold = unit(args, "threshold", 0.5)?;
            if !source.has_partial_alpha() && threshold > 0.0 {
                return Ok(Value::Texture(source.clone()));
            }
            image::cutout(source, threshold)
        }
        Op::Texture | Op::Draw => unreachable!("texture specialized execution"),
    };
    publish([output.width(), output.height()], output.into_raw())
}
