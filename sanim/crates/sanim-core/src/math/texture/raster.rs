use super::*;
use crate::math::color::{MixOptions, Ramp};
use sanim_voxel::procedural::Field;

fn mode(has: impl Fn(&str) -> bool) -> Result<(), String> {
    if has("pixels") {
        if ["fill", "field", "colors", "mask", "alpha", "space"]
            .into_iter()
            .any(has)
        {
            return Err("Texture pixels= cannot be combined with fill=, field=, colors=, mask=, alpha= or space=; use one construction mode".into());
        }
    } else if (has("field") || has("colors")) && (!has("field") || !has("colors") || has("fill")) {
        return Err("Texture field=Density requires colors=List<Color> and cannot be combined with fill=; alternatively use fill=Color or pixels=List<Color>".into());
    }
    Ok(())
}
pub(super) fn check(args: &std::collections::BTreeMap<String, Type>) -> Result<(), String> {
    mode(|name| args.contains_key(name))
}
fn density<'a>(args: &'a Arguments, name: &str) -> Result<Option<&'a Field>, String> {
    args.get(name)
        .map(|value| {
            if let Value::Density(field) = value {
                Ok(field.as_ref())
            } else {
                Err(format!("texture {name} requires Density"))
            }
        })
        .transpose()
}

pub(super) fn cost(args: &Arguments) -> Result<Cost, String> {
    mode(|name| args.contains_key(name))?;
    let size = pair(required(args, "size")?)?;
    let bytes = sanim_voxel::texture_bytes(size[0], size[1])?;
    let count = bytes / 4;
    let mut cost = Cost {
        work: count,
        bytes: checked_product(bytes, 2)?,
        output_bytes: bytes,
    };
    if let Some(values) = args.get("pixels") {
        let length = values.list()?.len();
        if length != count {
            return Err(format!(
                "Texture size {}×{} requires exactly {count} colors in row-major order; pixels has {length}",
                size[0], size[1]
            ));
        }
    } else {
        let fields = [
            density(args, "field")?,
            density(args, "mask")?,
            density(args, "alpha")?,
        ];
        if fields.iter().any(Option::is_some) {
            let colors = args.get("colors").map(Value::list).transpose()?;
            let length = colors.map_or(1, |colors| colors.len());
            if length == 0 {
                return Err("texture color ramp must have at least one Color".into());
            }
            cost.bytes = cost
                .bytes
                .checked_add(Ramp::allocation_bytes(length)?)
                .ok_or("texture byte budget overflow")?;
            let ramp_work = if length > 1 { Ramp::MAX_SAMPLE_WORK } else { 1 };
            let sample_work = fields
                .into_iter()
                .flatten()
                .map(Field::sample_work)
                .sum::<usize>()
                + ramp_work
                + 1;
            cost.work = checked_product(count, sample_work)?
                .checked_add(length)
                .ok_or("texture work budget overflow")?;
        }
    }
    Ok(cost)
}

pub(super) fn execute(args: &Arguments) -> Result<Value, String> {
    let [width, height] = pair(required(args, "size")?)?;
    let bytes = sanim_voxel::texture_bytes(width, height)?;
    let mut pixels = Vec::with_capacity(bytes);
    if let Some(values) = args.get("pixels") {
        for (index, value) in values.list()?.iter().enumerate() {
            let Value::Color(color) = value else {
                return Err(format!("texture pixel {index} requires Color"));
            };
            pixels.extend(color.rgba8()?);
        }
    } else {
        let field = density(args, "field")?;
        let mask = density(args, "mask")?;
        let alpha = density(args, "alpha")?;
        if field.is_none() && mask.is_none() && alpha.is_none() {
            let fill = color(args, "fill")?;
            for _ in 0..bytes / 4 {
                pixels.extend(fill);
            }
        } else {
            let colors = if let Some(values) = args.get("colors") {
                values
                    .list()?
                    .iter()
                    .map(|value| {
                        if let Value::Color(color) = value {
                            color.srgba()
                        } else {
                            Err("texture ramp requires Color values".into())
                        }
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                vec![match args.get("fill") {
                    Some(Value::Color(color)) => color.srgba()?,
                    None => [0.; 4],
                    _ => return Err("texture fill requires Color".into()),
                }]
            };
            let options = MixOptions {
                space: text(args, "space", "oklab")?.parse()?,
                ..Default::default()
            };
            let ramp = Ramp::new(colors, options)?;
            let mut field = field.map(Field::sampler);
            let mut mask = mask.map(Field::sampler);
            let mut alpha = alpha.map(Field::sampler);
            for y in 0..height {
                for x in 0..width {
                    let point = [f64::from(x) + 0.5, f64::from(y) + 0.5, 0.0];
                    if let Some(mask) = &mut mask
                        && mask.sample(point)? <= 0.0
                    {
                        pixels.extend([0; 4]);
                        continue;
                    }
                    let position = field
                        .as_mut()
                        .map(|field| field.sample(point))
                        .transpose()?
                        .unwrap_or(0.0);
                    let mut color = ramp.sample(position)?;
                    if let Some(alpha) = &mut alpha {
                        color[3] *= alpha.sample(point)?.clamp(0.0, 1.0) as f32;
                    }
                    pixels.extend(
                        shrimply_math_color::Color::<f32>::from(color)
                            .to_rgba_u32()
                            .to_le_bytes(),
                    );
                }
            }
        }
    }
    publish([width, height], pixels)
}
