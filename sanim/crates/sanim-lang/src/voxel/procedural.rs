//! Native scalar-field programs keep terrain evaluation out of the DSL interpreter.
use super::{coordinate_type, declaration, edit_material, parameter, required};
use crate::{
    native,
    prelude::{Builtin, Execution},
};
use sanim_core::*;
use sanim_voxel::procedural::{
    Axes, Binary, Field, Heightmap, Noise, NoiseKind, SurfacePoints, Unary,
};
use std::sync::Arc;

const BINARY_METHODS: [(&str, Binary); 8] = [
    ("Density.add", Binary::Add),
    ("Density.subtract", Binary::Subtract),
    ("Density.multiply", Binary::Multiply),
    ("Density.divide", Binary::Divide),
    ("Density.min", Binary::Min),
    ("Density.max", Binary::Max),
    ("Density.pow", Binary::Power),
    ("Density.mod", Binary::Modulo),
];
const UNARY_METHODS: [(&str, Unary); 6] = [
    ("Density.abs", Unary::Abs),
    ("Density.negate", Unary::Negate),
    ("Density.floor", Unary::Floor),
    ("Density.fract", Unary::Fract),
    ("Density.sin", Unary::Sin),
    ("Density.cos", Unary::Cos),
];

fn number() -> Type {
    Type::Numeric(Unit::Number)
}
fn vector() -> Type {
    Type::List(Box::new(number()), ListLength::Exact(3))
}
fn literals(values: impl IntoIterator<Item = &'static str>) -> Type {
    Type::union(
        values
            .into_iter()
            .map(|value| Type::StringLiteral(value.into())),
    )
    .expect("nonempty field literals")
}
fn builtin(
    name: &'static str,
    parameters: Vec<crate::prelude::Parameter>,
    result: Type,
) -> Builtin {
    let mut builtin = declaration(name, parameters, result);
    builtin.execute = Execution::Native(execute);
    builtin
}
pub(super) fn register(out: &mut Vec<Builtin>) {
    out.push(builtin(
        "Random",
        vec![parameter("seed", number(), false)],
        Type::Random,
    ));
    out.push(builtin(
        "Noise",
        vec![
            parameter("seed", number(), false),
            parameter("kind", literals(NoiseKind::ALL.map(NoiseKind::name)), false),
            parameter("scale", number(), false),
            parameter("octaves", number(), false),
            parameter("persistence", number(), false),
            parameter("lacunarity", number(), false),
            parameter("axes", literals(Axes::ALL.map(Axes::name)), false),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Density",
        vec![
            parameter("value", number(), false),
            parameter("axis", literals(["x", "y", "z"]), false),
            parameter("scale", number(), false),
            parameter("offset", number(), false),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Distance",
        vec![
            parameter("center", vector(), false),
            parameter("axes", literals(Axes::ALL.map(Axes::name)), false),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Instances",
        vec![
            parameter("asset", Type::VoxelGrid, true),
            parameter(
                "positions",
                Type::List(Box::new(coordinate_type()), ListLength::Unknown),
                true,
            ),
            parameter("size", vector(), false),
        ],
        Type::VoxelLayout,
    ));
}
pub(super) fn methods() -> Vec<Builtin> {
    let mut out = Vec::new();
    for name in ["Random.number", "Random.integer", "Random.chance"] {
        let mut parameters = vec![
            parameter("random", Type::Random, true),
            parameter("index", number(), true),
        ];
        if name == "Random.chance" {
            parameters.push(parameter("probability", number(), true));
        } else {
            parameters.extend([
                parameter("min", number(), name == "Random.integer"),
                parameter("max", number(), name == "Random.integer"),
            ]);
        }
        out.push(builtin(
            name,
            parameters,
            if name == "Random.chance" {
                Type::Bool
            } else {
                number()
            },
        ));
    }
    let scalar = Type::union([number(), Type::Density]).expect("field operands");
    for (name, _) in BINARY_METHODS {
        out.push(builtin(
            name,
            vec![
                parameter("field", Type::Density, true),
                parameter("value", scalar.clone(), true),
            ],
            Type::Density,
        ));
    }
    for (name, _) in UNARY_METHODS {
        out.push(builtin(
            name,
            vec![parameter("field", Type::Density, true)],
            Type::Density,
        ));
    }
    for name in ["Density.clamp", "Density.smoothstep"] {
        out.push(builtin(
            name,
            vec![
                parameter("field", Type::Density, true),
                parameter("min", number(), false),
                parameter("max", number(), false),
            ],
            Type::Density,
        ));
    }
    out.push(builtin(
        "Density.remap",
        vec![
            parameter("field", Type::Density, true),
            parameter("from_min", number(), true),
            parameter("from_max", number(), true),
            parameter("to_min", number(), true),
            parameter("to_max", number(), true),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Density.translate",
        vec![
            parameter("field", Type::Density, true),
            parameter("offset", vector(), true),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Density.scale",
        vec![
            parameter("field", Type::Density, true),
            parameter(
                "factor",
                Type::union([number(), vector()]).expect("domain scale"),
                true,
            ),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Density.warp",
        vec![
            parameter("field", Type::Density, true),
            parameter("seed", number(), false),
            parameter("scale", number(), false),
            parameter("strength", number(), false),
            parameter("axes", literals(Axes::ALL.map(Axes::name)), false),
        ],
        Type::Density,
    ));
    out.push(builtin(
        "Density.sample",
        vec![
            parameter("field", Type::Density, true),
            parameter("position", vector(), true),
        ],
        number(),
    ));
    let material = Type::union([Type::Color, Type::VoxelMaterial]).expect("field material");
    for name in [
        "VoxelGrid.heightmap",
        "VoxelGrid.density",
        "VoxelGrid.carve",
    ] {
        let mut parameters = vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("field", Type::Density, true),
        ];
        if name != "VoxelGrid.carve" {
            parameters.push(parameter("material", material.clone(), true));
        }
        if name != "VoxelGrid.heightmap" {
            parameters.extend([
                parameter("threshold", number(), false),
                parameter("min", coordinate_type(), false),
                parameter("max", coordinate_type(), false),
            ]);
        }
        out.push(builtin(name, parameters, Type::VoxelGrid));
    }
    out.push(builtin(
        "VoxelGrid.surface",
        vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("material", material.clone(), true),
            parameter("depth", number(), false),
            parameter("mask", Type::Density, false),
        ],
        Type::VoxelGrid,
    ));
    out.push(builtin(
        "VoxelGrid.flood",
        vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("level", number(), true),
            parameter("material", material.clone(), true),
        ],
        Type::VoxelGrid,
    ));
    out.push(builtin(
        "VoxelGrid.scatter",
        vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("material", material, true),
            parameter("density", scalar.clone(), false),
            parameter("seed", number(), false),
            parameter("step", number(), false),
        ],
        Type::VoxelGrid,
    ));
    out.push(builtin(
        "VoxelGrid.surface_points",
        vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("step", number(), false),
            parameter("seed", number(), false),
            parameter("jitter", number(), false),
            parameter("density", scalar, false),
            parameter("mask", Type::Density, false),
            parameter("max_slope", number(), false),
        ],
        Type::List(Box::new(vector()), ListLength::Unknown),
    ));
    out.push(builtin(
        "VoxelGrid.place_many",
        vec![
            parameter("grid", Type::VoxelGrid, true),
            parameter("child", Type::VoxelGrid, true),
            parameter(
                "positions",
                Type::List(Box::new(coordinate_type()), ListLength::Unknown),
                true,
            ),
        ],
        Type::VoxelGrid,
    ));
    out
}

fn scalar(args: &imbl::OrdMap<String, Value>, name: &str, default: f64) -> Result<f64, String> {
    args.get(name)
        .map(|value| value.number().and_then(|number| math::to_f64(number.value)))
        .unwrap_or(Ok(default))
}
fn integer(
    args: &imbl::OrdMap<String, Value>,
    name: &str,
    default: usize,
) -> Result<usize, String> {
    args.get(name)
        .map(|value| value.number()?.usize())
        .unwrap_or(Ok(default))
}
fn seed(args: &imbl::OrdMap<String, Value>, name: &str) -> Result<u64, String> {
    args.get(name)
        .map(|value| value.number()?.u64())
        .unwrap_or(Ok(0))
}
fn axes(args: &imbl::OrdMap<String, Value>) -> Result<Axes, String> {
    args.get("axes")
        .map(|value| value.string()?.parse())
        .unwrap_or(Ok(Axes::default()))
}
fn triple(value: &Value) -> Result<[f64; 3], String> {
    let values = value.list()?;
    if values.len() != 3 {
        return Err("field coordinates require exactly three Number components [x,y,z]".into());
    }
    let mut output = [0.0; 3];
    for (axis, value) in values.iter().enumerate() {
        output[axis] = math::to_f64(value.number()?.value)?;
    }
    Ok(output)
}
fn output(value: f64) -> Result<Value, String> {
    Ok(Value::Number(Number {
        value: math::functions::approximate(value)?,
        unit: Unit::Number,
    }))
}
fn density(value: &Value) -> Result<Arc<Field>, String> {
    match value {
        Value::Density(field) => Ok(field.clone()),
        Value::Number(number) => Ok(Arc::new(Field::constant(math::to_f64(number.value)?)?)),
        _ => Err("field operands require Density or Number".into()),
    }
}

fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: native::Context<'_>,
) -> Result<Value, String> {
    if name == "Instances" {
        use sanim_voxel::layout::{Coordinate, Dimension, MAX_NODES, Node, Placement, Space};
        let Value::VoxelGrid(asset) = required(args, "asset")? else {
            return Err("Instances requires a VoxelGrid asset".into());
        };
        let positions = required(args, "positions")?.list()?;
        if positions.len() + 1 > MAX_NODES {
            return Err(format!(
                "Instances needs {} layout nodes, exceeding {MAX_NODES}; use native scatter for dense foliage",
                positions.len() + 1
            ));
        }
        context.charge_work(positions.len() + 1)?;
        let size = args
            .get("size")
            .map(|value| super::coordinates(value, None))
            .transpose()?;
        if positions.is_empty() && size.is_none() {
            return Err("empty Instances requires an explicit size=[width,height,depth]".into());
        }
        let mut extent = [0; 3];
        let mut children = imbl::Vector::new();
        for position in positions.iter() {
            let at = super::coordinates(position, size)?;
            for axis in 0..3 {
                extent[axis] = extent[axis].max(
                    at[axis]
                        .checked_add(asset.dimensions()[axis])
                        .ok_or("instance position overflows the cell range")?,
                );
            }
            children.push_back(Arc::new(Node::part(
                asset.clone(),
                Placement {
                    absolute: true,
                    at: at.map(|value| Coordinate::Cells(Fraction::from(value))),
                    ..Default::default()
                },
            )?));
        }
        let size = size.unwrap_or(extent);
        if size
            .iter()
            .any(|axis| *axis == 0 || *axis > sanim_voxel::MAX_DIMENSION)
        {
            return Err(
                "instance bounds must be positive and within the maximum voxel dimension".into(),
            );
        }
        if (0..3).any(|axis| extent[axis] > size[axis]) {
            return Err(format!(
                "instance extent {extent:?} exceeds its declared size {size:?}"
            ));
        }
        return Ok(Value::VoxelLayout(Arc::new(Node::space(Space {
            size: size.map(Dimension::Cells),
            children,
            placement: Placement {
                absolute: true,
                ..Default::default()
            },
            ..Default::default()
        })?)));
    }
    if name == "Random" {
        return Ok(Value::Random(seed(args, "seed")?));
    }
    if name.starts_with("Random.") {
        let Value::Random(seed) = required(args, "random")? else {
            return Err("random receiver requires Random(seed=...)".into());
        };
        let sample = sanim_voxel::procedural::random(*seed, self::seed(args, "index")?);
        if name == "Random.chance" {
            let probability = scalar(args, "probability", 0.5)?;
            if !(0.0..=1.0).contains(&probability) {
                return Err("random probability must be within 0..=1".into());
            }
            return Ok(Value::Bool(sample < probability));
        }
        let min = args
            .get("min")
            .map(Value::number)
            .transpose()?
            .map_or(Fraction::from(0), |number| number.value);
        let max = args
            .get("max")
            .map(Value::number)
            .transpose()?
            .map_or(Fraction::from(1), |number| number.value);
        if !math::finite(&min) || !math::finite(&max) || min >= max {
            return Err("random range requires finite min < max; max is exclusive".into());
        }
        if name == "Random.integer" {
            if min.fract() != Fraction::from(0) || max.fract() != Fraction::from(0) {
                return Err("random integer bounds must be integers".into());
            }
            let min = math::to_f64(min)?;
            let max = math::to_f64(max)?;
            if min.abs() > (1u64 << 53) as f64
                || max.abs() > (1u64 << 53) as f64
                || (max - min) > (1u64 << 53) as f64
            {
                return Err("random integer bounds and their range must not exceed 2^53".into());
            }
            return output(min + (sample * (max - min)).floor());
        }
        // Keep the exclusive upper bound exact even when the range is tiny.
        // Quantize the approximate unit sample once, before exact scalar arithmetic.
        const RANDOM_STEPS: u64 = 1_000_000_000;
        let unit = Fraction::new((sample * RANDOM_STEPS as f64).floor() as u64, RANDOM_STEPS);
        let value = math::arithmetic(
            "+",
            min,
            math::arithmetic("*", math::arithmetic("-", max, min)?, unit)?,
        )?;
        return Ok(Value::Number(Number {
            value,
            unit: Unit::Number,
        }));
    }
    if name == "Noise" {
        let options = Noise {
            seed: seed(args, "seed")?,
            kind: args
                .get("kind")
                .map(Value::string)
                .transpose()?
                .unwrap_or("fbm")
                .parse()?,
            scale: scalar(args, "scale", 32.0)?,
            octaves: u32::try_from(integer(args, "octaves", 4)?)
                .map_err(|_| "noise octave count exceeds u32")?,
            persistence: scalar(args, "persistence", 0.5)?,
            lacunarity: scalar(args, "lacunarity", 2.0)?,
            axes: axes(args)?,
        };
        return Ok(Value::Density(Arc::new(Field::noise(options)?)));
    }
    if name == "Density" {
        let field = if let Some(axis) = args.get("axis") {
            if args.contains_key("value") {
                return Err("Density accepts either value=... or axis=..., not both".into());
            }
            let axis = match axis.string()? {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                _ => return Err("density axis requires x, y, or z".into()),
            };
            Field::axis(axis)?
                .binary(
                    Binary::Multiply,
                    &Field::constant(scalar(args, "scale", 1.0)?)?,
                )?
                .binary(Binary::Add, &Field::constant(scalar(args, "offset", 0.0)?)?)?
        } else {
            Field::constant(scalar(args, "value", 0.0)?)?
        };
        return Ok(Value::Density(Arc::new(field)));
    }
    if name == "Distance" {
        let center = args
            .get("center")
            .map(triple)
            .transpose()?
            .unwrap_or([0.0; 3]);
        return Ok(Value::Density(Arc::new(Field::distance(
            center,
            axes(args)?.mask(),
        )?)));
    }
    if name.starts_with("Density.") {
        let field = density(required(args, "field")?)?;
        if name == "Density.sample" {
            context.charge_work(field.sample_work())?;
            return output(field.sample(triple(required(args, "position")?)?)?);
        }
        let result = match name {
            "Density.clamp" => field.clamp(scalar(args, "min", 0.0)?, scalar(args, "max", 1.0)?)?,
            "Density.smoothstep" => {
                field.smoothstep(scalar(args, "min", 0.0)?, scalar(args, "max", 1.0)?)?
            }
            "Density.translate" => field.transform(triple(required(args, "offset")?)?, [1.0; 3])?,
            "Density.scale" => field.transform(
                [0.0; 3],
                match required(args, "factor")? {
                    Value::Number(value) => [math::to_f64(value.value)?; 3],
                    value => triple(value)?,
                },
            )?,
            "Density.warp" => field.warp(
                Noise {
                    seed: seed(args, "seed")?,
                    scale: scalar(args, "scale", 32.0)?,
                    octaves: 3,
                    axes: axes(args)?,
                    ..Default::default()
                },
                scalar(args, "strength", 3.0)?,
            )?,
            "Density.remap" => {
                let a = scalar(args, "from_min", 0.0)?;
                let b = scalar(args, "from_max", 1.0)?;
                let c = scalar(args, "to_min", 0.0)?;
                let d = scalar(args, "to_max", 1.0)?;
                if a == b {
                    return Err("density remap input bounds must differ".into());
                }
                field
                    .binary(Binary::Subtract, &Field::constant(a)?)?
                    .binary(Binary::Multiply, &Field::constant((d - c) / (b - a))?)?
                    .binary(Binary::Add, &Field::constant(c)?)?
            }
            _ => {
                if let Some((_, operation)) =
                    UNARY_METHODS.iter().find(|(method, _)| *method == name)
                {
                    field.unary(*operation)?
                } else if let Some((_, operation)) =
                    BINARY_METHODS.iter().find(|(method, _)| *method == name)
                {
                    field.binary(*operation, density(required(args, "value")?)?.as_ref())?
                } else {
                    return Err(format!("unregistered density method '{name}'"));
                }
            }
        };
        return Ok(Value::Density(Arc::new(result)));
    }
    let Value::VoxelGrid(grid) = required(args, "grid")? else {
        return Err("procedural grid operation requires VoxelGrid".into());
    };
    if name == "VoxelGrid.surface_points" {
        let probability_field = args.get("density").map(density).transpose()?;
        let mask = args.get("mask").map(density).transpose()?;
        let options = SurfacePoints {
            step: u32::try_from(integer(args, "step", 4)?)
                .map_err(|_| "surface step exceeds u32")?,
            seed: seed(args, "seed")?,
            jitter: scalar(args, "jitter", 0.0)?,
            density: probability_field.as_deref(),
            mask: mask.as_deref(),
            max_slope: args
                .get("max_slope")
                .map(|value| math::to_f64(value.number()?.value))
                .transpose()?,
        };
        context.charge_work(grid.surface_points_work(&options)?)?;
        let points = grid.surface_points(&options)?;
        let values = points
            .into_iter()
            .map(|point| {
                Value::List(List {
                    element_type: Arc::new(number()),
                    values: point
                        .into_iter()
                        .map(|value| {
                            Value::Number(Number {
                                value: Fraction::from(value),
                                unit: Unit::Number,
                            })
                        })
                        .collect(),
                })
            })
            .collect();
        return Ok(Value::List(List {
            element_type: Arc::new(vector()),
            values,
        }));
    }
    if name == "VoxelGrid.place_many" {
        let Value::VoxelGrid(child) = required(args, "child")? else {
            return Err("place_many child requires VoxelGrid".into());
        };
        let positions = required(args, "positions")?
            .list()?
            .iter()
            .map(|value| {
                super::coordinates(value, Some(grid.dimensions())).and_then(|point| {
                    point
                        .map(|value| {
                            i32::try_from(value)
                                .map_err(|_| "placement coordinate exceeds i32".to_string())
                        })
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()
                        .map(|values| [values[0], values[1], values[2]])
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut work = 0usize;
        for at in &positions {
            work = work.saturating_add(grid.placement_work(child, *at)?);
        }
        context.charge_work(work)?;
        return Ok(Value::VoxelGrid(Arc::new(
            grid.place_many(child, &positions)?,
        )));
    }
    if matches!(name, "VoxelGrid.density" | "VoxelGrid.carve") {
        let (volume, material) = if name == "VoxelGrid.carve" {
            (grid.as_ref().clone(), sanim_voxel::EMPTY)
        } else {
            edit_material(grid, required(args, "material")?)?
        };
        let field = density(required(args, "field")?)?;
        let min = args
            .get("min")
            .map(|value| super::coordinates(value, Some(volume.dimensions())))
            .transpose()?
            .unwrap_or([0; 3]);
        let max = args
            .get("max")
            .map(|value| super::coordinates(value, Some(volume.dimensions())))
            .transpose()?
            .unwrap_or(volume.dimensions());
        context.charge_work(volume.density_work(&field, min, max)?)?;
        return Ok(Value::VoxelGrid(Arc::new(volume.fill_density(
            &field,
            min,
            max,
            scalar(args, "threshold", 0.0)?,
            material,
        )?)));
    }
    let (volume, material) = edit_material(grid, required(args, "material")?)?;
    let result = match name {
        "VoxelGrid.scatter" => {
            let field = args
                .get("density")
                .map(density)
                .transpose()?
                .unwrap_or(Arc::new(Field::constant(1.0)?));
            let step =
                u32::try_from(integer(args, "step", 1)?).map_err(|_| "scatter step exceeds u32")?;
            context.charge_work(volume.scatter_work(&field, step)?)?;
            volume.scatter(&field, material, seed(args, "seed")?, step)?
        }
        "VoxelGrid.heightmap" => {
            let field = density(required(args, "field")?)?;
            context.charge_work(Heightmap::sample_work(&field, volume.dimensions())?)?;
            let plan = Heightmap::new(&field, volume.dimensions())?;
            context.charge_work(plan.work())?;
            plan.apply(&volume, material)?
        }
        "VoxelGrid.surface" => {
            let mask = args.get("mask").map(density).transpose()?;
            context.charge_work(volume.surface_paint_work(mask.as_deref())?)?;
            volume.paint_surface(
                material,
                u32::try_from(integer(args, "depth", 1)?)
                    .map_err(|_| "surface depth exceeds u32")?,
                mask.as_deref(),
            )?
        }
        "VoxelGrid.flood" => {
            let level =
                u32::try_from(integer(args, "level", 0)?).map_err(|_| "flood level exceeds u32")?;
            context.charge_work(volume.flood_work(level)?)?;
            volume.flood(level, material)?
        }
        _ => return Err(format!("unregistered procedural method '{name}'")),
    };
    Ok(Value::VoxelGrid(Arc::new(result)))
}
