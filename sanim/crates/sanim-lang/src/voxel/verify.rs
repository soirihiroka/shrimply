//! Pure snapshot assertions use canonical sparse layout and camera validators.
use super::{declaration, parameter, required};
use crate::{
    native,
    prelude::{Builtin, Execution},
};
use sanim_core::{Geometry, Type, Unit, Value, math};

pub(super) fn methods() -> Vec<Builtin> {
    let mut result = Vec::new();
    for (name, first, second) in [
        ("VoxelLayout.assert_disjoint", "first", "second"),
        ("VoxelLayout.assert_within", "inner", "outer"),
    ] {
        result.push(declaration(
            name,
            vec![
                parameter("root", Type::VoxelLayout, true),
                parameter(first, Type::VoxelLayout, true),
                parameter(second, Type::VoxelLayout, true),
            ],
            Type::Unit,
        ));
    }
    result.push(declaration(
        "Element.assert_framed",
        vec![
            parameter("scene", Type::Element, true),
            parameter("margin", Type::Numeric(Unit::Percent), false),
        ],
        Type::Unit,
    ));
    for builtin in &mut result {
        builtin.execute = Execution::Native(execute);
    }
    result
}

fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: native::Context<'_>,
) -> Result<Value, String> {
    let statistics = if name == "Element.assert_framed" {
        let Value::Element(element) = required(args, "scene")? else {
            return Err("assert_framed requires a VoxelScene element".into());
        };
        let Geometry::Voxel(scene) = element.geometry.as_ref() else {
            return Err(
                "assert_framed currently supports VoxelScene; use it on the scene element".into(),
            );
        };
        let viewport = [context.animation.width, context.animation.height];
        let camera = scene.resolve_camera(viewport)?;
        let margin = args
            .get("margin")
            .map(Value::number)
            .transpose()?
            .map(|number| math::to_f64(number.value))
            .transpose()?
            .unwrap_or(math::to_f64(scene.frame_margin)?);
        sanim_voxel::verify::assert_framed(&camera, scene.dimensions(), viewport, margin)?
    } else {
        let layout = |name| match required(args, name)? {
            Value::VoxelLayout(layout) => Ok(layout),
            _ => Err(format!("{name} requires VoxelPart or VoxelBox")),
        };
        let root = layout("root")?;
        let plan = sanim_voxel::layout::prepare(root)?;
        let maximum = usize::try_from(context.remaining_work).unwrap_or(usize::MAX);
        if name == "VoxelLayout.assert_disjoint" {
            sanim_voxel::verify::assert_disjoint(
                root,
                plan.placements(),
                layout("first")?,
                layout("second")?,
                maximum,
            )?
        } else {
            sanim_voxel::verify::assert_within(
                root,
                plan.placements(),
                layout("inner")?,
                layout("outer")?,
                maximum,
            )?
        }
    };
    context.charge_work(statistics.work)?;
    Ok(Value::Unit)
}
