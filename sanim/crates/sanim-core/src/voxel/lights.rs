//! One typed record/configuration boundary serves preparation, retained signals, and editor metadata.
use crate::{Fraction, List, ListLength, Number, Type, Unit, Value, math};
use sanim_voxel::{Environment, Light, PointLight, SunLight, layout::Coordinate};
use std::{collections::BTreeMap, sync::Arc};

fn number_list() -> Type {
    Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3))
}
pub fn signature(name: &str) -> (Vec<(&'static str, Type, bool)>, Type) {
    let num = Type::Numeric(Unit::Number);
    let parameters = match name {
        "SunLight" => vec![
            ("direction", number_list(), false),
            ("color", Type::Color, false),
            ("intensity", num.clone(), false),
            ("diameter", Type::Numeric(Unit::Angle), false),
        ],
        "PointLight" => vec![
            ("position", super::config::coordinate_type(), false),
            ("color", Type::Color, false),
            ("intensity", num.clone(), false),
            ("radius", num, false),
        ],
        "Environment" => vec![
            ("top", Type::Color, false),
            ("horizon", Type::Color, false),
            ("ground", Type::Color, false),
            ("intensity", num, false),
            ("visible", Type::Bool, false),
        ],
        _ => unreachable!("registered light configuration constructor"),
    };
    let mut fields = parameters
        .iter()
        .map(|(name, ty, _)| ((*name).into(), ty.clone()))
        .collect::<BTreeMap<_, _>>();
    if name != "Environment" {
        fields.insert(
            "kind".into(),
            Type::StringLiteral(if name == "SunLight" { "sun" } else { "point" }.into()),
        );
    }
    (parameters, Type::Record(fields))
}
pub fn light_type() -> Type {
    Type::union([signature("SunLight").1, signature("PointLight").1]).expect("scene light kinds")
}
pub fn lights_type() -> Type {
    Type::List(Box::new(light_type()), ListLength::Unknown)
}
pub fn environment_type() -> Type {
    signature("Environment").1
}
fn number(value: f32, unit: Unit) -> Result<Value, String> {
    Ok(Value::Number(Number {
        value: math::from_f64(f64::from(value))?,
        unit,
    }))
}
fn numbers(values: [f32; 3]) -> Result<Value, String> {
    Ok(Value::List(List::typed(
        Type::Numeric(Unit::Number),
        values
            .into_iter()
            .map(|value| number(value, Unit::Number))
            .collect::<Result<Vec<_>, _>>()?,
    )))
}
fn color_value(color: [f32; 3]) -> Result<Value, String> {
    let color = shrimply_math_color::Color::new(color[0], color[1], color[2], 1.)
        .linear_to_srgb::<true>()
        .to_array();
    let mut values = [Fraction::from(1); 4];
    for axis in 0..3 {
        values[axis] = math::from_f64(f64::from(color[axis]))?;
    }
    Ok(Value::Color(crate::Color(values)))
}
fn color(value: &Value) -> Result<[f32; 3], String> {
    let Value::Color(color) = value else {
        return Err("voxel light color requires Color(...)".into());
    };
    if color.0[3] != Fraction::from(1) {
        return Err("voxel light/environment colors require opaque alpha; set intensity=0 to disable illumination".into());
    }
    Ok(shrimply_math_color::Color::<f32>::from(color.srgba()?)
        .to_linear()
        .to_rgb_array())
}
fn coordinate_values(position: [Coordinate; 3]) -> Result<Value, String> {
    let mut values = Vec::with_capacity(3);
    for coordinate in position {
        let (value, unit) = match coordinate {
            Coordinate::Cells(value) => (value, Unit::Number),
            Coordinate::Percent(value) => (
                math::arithmetic("/", value, Fraction::from(100))?,
                Unit::Percent,
            ),
        };
        values.push(Value::Number(Number { value, unit }));
    }
    let Type::List(element, _) = super::config::coordinate_type() else {
        unreachable!("coordinate list");
    };
    Ok(Value::List(List::typed(*element, values)))
}
pub fn light_value(light: &Light) -> Result<Value, String> {
    let fields = match light {
        Light::Sun(light) => vec![
            ("kind", Value::String("sun".into())),
            ("direction", numbers(light.direction)?),
            ("color", color_value(light.color)?),
            ("intensity", number(light.intensity, Unit::Number)?),
            (
                "diameter",
                number(light.angular_diameter_degrees, Unit::Angle)?,
            ),
        ],
        Light::Point(light) => vec![
            ("kind", Value::String("point".into())),
            ("position", coordinate_values(light.position)?),
            ("color", color_value(light.color)?),
            ("intensity", number(light.intensity, Unit::Number)?),
            ("radius", number(light.radius, Unit::Number)?),
        ],
    };
    Ok(Value::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    ))
}
pub fn environment_value(environment: &Environment) -> Result<Value, String> {
    Ok(Value::Record(
        [
            ("top", color_value(environment.top)?),
            ("horizon", color_value(environment.horizon)?),
            ("ground", color_value(environment.ground)?),
            ("intensity", number(environment.intensity, Unit::Number)?),
            ("visible", Value::Bool(environment.visible)),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect(),
    ))
}
fn fields(value: &Value) -> Result<&imbl::OrdMap<String, Value>, String> {
    if let Value::Record(fields) = value {
        Ok(fields)
    } else {
        Err("voxel lighting requires typed SunLight(...), PointLight(...), or Environment(...) records".into())
    }
}
fn field<'a>(fields: &'a imbl::OrdMap<String, Value>, name: &str) -> Result<&'a Value, String> {
    fields
        .get(name)
        .ok_or_else(|| format!("voxel lighting record is missing '{name}'"))
}
fn scalar(fields: &imbl::OrdMap<String, Value>, name: &str, unit: Unit) -> Result<f32, String> {
    let number = field(fields, name)?.number()?;
    if number.unit != unit {
        return Err(format!(
            "voxel lighting {name} requires {}",
            Type::Numeric(unit)
        ));
    }
    let value = math::to_f64(number.value)? as f32;
    if !value.is_finite() {
        return Err(format!(
            "voxel lighting {name} exceeds the finite GPU numeric range"
        ));
    }
    Ok(value)
}
pub fn light(value: &Value) -> Result<Light, String> {
    let fields = fields(value)?;
    let color = color(field(fields, "color")?)?;
    let intensity = scalar(fields, "intensity", Unit::Number)?;
    let light = match field(fields, "kind")?.string()? {
        "sun" => {
            let values = field(fields, "direction")?.list()?;
            if values.len() != 3 {
                return Err("sun direction requires exactly three numeric components".into());
            }
            let mut direction = [0.; 3];
            for (axis, value) in values.iter().enumerate() {
                let value = value.number()?;
                if value.unit != Unit::Number {
                    return Err("sun direction requires dimensionless Number components".into());
                }
                direction[axis] = math::to_f64(value.value)? as f32;
            }
            Light::Sun(SunLight {
                direction,
                color,
                intensity,
                angular_diameter_degrees: scalar(fields, "diameter", Unit::Angle)?,
            })
        }
        "point" => Light::Point(PointLight {
            position: super::config::coordinates(field(fields, "position")?)?,
            color,
            intensity,
            radius: scalar(fields, "radius", Unit::Number)?,
        }),
        kind => {
            return Err(format!(
                "unknown voxel light kind '{kind}'; use SunLight(...) or PointLight(...)"
            ));
        }
    };
    light.validate()?;
    Ok(light)
}
pub fn environment(value: &Value) -> Result<Environment, String> {
    let fields = fields(value)?;
    let Value::Bool(visible) = field(fields, "visible")? else {
        return Err("environment visible requires Bool".into());
    };
    let environment = Environment {
        top: color(field(fields, "top")?)?,
        horizon: color(field(fields, "horizon")?)?,
        ground: color(field(fields, "ground")?)?,
        intensity: scalar(fields, "intensity", Unit::Number)?,
        visible: *visible,
    };
    environment.validate()?;
    Ok(environment)
}
pub fn lights(value: &Value) -> Result<Arc<[Light]>, String> {
    let values = value.list()?;
    if values.len() > sanim_voxel::MAX_LIGHTS {
        return Err(format!(
            "voxel scene exceeds its {}-light budget",
            sanim_voxel::MAX_LIGHTS
        ));
    }
    values
        .iter()
        .map(light)
        .collect::<Result<Vec<_>, _>>()
        .map(Arc::from)
}
pub fn lights_value(lights: &[Light]) -> Result<Value, String> {
    Ok(Value::List(List::typed(
        light_type(),
        lights
            .iter()
            .map(light_value)
            .collect::<Result<Vec<_>, _>>()?,
    )))
}

pub fn execute(name: &str, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
    let value = match name {
        "SunLight" => light_value(&Light::Sun(Default::default()))?,
        "PointLight" => light_value(&Light::Point(Default::default()))?,
        "Environment" => environment_value(&Environment::default())?,
        _ => unreachable!("registered light constructor"),
    };
    let Value::Record(mut fields) = value else {
        unreachable!("light records");
    };
    for (key, value) in args {
        if key != "kind" && fields.contains_key(key) {
            fields.insert(key.clone(), value.clone());
        }
    }
    let value = Value::Record(fields);
    if name == "Environment" {
        environment(&value)?;
    } else {
        light(&value)?;
    }
    Ok(value)
}
