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
        ty: Type::union([Type::String, Type::Texture]).expect("image source types"),
        required: true,
    }];
    parameters.extend(crate::prelude::element_parameters());
    output.push(Builtin {
        name: "Image",
        parameters,
        result: Type::Element,
        state_effect: false,
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
    let [width, height] = math::media::intrinsic_size([asset.width, asset.height])?;
    native::make_element_with_geometry(
        "Image",
        &args,
        Geometry::Media {
            asset,
            width,
            height,
        },
        path,
        &[],
        Span::default(),
    )
}

fn execute(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: native::Context<'_>,
) -> Result<Value, String> {
    let asset = match args.get("src") {
        Some(Value::Texture(texture)) => Arc::new(media::Asset::from_texture(texture.clone())),
        _ => {
            let Some(Value::Element(source)) = args.get("asset") else {
                return Err(
                    "Image requires a Texture value or a supported bitmap/SVG source path".into(),
                );
            };
            let Geometry::Media { asset, .. } = source.geometry.as_ref() else {
                return Err("Image requires a supported bitmap or SVG source".into());
            };
            asset.clone()
        }
    };
    let explicit_width = args.contains_key("width");
    let explicit_height = args.contains_key("height");
    let mut args = args.clone();
    let [width, height] = math::media::element_size(
        [asset.width, asset.height],
        args.get("width").map(Value::number).transpose()?,
        args.get("height").map(Value::number).transpose()?,
    )?;
    args.entry("width".into()).or_insert(Value::Number(Number {
        value: width,
        unit: Unit::Length,
    }));
    args.entry("height".into()).or_insert(Value::Number(Number {
        value: height,
        unit: Unit::Length,
    }));
    let Value::Element(mut root) = native::make_element_with_geometry(
        "Image",
        &args,
        Geometry::Media {
            asset: asset.clone(),
            width,
            height,
        },
        context.module,
        context.identity,
        context.span,
    )?
    else {
        unreachable!()
    };
    let element = Arc::make_mut(&mut root);
    if !explicit_width {
        element.layout.width = None;
    }
    if !explicit_height {
        element.layout.height = None;
    }
    Ok(Value::Element(root))
}
