use super::{operations::*, *};
use crate::{Type, Unit, Value};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Space {
    Rgb,
    Hsl,
    Hsv,
    Oklab,
    Oklch,
}
pub(super) struct Definition {
    pub space: Space,
    pub constructor: &'static str,
    pub method: &'static str,
    pub channels: [(&'static str, Unit); 3],
}
const DEFINITIONS: [Definition; 5] = [
    Definition {
        space: Space::Rgb,
        constructor: "Rgb",
        method: "Color.to_rgb",
        channels: [
            ("red", Unit::Number),
            ("green", Unit::Number),
            ("blue", Unit::Number),
        ],
    },
    Definition {
        space: Space::Hsl,
        constructor: "Hsl",
        method: "Color.to_hsl",
        channels: [
            ("hue", Unit::Angle),
            ("saturation", Unit::Number),
            ("lightness", Unit::Number),
        ],
    },
    Definition {
        space: Space::Hsv,
        constructor: "Hsv",
        method: "Color.to_hsv",
        channels: [
            ("hue", Unit::Angle),
            ("saturation", Unit::Number),
            ("value", Unit::Number),
        ],
    },
    Definition {
        space: Space::Oklab,
        constructor: "Oklab",
        method: "Color.to_oklab",
        channels: [
            ("lightness", Unit::Number),
            ("a", Unit::Number),
            ("b", Unit::Number),
        ],
    },
    Definition {
        space: Space::Oklch,
        constructor: "Oklch",
        method: "Color.to_oklch",
        channels: [
            ("lightness", Unit::Number),
            ("chroma", Unit::Number),
            ("hue", Unit::Angle),
        ],
    },
];
impl Space {
    pub const ALL: [Self; 5] = [Self::Rgb, Self::Hsl, Self::Hsv, Self::Oklab, Self::Oklch];
    pub(super) fn definition(self) -> &'static Definition {
        DEFINITIONS
            .iter()
            .find(|definition| definition.space == self)
            .expect("color space")
    }
    pub(super) fn signature(self, constructor: bool) -> (Vec<(&'static str, Type, bool)>, Type) {
        let definition = self.definition();
        if constructor {
            let mut parameters = definition
                .channels
                .into_iter()
                .map(|(name, unit)| (name, Type::Numeric(unit), true))
                .collect::<Vec<_>>();
            parameters.push(("alpha", number(), false));
            if matches!(self, Self::Oklab | Self::Oklch) {
                parameters.push(gamut_parameter());
            }
            (parameters, Type::Color)
        } else {
            let fields = definition
                .channels
                .iter()
                .copied()
                .chain(std::iter::once(("alpha", Unit::Number)))
                .map(|(name, unit)| (name.to_string(), Type::Numeric(unit)))
                .collect();
            (vec![("color", Type::Color, true)], Type::Record(fields))
        }
    }
    pub(super) fn construct(self, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
        let mut channels = [0.0; 4];
        for (out, (name, unit)) in channels.iter_mut().zip(self.definition().channels) {
            *out = if unit == Unit::Angle {
                angle(args, name)?
            } else if matches!(self, Self::Oklab) && matches!(name, "a" | "b")
                || matches!(self, Self::Oklch) && name == "chroma"
            {
                scalar(args, name, None)?
            } else {
                bounded(args, name, None)?
            };
        }
        channels[3] = bounded(args, "alpha", Some(1.0))?;
        let alpha = args
            .get("alpha")
            .map(|value| value.number().map(|number| number.value))
            .transpose()?
            .unwrap_or(crate::Fraction::from(1));
        if self == Self::Rgb {
            let mut result = crate::Color([crate::Fraction::from(1); 4]);
            for (out, (name, _)) in result.0.iter_mut().zip(self.definition().channels) {
                *out = required(args, name)?.number()?.value;
            }
            result.0[3] = alpha;
            return Ok(Value::Color(result));
        }
        let rgba = match self {
            Self::Rgb => unreachable!("exact RGB returned above"),
            Self::Hsl => hsl_to_rgba(channels),
            Self::Hsv => hsv_to_rgba(channels),
            Self::Oklab => map_oklab(
                shrimply_math_color::Oklab {
                    l: channels[0],
                    a: channels[1],
                    b: channels[2],
                },
                channels[3],
                gamut(args)?,
            )?,
            Self::Oklch => {
                if channels[1] < 0.0 {
                    return Err("Oklch chroma must be nonnegative".into());
                }
                map_oklab(
                    oklch_to_oklab([channels[0], channels[1], channels[2]]),
                    channels[3],
                    gamut(args)?,
                )?
            }
        };
        let mut result = from_rgba(rgba)?;
        result.0[3] = alpha;
        Ok(Value::Color(result))
    }
    pub(super) fn convert(self, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
        let color = color(args, "color")?;
        let definition = self.definition();
        if self == Self::Rgb {
            let fields = definition
                .channels
                .iter()
                .map(|(name, _)| *name)
                .chain(std::iter::once("alpha"))
                .zip(color.0)
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        Value::Number(crate::Number {
                            value,
                            unit: Unit::Number,
                        }),
                    )
                })
                .collect();
            return Ok(Value::Record(fields));
        }
        let rgba = color.srgba()?;
        let channels = match self {
            Self::Rgb => unreachable!(),
            Self::Hsl => rgba_to_hsl(rgba),
            Self::Hsv => rgba_to_hsv(rgba),
            Self::Oklab => {
                let lab = rgba_to_oklab(rgba);
                [lab.l, lab.a, lab.b, rgba[3]]
            }
            Self::Oklch => {
                let lch = oklab_to_oklch(rgba_to_oklab(rgba));
                [lch[0], lch[1], lch[2], rgba[3]]
            }
        };
        let fields = definition
            .channels
            .iter()
            .copied()
            .chain(std::iter::once(("alpha", Unit::Number)))
            .zip(channels)
            .map(|((name, unit), channel)| {
                Ok((
                    name.to_string(),
                    if name == "alpha" {
                        Value::Number(crate::Number {
                            value: color.0[3],
                            unit: Unit::Number,
                        })
                    } else {
                        value(channel, unit)?
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Value::Record(fields))
    }
}
