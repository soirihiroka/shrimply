//! Pure voxel construction and edits reuse the language's native registry and caches.
mod camera;
mod material;
mod procedural;
mod verify;
use crate::{
    native,
    prelude::{Builtin, Execution, Parameter, parameter},
};
use sanim_core::*;
use std::sync::{Arc, OnceLock};

fn declaration(name: &'static str, parameters: Vec<Parameter>, result: Type) -> Builtin {
    Builtin::pure(name, parameters, result, execute)
}

use sanim_core::voxel::config::coordinate_type;

pub(crate) fn register(out: &mut Vec<Builtin>) {
    camera::register(out);
    procedural::register(out);
    for operation in [
        code::values::Native::VoxelBox,
        code::values::Native::VoxelPart,
        code::values::Native::SunLight,
        code::values::Native::PointLight,
        code::values::Native::Environment,
    ] {
        let (parameters, result) = operation.signature();
        let mut builtin = declaration(
            operation.name(),
            parameters
                .into_iter()
                .map(|(name, ty, required)| parameter(name, ty, required))
                .collect(),
            result,
        );
        builtin.execute = Execution::Native(native::configuration);
        out.push(builtin);
    }
    out.push(declaration(
        "VoxelGrid",
        vec![parameter(
            "size",
            Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3)),
            true,
        )],
        Type::VoxelGrid,
    ));
    out.push(declaration(
        "VoxelMaterial",
        material::parameters(),
        Type::VoxelMaterial,
    ));
    let mut scene = vec![
        parameter("id", Type::String, false),
        parameter("opacity", Type::Numeric(Unit::Number), false),
    ];
    scene.extend(sanim_core::voxel::INPUTS.iter().map(|(name, property)| {
        parameter(
            name,
            sanim_core::voxel::input_type(*property).expect("voxel input type"),
            false,
        )
    }));
    let mut scene = declaration("VoxelScene", scene, Type::Element);
    scene.check = scene_type;
    out.push(scene);
}

fn scene_type(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let supplied = |name: &str| {
        builtin
            .parameters
            .iter()
            .zip(actuals)
            .any(|(parameter, ty)| parameter.name == name && *ty != Type::Unit)
    };
    if supplied("camera")
        && ["view", "yaw", "pitch", "zoom", "fov"]
            .into_iter()
            .any(supplied)
    {
        return Err("VoxelScene camera= cannot be combined with fitted view/yaw/pitch/zoom/fov; configure pose and lens on Camera(...) instead".into());
    }
    for (parameter, actual) in builtin.parameters.iter().zip(actuals) {
        if parameter.name == "lights"
            && let Type::List(_, ListLength::Exact(length)) = actual
            && *length > sanim_voxel::MAX_LIGHTS
        {
            return Err(format!(
                "VoxelScene lights has {length} entries; maximum {}. Merge lights or reduce their count",
                sanim_voxel::MAX_LIGHTS
            ));
        }
    }
    let supplied = builtin
        .parameters
        .iter()
        .zip(actuals)
        .filter(|(parameter, ty)| {
            matches!(parameter.name, "volume" | "layout") && **ty != Type::Unit
        })
        .count();
    if supplied != 1 {
        return Err("VoxelScene requires exactly one of volume=VoxelGrid(...) or layout=<VoxelBox>...</VoxelBox>".into());
    }
    Ok(builtin.result.clone())
}

pub(crate) fn methods() -> &'static [Builtin] {
    static METHODS: OnceLock<Vec<Builtin>> = OnceLock::new();
    METHODS.get_or_init(|| {
        let material =
            Type::union([Type::Color, Type::VoxelMaterial]).expect("voxel material inputs");
        let mut methods = vec![
            declaration(
                "VoxelGrid.set",
                vec![
                    parameter("grid", Type::VoxelGrid, true),
                    parameter("position", coordinate_type(), true),
                    parameter("material", material.clone(), true),
                ],
                Type::VoxelGrid,
            ),
            declaration(
                "VoxelGrid.fill",
                vec![
                    parameter("grid", Type::VoxelGrid, true),
                    parameter("min", coordinate_type(), true),
                    parameter("max", coordinate_type(), true),
                    parameter("material", material, true),
                ],
                Type::VoxelGrid,
            ),
            declaration(
                "VoxelGrid.get",
                vec![
                    parameter("grid", Type::VoxelGrid, true),
                    parameter("position", coordinate_type(), true),
                ],
                Type::Option(Box::new(Type::VoxelMaterial)),
            ),
            declaration(
                "VoxelGrid.erase",
                vec![
                    parameter("grid", Type::VoxelGrid, true),
                    parameter("min", coordinate_type(), true),
                    parameter("max", coordinate_type(), true),
                ],
                Type::VoxelGrid,
            ),
            declaration(
                "VoxelGrid.place",
                vec![
                    parameter("grid", Type::VoxelGrid, true),
                    parameter("child", Type::VoxelGrid, true),
                    parameter("at", coordinate_type(), true),
                    parameter("anchor", coordinate_type(), false),
                    parameter(
                        "offset",
                        Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(3)),
                        false,
                    ),
                ],
                Type::VoxelGrid,
            ),
        ];
        methods.extend(verify::methods());
        methods.extend(camera::methods());
        methods.extend(procedural::methods());
        for method in &mut methods {
            crate::prelude::finalize(method);
        }
        methods
    })
}

pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    methods().iter().filter(move |method| {
        method
            .parameters
            .first()
            .is_some_and(|parameter| parameter.ty.accepts(receiver))
    })
}

use crate::native::required;

fn coordinates(value: &Value, dimensions: Option<[u32; 3]>) -> Result<[u32; 3], String> {
    let values = value.list()?;
    if values.len() != 3 {
        return Err("voxel coordinates require exactly three numeric values [x,y,z]".into());
    }
    let mut result = [0; 3];
    for (axis, value) in values.iter().enumerate() {
        let number = value.number()?;
        let value=match number.unit {
            Unit::Number=>number.value,
            Unit::Percent if dimensions.is_some()=>math::arithmetic("*",number.value,Fraction::from(dimensions.expect("percentage dimensions")[axis]))?.floor(),
            _=>return Err("voxel coordinates require dimensionless integer cell indices or parent-relative percentages".into()),
        };
        let index = Number {
            value,
            unit: Unit::Number,
        }
        .usize()?;
        result[axis] = u32::try_from(index)
            .map_err(|_| "voxel coordinate exceeds the supported integer range")?;
    }
    Ok(result)
}

fn rgba(value: &Value) -> Result<u32, String> {
    let Value::Color(color) = value else {
        return Err("voxel material color requires Color(...)".into());
    };
    if color.0[3] != Fraction::from(0) && color.0[3] != Fraction::from(1) {
        return Err(
            "voxel material colors require opaque alpha 1; use VoxelMaterial(transmission=...) for glass/water transparency, or transparent alpha 0 to erase cells"
                .into(),
        );
    }
    let bytes = color.rgba8()?;
    Ok(u32::from_be_bytes(bytes))
}

fn edit_material(
    volume: &sanim_voxel::Volume,
    value: &Value,
) -> Result<(sanim_voxel::Volume, u32), String> {
    let material = match value {
        Value::VoxelMaterial(material) => material.clone(),
        Value::Color(_) => {
            let color = rgba(value)?;
            if color & 0xff == 0 {
                return Ok((volume.clone(), sanim_voxel::EMPTY));
            }
            Arc::new(sanim_voxel::Material::solid(color)?)
        }
        _ => return Err("voxel cells require a Color or VoxelMaterial".into()),
    };
    volume.with_material(material)
}

fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: native::Context<'_>,
) -> Result<Value, String> {
    if name == "VoxelGrid" {
        let size = coordinates(required(args, "size")?, None)?;
        return Ok(Value::VoxelGrid(Arc::new(
            sanim_voxel::Volume::with_limits(
                size,
                sanim_voxel::Limits {
                    max_bytes: context
                        .limits
                        .data
                        .max_bytes
                        .min(sanim_voxel::MAX_RESIDENT_BYTES),
                    ..Default::default()
                },
            )?,
        )));
    }
    if name == "VoxelMaterial" {
        return material::build(args).map(Value::VoxelMaterial);
    }
    if name == "VoxelScene" {
        let scene=match(args.get("volume"),args.get("layout")){
            (Some(Value::VoxelGrid(volume)),None)=>sanim_core::voxel::Scene{source:sanim_voxel::SceneSource::Volume(volume.clone()),layout:None,placements:Arc::from([]),camera_spec:None,camera:Default::default(),lights:None,environment:None,check_framing:false,frame_margin:Fraction::from(0)},
            (None,Some(Value::VoxelLayout(layout)))=>{
                let resolved=sanim_core::voxel::resolve_instances_layout(layout)?;
                sanim_core::voxel::Scene{source:resolved.source.clone(),layout:Some(layout.clone()),placements:resolved.placements.clone(),camera_spec:None,camera:Default::default(),lights:None,environment:None,check_framing:false,frame_margin:Fraction::from(0)}
            }
            (None,None)=>return Err("VoxelScene requires either volume=VoxelGrid(...) or layout=<VoxelBox>...</VoxelBox>".into()),
            (Some(_),Some(_))=>return Err("VoxelScene accepts either volume or layout, not both".into()),
            _=>return Err("VoxelScene volume requires VoxelGrid; layout requires VoxelBox or VoxelPart".into()),
        };
        let mut geometry = Geometry::Voxel(Arc::new(scene));
        for (name, property) in sanim_core::voxel::INPUTS {
            if matches!(property, Property::VoxelVolume | Property::VoxelLayout) {
                continue;
            }
            if let Some(value) = args.get(*name) {
                sanim_core::voxel::set(&mut geometry, *property, value.clone())?;
            }
        }
        return native::make_element_with_geometry(
            name,
            args,
            geometry,
            context.module,
            context.identity,
            context.span,
        );
    }
    let Value::VoxelGrid(grid) = required(args, "grid")? else {
        return Err("voxel edit receiver requires VoxelGrid".into());
    };
    if name == "VoxelGrid.place" {
        let Value::VoxelGrid(child) = required(args, "child")? else {
            return Err("voxel placement child requires VoxelGrid".into());
        };
        let origin = math::voxel::placement_origin(
            required(args, "at")?,
            args.get("anchor"),
            args.get("offset"),
            grid.dimensions(),
            child.dimensions(),
        )?;
        context.charge_work(grid.placement_work(child, origin)?)?;
        return grid
            .place(child, origin)
            .map(|volume| Value::VoxelGrid(Arc::new(volume)));
    }
    if name == "VoxelGrid.get" {
        let position = coordinates(required(args, "position")?, Some(grid.dimensions()))?;
        let id = grid.get(position)?;
        return Ok(grid
            .material(id)
            .map(|material| Value::Some(Arc::new(Value::VoxelMaterial(material.clone()))))
            .unwrap_or(Value::None));
    }
    let (volume, id) = if name == "VoxelGrid.erase" {
        (grid.as_ref().clone(), sanim_voxel::EMPTY)
    } else {
        edit_material(grid, required(args, "material")?)?
    };
    let volume = if name == "VoxelGrid.set" {
        context.charge_work(sanim_voxel::CHUNK_VOXELS)?;
        volume.set(
            coordinates(required(args, "position")?, Some(volume.dimensions()))?,
            id,
        )?
    } else {
        let min = coordinates(required(args, "min")?, Some(volume.dimensions()))?;
        let max = coordinates(required(args, "max")?, Some(volume.dimensions()))?;
        let work = volume.fill_work(min, max)?;
        context.charge_work(work)?;
        volume.fill_box(min, max, id)?
    };
    Ok(Value::VoxelGrid(Arc::new(volume)))
}
