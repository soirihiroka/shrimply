//! Native image elements share layout and source watching with other elements.
use crate::{
    native, native_types,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::*;
use std::sync::Arc;

pub(crate) fn register(output: &mut Vec<Builtin>) {
    let mut parameters = vec![Parameter {
        name: "src",
        ty: Type::String,
        required: true,
    }];
    parameters.extend(crate::prelude::element_parameters());
    output.push(Builtin {
        name: "Image",
        parameters,
        result: Type::Element,
        schedule: false,
        timing: Default::default(),
        content: Default::default(),
        execute: Execution::Source(execute),
        check: native_types::fixed,
    });
}

pub(crate) fn resource(path: &str, asset: Arc<media::Asset>) -> Result<Value, String> {
    let mut args = imbl::OrdMap::new();
    args.insert(
        "id".into(),
        Value::String(format!("media:{path}:{}", asset.fingerprint()).into()),
    );
    let Value::Element(mut root) = native::make_element("Rect", &args, path, &[], Span::default())?
    else {
        unreachable!()
    };
    let [width, height] = math::media::intrinsic_size([asset.width, asset.height])?;
    Arc::make_mut(&mut root).geometry = Arc::new(Geometry::Media {
        asset,
        width,
        height,
    });
    Ok(Value::Element(root))
}

fn execute(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: native::Context<'_>,
) -> Result<Value, String> {
    let Some(Value::Element(source)) = args.get("asset") else {
        return Err("Image source was not loaded by the resource runtime".into());
    };
    let Geometry::Media {
        asset,
        width,
        height,
    } = source.geometry.as_ref()
    else {
        return Err("Image requires a supported bitmap or SVG source".into());
    };
    let explicit_width = args.contains_key("width");
    let explicit_height = args.contains_key("height");
    let mut args = args.clone();
    let mut width = *width;
    let mut height = *height;
    match (args.get("width"), args.get("height")) {
        (Some(value), None) if value.number()?.unit == Unit::Length => {
            width = value.number()?.value;
            height = math::arithmetic(
                "*",
                width,
                Fraction::new(u64::from(asset.height), u64::from(asset.width)),
            )?;
        }
        (None, Some(value)) if value.number()?.unit == Unit::Length => {
            height = value.number()?.value;
            width = math::arithmetic(
                "*",
                height,
                Fraction::new(u64::from(asset.width), u64::from(asset.height)),
            )?;
        }
        _ => {}
    }
    args.entry("width".into()).or_insert(Value::Number(Number {
        value: width,
        unit: Unit::Length,
    }));
    args.entry("height".into()).or_insert(Value::Number(Number {
        value: height,
        unit: Unit::Length,
    }));
    let Value::Element(mut root) = native::make_element(
        "Rect",
        &args,
        context.module,
        context.identity,
        context.span,
    )?
    else {
        unreachable!()
    };
    let element = Arc::make_mut(&mut root);
    element.geometry = Arc::new(Geometry::Media {
        asset: asset.clone(),
        width,
        height,
    });
    if !explicit_width {
        element.layout.width = None;
    }
    if !explicit_height {
        element.layout.height = None;
    }
    Ok(Value::Element(root))
}
