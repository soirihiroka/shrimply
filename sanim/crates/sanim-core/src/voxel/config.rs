//! One configuration boundary for prepared and signal-driven voxel layout trees.
use crate::{Fraction, ListLength, Number, Type, Unit, Value, math};
use num_traits::ToPrimitive;
use sanim_voxel::layout::{Align, Axis, Coordinate, Dimension, Justify, Node, Placement, Space};
use std::sync::Arc;

fn number_list() -> Type {
    Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3))
}
pub fn coordinate_type() -> Type {
    Type::List(
        Box::new(
            Type::union([Type::Numeric(Unit::Number), Type::Numeric(Unit::Percent)])
                .expect("voxel coordinate units"),
        ),
        ListLength::Exact(3),
    )
}
fn choices(names: &[&str]) -> Type {
    Type::union(names.iter().map(|name| Type::StringLiteral((*name).into())))
        .expect("voxel choices")
}

pub fn signature(name: &str) -> (Vec<(&'static str, Type, bool)>, Type) {
    let alignment = Type::List(
        Box::new(choices(&["start", "center", "end"])),
        ListLength::Exact(3),
    );
    let mut parameters = if name == "VoxelPart" {
        vec![
            ("volume", Type::VoxelGrid, true),
            ("align", alignment, false),
        ]
    } else {
        vec![
            (
                "size",
                Type::List(
                    Box::new(
                        Type::union([
                            Type::Numeric(Unit::Number),
                            Type::Numeric(Unit::Percent),
                            Type::StringLiteral("auto".into()),
                        ])
                        .expect("voxel dimensions"),
                    ),
                    ListLength::Exact(3),
                ),
                false,
            ),
            ("flow", choices(&["x", "y", "z"]), false),
            ("gap", Type::Numeric(Unit::Number), false),
            (
                "padding",
                Type::union([Type::Numeric(Unit::Number), number_list()]).expect("voxel padding"),
                false,
            ),
            (
                "justify",
                choices(&["start", "center", "end", "between", "around", "evenly"]),
                false,
            ),
            ("align", alignment, false),
            (
                "children",
                Type::List(Box::new(Type::VoxelLayout), ListLength::Unknown),
                false,
            ),
        ]
    };
    parameters.extend([
        ("position_mode", choices(&["relative", "absolute"]), false),
        ("at", coordinate_type(), false),
        ("anchor", coordinate_type(), false),
        ("offset", number_list(), false),
    ]);
    (parameters, Type::VoxelLayout)
}

fn scalar(value: &Value) -> Result<u32, String> {
    let value = cells(value)?;
    if value < Fraction::from(0) {
        return Err("voxel layout sizes, gaps, and padding require nonnegative cell counts".into());
    }
    value
        .floor()
        .to_u32()
        .ok_or_else(|| "voxel layout cell count is too large".into())
}
fn cells(value: &Value) -> Result<Fraction, String> {
    let number = value.number()?;
    if number.unit != Unit::Number || !math::finite(&number.value) {
        return Err("voxel layout requires finite dimensionless cell counts".into());
    }
    Ok(number.value)
}
fn triple<T>(
    value: &Value,
    mut convert: impl FnMut(&Value) -> Result<T, String>,
) -> Result<[T; 3], String> {
    let values = value.list()?;
    if values.len() != 3 {
        return Err("voxel layout requires exactly three components [x,y,z]".into());
    }
    Ok([
        convert(values.get(0).expect("three components"))?,
        convert(values.get(1).expect("three components"))?,
        convert(values.get(2).expect("three components"))?,
    ])
}
fn align(value: &Value) -> Result<[Align; 3], String> {
    triple(value, |value| match value.string()? {
        "start" => Ok(Align::Start),
        "center" => Ok(Align::Center),
        "end" => Ok(Align::End),
        _ => Err("voxel alignment requires start, center, or end".into()),
    })
}
fn coordinate(value: &Value) -> Result<Coordinate, String> {
    let number = value.number()?;
    match number.unit {
        Unit::Number => cells(value).map(Coordinate::Cells),
        Unit::Percent => {
            math::arithmetic("*", number.value, Fraction::from(100)).map(Coordinate::Percent)
        }
        _ => Err("voxel placement requires cell counts or percentages".into()),
    }
}
/// Shared coordinate decoding for layout, lights, and cameras. Values can be outside the containing volume.
pub fn coordinates(value: &Value) -> Result<[Coordinate; 3], String> {
    triple(value, coordinate)
}

fn placement(args: &imbl::OrdMap<String, Value>) -> Result<Placement, String> {
    let absolute = match args
        .get("position_mode")
        .map(Value::string)
        .transpose()?
        .unwrap_or("relative")
    {
        "relative" => false,
        "absolute" => true,
        _ => return Err("voxel position_mode requires relative or absolute".into()),
    };
    let mut placement = Placement {
        absolute,
        ..Default::default()
    };
    if let Some(value) = args.get("at") {
        placement.at = coordinates(value)?;
    }
    if let Some(value) = args.get("anchor") {
        placement.anchor = coordinates(value)?;
    }
    if let Some(value) = args.get("offset") {
        placement.offset = triple(value, cells)?;
    }
    Ok(placement)
}

pub fn execute(name: &str, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
    let mut placement = placement(args)?;
    if name == "VoxelPart" {
        let Some(Value::VoxelGrid(volume)) = args.get("volume") else {
            return Err("VoxelPart requires a VoxelGrid volume".into());
        };
        placement.align = args.get("align").map(align).transpose()?;
        return Node::part(volume.clone(), placement)
            .map(|node| Value::VoxelLayout(Arc::new(node)));
    }
    let mut space = Space {
        placement,
        ..Default::default()
    };
    if let Some(size) = args.get("size") {
        space.size = triple(size, |value| match value {
            Value::String(name) if name.as_ref() == "auto" => Ok(Dimension::Auto),
            Value::Number(Number {
                unit: Unit::Percent,
                value,
            }) => math::arithmetic("*", *value, Fraction::from(100)).map(Dimension::Percent),
            _ => scalar(value).map(Dimension::Cells),
        })?;
    }
    space.flow = match args
        .get("flow")
        .map(Value::string)
        .transpose()?
        .unwrap_or("x")
    {
        "x" => Axis::X,
        "y" => Axis::Y,
        "z" => Axis::Z,
        _ => return Err("voxel flow requires x, y, or z".into()),
    };
    if let Some(value) = args.get("gap") {
        space.gap = scalar(value)?;
    }
    if let Some(value) = args.get("padding") {
        space.padding = if matches!(value, Value::Number(_)) {
            [scalar(value)?; 3]
        } else {
            triple(value, scalar)?
        };
    }
    space.justify = match args
        .get("justify")
        .map(Value::string)
        .transpose()?
        .unwrap_or("start")
    {
        "start" => Justify::Start,
        "center" => Justify::Center,
        "end" => Justify::End,
        "between" => Justify::Between,
        "around" => Justify::Around,
        "evenly" => Justify::Evenly,
        _ => {
            return Err(
                "voxel justify requires start, center, end, between, around, or evenly".into(),
            );
        }
    };
    if let Some(value) = args.get("align") {
        space.align = align(value)?;
    }
    if let Some(value) = args.get("children") {
        space.children = value
            .list()?
            .iter()
            .map(|value| {
                let Value::VoxelLayout(node) = value else {
                    return Err("VoxelBox children must be VoxelBox or VoxelPart values".into());
                };
                Ok(node.clone())
            })
            .collect::<Result<_, String>>()?;
    }
    Node::space(space).map(|node| Value::VoxelLayout(Arc::new(node)))
}
