//! Canonical immutable raster operations shared by preparation and retained signals.
mod cache;
pub mod draw;
mod edit;
mod image;
mod raster;
use crate::code::values::NativeCost as Cost;
use crate::{ListLength, Type, Unit, Value, math};
pub use cache::{CacheStats, cache_stats};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
pub const MAX_BYTES: usize = crate::reactive::MAX_REACTIVE_BYTES;
pub const MAX_WORK: usize = sanim_voxel::procedural::MAX_PROCEDURAL_WORK;
type Arguments = imbl::OrdMap<String, Value>;
type Signature = Vec<(&'static str, Type, bool)>;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Texture,
    Size,
    Pixel,
    Crop,
    Resize,
    Flip,
    Rotate,
    Pad,
    Tile,
    Blur,
    Sharpen,
    Grayscale,
    Invert,
    Brightness,
    Contrast,
    Saturation,
    Hue,
    Blend,
    Composite,
    Mask,
    WithAlpha,
    Cutout,
    Draw,
}
impl Op {
    pub const ALL: [Self; 23] = [
        Self::Texture,
        Self::Size,
        Self::Pixel,
        Self::Crop,
        Self::Resize,
        Self::Flip,
        Self::Rotate,
        Self::Pad,
        Self::Tile,
        Self::Blur,
        Self::Sharpen,
        Self::Grayscale,
        Self::Invert,
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Hue,
        Self::Blend,
        Self::Composite,
        Self::Mask,
        Self::WithAlpha,
        Self::Cutout,
        Self::Draw,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Texture => "Texture",
            Self::Size => "Texture.size",
            Self::Pixel => "Texture.pixel",
            Self::Crop => "Texture.crop",
            Self::Resize => "Texture.resize",
            Self::Flip => "Texture.flip",
            Self::Rotate => "Texture.rotate",
            Self::Pad => "Texture.pad",
            Self::Tile => "Texture.tile",
            Self::Blur => "Texture.blur",
            Self::Sharpen => "Texture.sharpen",
            Self::Grayscale => "Texture.grayscale",
            Self::Invert => "Texture.invert",
            Self::Brightness => "Texture.brightness",
            Self::Contrast => "Texture.contrast",
            Self::Saturation => "Texture.saturation",
            Self::Hue => "Texture.hue",
            Self::Blend => "Texture.blend",
            Self::Composite => "Texture.composite",
            Self::Mask => "Texture.mask",
            Self::WithAlpha => "Texture.with_alpha",
            Self::Cutout => "Texture.cutout",
            Self::Draw => "Texture.draw",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.name() == name)
    }
    pub fn signature(self) -> (Signature, Type) {
        let num = || Type::Numeric(Unit::Number);
        let mut args = if self == Self::Texture {
            vec![]
        } else {
            vec![("texture", Type::Texture, true)]
        };
        let space = || choices(math::color::MixSpace::ALL.map(|space| space.name()));
        args.extend(match self {
            Self::Texture => vec![
                ("size", pair_type(), true),
                (
                    "pixels",
                    Type::List(Box::new(Type::Color), ListLength::Unknown),
                    false,
                ),
                ("fill", Type::Color, false),
                ("field", Type::Density, false),
                (
                    "colors",
                    Type::List(Box::new(Type::Color), ListLength::Unknown),
                    false,
                ),
                ("mask", Type::Density, false),
                ("alpha", Type::Density, false),
                ("space", space(), false),
            ],
            Self::Size | Self::Grayscale | Self::Invert => vec![],
            Self::Pixel => vec![("at", pair_type(), true)],
            Self::Crop => vec![("min", pair_type(), true), ("size", pair_type(), true)],
            Self::Resize => vec![
                ("size", pair_type(), true),
                (
                    "filter",
                    choices(image::FILTERS.map(|(name, _)| name)),
                    false,
                ),
            ],
            Self::Flip => vec![("axes", choices(image::FLIP_AXES), false)],
            Self::Rotate => vec![
                ("angle", Type::Numeric(Unit::Angle), false),
                ("expand", Type::Bool, false),
                ("filter", choices(["nearest", "triangle"]), false),
                ("fill", Type::Color, false),
            ],
            Self::Pad => vec![
                ("size", pair_type(), true),
                ("at", pair_type(), false),
                ("fill", Type::Color, false),
            ],
            Self::Tile => vec![("size", pair_type(), true)],
            Self::Blur => vec![("radius", num(), false)],
            Self::Sharpen => vec![
                ("radius", num(), false),
                ("amount", num(), false),
                ("threshold", num(), false),
            ],
            Self::Brightness | Self::Contrast | Self::Saturation => vec![("factor", num(), false)],
            Self::Hue => vec![("angle", Type::Numeric(Unit::Angle), true)],
            Self::Blend => vec![
                ("other", Type::Texture, true),
                ("amount", num(), false),
                ("space", space(), false),
            ],
            Self::Composite => vec![
                ("over", Type::Texture, true),
                ("at", pair_type(), false),
                ("mode", Type::String, false),
                ("opacity", num(), false),
            ],
            Self::Mask => vec![
                ("mask", Type::Texture, true),
                ("channel", choices(image::MASK_CHANNELS), false),
            ],
            Self::WithAlpha => vec![
                (
                    "alpha",
                    Type::union([Type::Texture, num()]).expect("alpha inputs"),
                    true,
                ),
                ("channel", choices(image::MASK_CHANNELS), false),
            ],
            Self::Cutout => vec![("threshold", num(), false)],
            Self::Draw => vec![("content", Type::Element, true)],
        });
        (
            args,
            match self {
                Self::Size => pair_type(),
                Self::Pixel => Type::Color,
                _ => Type::Texture,
            },
        )
    }
    pub fn check(
        self,
        args: &std::collections::BTreeMap<String, Type>,
        result: Type,
    ) -> Result<Type, String> {
        if self == Self::Texture {
            raster::check(args)?;
        }
        Ok(result)
    }
    pub fn execute(self, args: &Arguments) -> Result<Value, String> {
        self.evaluate(args, |_| Ok(()))
    }
    pub fn evaluate(
        self,
        args: &Arguments,
        mut reserve: impl FnMut(Cost) -> Result<(), String>,
    ) -> Result<Value, String> {
        let key = if self == Self::Draw {
            draw::key(args)?
        } else {
            self.cached().then(|| cache::key(self, args)).transpose()?
        };
        if let Some(key) = &key
            && let Some(texture) = cache::get(key)
        {
            reserve(Cost {
                work: 0,
                bytes: 0,
                output_bytes: texture.pixels().len(),
            })?;
            return Ok(Value::Texture(texture));
        }
        let cost = if self == Self::Texture {
            raster::cost(args)?
        } else if self == Self::Draw {
            draw::cost(args)?
        } else {
            edit::cost(self, args)?
        };
        if cost.bytes.max(cost.output_bytes) > MAX_BYTES {
            return Err(format!(
                "{} requires {} temporary bytes, exceeding the texture byte budget {MAX_BYTES}",
                self.name(),
                cost.bytes
            ));
        }
        if cost.work > MAX_WORK {
            return Err(format!(
                "{} requires {} work units, exceeding the texture work budget {MAX_WORK}",
                self.name(),
                cost.work
            ));
        }
        reserve(cost)?;
        let value = if self == Self::Texture {
            raster::execute(args)
        } else if self == Self::Draw {
            draw::execute(args, cost)
        } else {
            edit::execute(self, args)
        }?;
        if let (Some(key), Value::Texture(texture)) = (key, &value)
            && !args
                .values()
                .any(|value| matches!(value,Value::Texture(input)if Arc::ptr_eq(input,texture)))
        {
            return Ok(Value::Texture(cache::publish(key, texture.clone())));
        }
        Ok(value)
    }
    fn cached(self) -> bool {
        !matches!(self, Self::Size | Self::Pixel)
    }
}
fn choices(values: impl IntoIterator<Item = &'static str>) -> Type {
    Type::union(
        values
            .into_iter()
            .map(|value| Type::StringLiteral(value.into())),
    )
    .expect("texture choices")
}
fn pair_type() -> Type {
    Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(2))
}
fn required<'a>(args: &'a Arguments, name: &str) -> Result<&'a Value, String> {
    args.get(name)
        .ok_or_else(|| format!("texture missing '{name}'"))
}
fn number(args: &Arguments, name: &str, default: f64) -> Result<f64, String> {
    args.get(name)
        .map(|value| math::to_f64(value.number()?.value))
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn text<'a>(args: &'a Arguments, name: &str, default: &'a str) -> Result<&'a str, String> {
    args.get(name)
        .map(Value::string)
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn boolean(args: &Arguments, name: &str, default: bool) -> Result<bool, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        None => Ok(default),
        _ => Err(format!("texture {name} requires Bool")),
    }
}
fn pair(value: &Value) -> Result<[u32; 2], String> {
    let values = value.list()?;
    if values.len() != 2 {
        return Err("texture coordinates require two integer values [x,y]".into());
    }
    Ok([
        u32::try_from(values[0].number()?.usize()?)
            .map_err(|_| "texture coordinate exceeds u32")?,
        u32::try_from(values[1].number()?.usize()?)
            .map_err(|_| "texture coordinate exceeds u32")?,
    ])
}
fn texture<'a>(args: &'a Arguments, name: &str) -> Result<&'a Arc<sanim_voxel::Texture>, String> {
    if let Value::Texture(texture) = required(args, name)? {
        Ok(texture)
    } else {
        Err(format!("{name} requires Texture"))
    }
}
fn publish(size: [u32; 2], pixels: Vec<u8>) -> Result<Value, String> {
    sanim_voxel::Texture::new(size[0], size[1], pixels.into())
        .map(|texture| Value::Texture(Arc::new(texture)))
}
fn color(args: &Arguments, name: &str) -> Result<[u8; 4], String> {
    match args.get(name) {
        Some(Value::Color(color)) => color.rgba8(),
        None => Ok([0; 4]),
        _ => Err(format!("texture {name} requires Color")),
    }
}
fn checked_product(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b)
        .ok_or_else(|| "texture resource budget overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_folding_rejects_host_byte_limits_before_cold_or_cached_pixels() {
        use crate::reactive::{Node, ReactiveGraph};
        let size = Value::List(crate::List::typed(
            Type::Numeric(Unit::Number),
            [127u32, 129]
                .map(|value| {
                    Value::Number(crate::Number {
                        value: value.into(),
                        unit: Unit::Number,
                    })
                })
                .into(),
        ));
        for warm in [false, true] {
            if warm {
                Op::Texture
                    .execute(&[("size".to_string(), size.clone())].into_iter().collect())
                    .unwrap();
            }
            let mut graph = ReactiveGraph::default();
            graph.set_native_byte_limit(32_000).unwrap();
            let size = graph
                .insert_node(Node::Constant(size.clone()), 100)
                .unwrap();
            let error = graph
                .insert_node(
                    Node::NativeConfig {
                        operation: crate::code::values::Native::Texture(Op::Texture),
                        arguments: [("size".to_string(), size)].into_iter().collect(),
                    },
                    100,
                )
                .unwrap_err();
            assert!(error.contains("host byte budget"), "{error}");
        }
    }
    #[test]
    fn lowered_host_limits_reject_restored_texture_cells_and_future_writes() {
        use crate::reactive::ReactiveGraph;
        let large = Value::Texture(Arc::new(
            sanim_voxel::Texture::new(128, 128, vec![0; 128 * 128 * 4].into()).unwrap(),
        ));
        let small = Value::Texture(Arc::new(
            sanim_voxel::Texture::new(1, 1, vec![0; 4].into()).unwrap(),
        ));
        for initial_large in [true, false] {
            let mut graph = ReactiveGraph::default();
            graph
                .set_cell(
                    1,
                    if initial_large {
                        large.clone()
                    } else {
                        small.clone()
                    },
                )
                .unwrap();
            if !initial_large {
                graph.write_cell(1, 1.into(), large.clone()).unwrap();
            }
            assert!(
                graph
                    .set_native_byte_limit(32_000)
                    .unwrap_err()
                    .contains("host byte budget")
            );
            let mut encoded = serde_json::to_value(&graph).unwrap();
            encoded["native_byte_limit"] = 32_000.into();
            assert!(
                serde_json::from_value::<ReactiveGraph>(encoded)
                    .unwrap_err()
                    .to_string()
                    .contains("host byte budget")
            );
        }
    }
    #[test]
    fn cached_pixels_remain_pinned_when_reservation_evicts_the_cache() {
        let args = [(
            "size".to_string(),
            Value::List(crate::List::typed(
                Type::Numeric(Unit::Number),
                [13u32, 17]
                    .into_iter()
                    .map(|v| {
                        Value::Number(crate::Number {
                            value: crate::Fraction::from(v),
                            unit: Unit::Number,
                        })
                    })
                    .collect(),
            )),
        )]
        .into_iter()
        .collect();
        let Value::Texture(original) = Op::Texture.execute(&args).unwrap() else {
            panic!("Texture")
        };
        let before = cache_stats().rasterizations;
        let Value::Texture(result) = Op::Texture
            .evaluate(&args, |cost| {
                assert_eq!(cost.work, 0);
                assert_eq!(cost.bytes, 0);
                assert_eq!(cost.output_bytes, 13 * 17 * 4);
                // This is the worst interleaving of a concurrent eviction: the Arc
                // has been chosen but the caller has not completed its reservation.
                cache::clear();
                Ok(())
            })
            .unwrap()
        else {
            panic!("Texture")
        };
        assert!(Arc::ptr_eq(&original, &result));
        assert_eq!(cache_stats().rasterizations, before);
    }
}
