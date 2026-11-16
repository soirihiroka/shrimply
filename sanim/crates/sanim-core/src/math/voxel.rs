//! Exact parent/child anchor placement, rounded once after composing all offsets.
use crate::{Fraction, Unit, Value, math::arithmetic};
use num_traits::ToPrimitive;

fn component(value: &Value, extent: u32) -> Result<Fraction, String> {
    let number = value.number()?;
    match number.unit {
        Unit::Number => Ok(number.value),
        Unit::Percent => arithmetic("*", number.value, Fraction::from(extent)),
        _ => Err(
            "voxel placement coordinates require cell numbers or parent-relative percentages"
                .into(),
        ),
    }
}

pub fn placement_origin(
    at: &Value,
    anchor: Option<&Value>,
    offset: Option<&Value>,
    parent: [u32; 3],
    child: [u32; 3],
) -> Result<[i32; 3], String> {
    let at = at.list()?;
    let anchor = anchor.map(Value::list).transpose()?;
    let offset = offset.map(Value::list).transpose()?;
    if at.len() != 3
        || anchor.as_ref().is_some_and(|values| values.len() != 3)
        || offset.as_ref().is_some_and(|values| values.len() != 3)
    {
        return Err("voxel placement requires three components for at, anchor, and offset".into());
    }
    let mut result = [0; 3];
    for axis in 0..3 {
        let at = component(at.get(axis).expect("three coordinates"), parent[axis])?;
        let anchor = anchor
            .as_ref()
            .map(|values| component(values.get(axis).expect("three coordinates"), child[axis]))
            .transpose()?
            .unwrap_or_else(|| Fraction::from(0));
        let offset = offset
            .as_ref()
            .map(|values| -> Result<Fraction, String> {
                let number = values.get(axis).expect("three coordinates").number()?;
                if number.unit != Unit::Number {
                    return Err("voxel placement offset requires dimensionless cell numbers".into());
                }
                Ok(number.value)
            })
            .transpose()?
            .unwrap_or_else(|| Fraction::from(0));
        let origin = arithmetic("+", arithmetic("-", at, anchor)?, offset)?.floor();
        result[axis] = origin
            .to_i32()
            .ok_or("voxel placement origin is outside the signed cell-coordinate range")?;
    }
    Ok(result)
}

/// Convert a displayed sRGB color into physical linear transmittance, without duplicating color transfer math.
pub fn attenuation_color(value: &Value) -> Result<[f32; 3], String> {
    let Value::Color(color) = value else {
        return Err("voxel attenuation_color requires Color(...)".into());
    };
    if color.0[3] != Fraction::from(1) {
        return Err("voxel attenuation_color must be opaque; control glass/water transparency with transmission".into());
    }
    Ok(shrimply_math_color::Color::<f32>::from(color.srgba()?)
        .to_linear()
        .to_rgb_array())
}
