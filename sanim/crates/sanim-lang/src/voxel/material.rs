//! One optical scalar registry supplies both typed parameters and constructor updates.
use super::{parameter, rgba};
use crate::prelude::Parameter;
use sanim_core::{Type, Unit, Value, math};
use sanim_voxel::{MaterialGeometry, OpticalProperties};
use std::sync::Arc;

type Scalar = (&'static str, fn(&mut OpticalProperties) -> &mut f32);
const SCALARS: &[Scalar] = &[
    ("transmission", |optics| &mut optics.transmission),
    ("ior", |optics| &mut optics.ior),
    ("roughness", |optics| &mut optics.roughness),
    ("attenuation_distance", |optics| {
        &mut optics.attenuation_distance
    }),
    ("wave_strength", |optics| &mut optics.wave_strength),
    ("wave_length", |optics| &mut optics.wave_length),
    ("wave_speed", |optics| &mut optics.wave_speed),
];
const TEXTURES: &[&str] = &[
    "texture", "side", "top", "bottom", "left", "right", "front", "back",
];

pub(super) fn parameters() -> Vec<Parameter> {
    let geometry = Type::union(
        MaterialGeometry::ALL
            .into_iter()
            .map(|geometry| Type::StringLiteral(geometry.name().into())),
    )
    .expect("voxel material geometry choices");
    let mut parameters = vec![
        parameter("color", Type::Color, false),
        parameter("emission", Type::Numeric(Unit::Number), false),
    ];
    parameters.extend(
        TEXTURES
            .iter()
            .map(|name| parameter(name, Type::Texture, false)),
    );
    parameters.push(parameter("geometry", geometry, false));
    parameters.push(parameter("attenuation_color", Type::Color, false));
    parameters.extend(
        SCALARS
            .iter()
            .map(|(name, _)| parameter(name, Type::Numeric(Unit::Number), false)),
    );
    parameters
}

pub(super) fn build(
    args: &imbl::OrdMap<String, Value>,
) -> Result<Arc<sanim_voxel::Material>, String> {
    let color = args.get("color").map(rgba).transpose()?.unwrap_or(u32::MAX);
    let texture = |name: &str| -> Result<Option<Arc<sanim_voxel::Texture>>, String> {
        args.get(name)
            .map(|value| {
                if let Value::Texture(texture) = value {
                    Ok(texture.clone())
                } else {
                    Err(format!("{name} requires a Texture value"))
                }
            })
            .transpose()
    };
    let all = texture("texture")?;
    let side = texture("side")?.or_else(|| all.clone());
    let faces = [
        texture("right")?.or_else(|| side.clone()),
        texture("left")?.or_else(|| side.clone()),
        texture("top")?.or_else(|| all.clone()),
        texture("bottom")?.or_else(|| all.clone()),
        texture("front")?.or_else(|| side.clone()),
        texture("back")?.or_else(|| side.clone()),
    ];
    let emission = args
        .get("emission")
        .map(Value::number)
        .transpose()?
        .map(|n| math::to_f64(n.value))
        .transpose()?
        .unwrap_or(0.) as f32;
    let geometry = args
        .get("geometry")
        .map(Value::string)
        .transpose()?
        .map(str::parse)
        .transpose()?
        .unwrap_or_default();
    let mut optics = OpticalProperties::default();
    for (name, field) in SCALARS {
        if let Some(value) = args.get(*name) {
            *field(&mut optics) = math::to_f64(value.number()?.value)? as f32;
        }
    }
    if let Some(value) = args.get("attenuation_color") {
        optics.attenuation_color = math::voxel::attenuation_color(value)?;
    }
    Ok(Arc::new(
        sanim_voxel::Material::textured(color, faces)?
            .with_emission(emission)?
            .with_geometry(geometry)?
            .with_optics(optics)?,
    ))
}
