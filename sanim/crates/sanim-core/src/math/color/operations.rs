//! One native color registry/evaluator for preparation and retained playback.
use super::*;
use crate::{Color, Fraction, Number, Type, Unit, Value, math};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Construct(Space),
    Convert(Space),
    Mix,
    Alpha,
    Lightness,
    Chroma,
    Hue,
    Rotate,
    Luminance,
    Contrast,
    Over,
    Distance,
}
const METHODS: [(Op, &str); 10] = [
    (Op::Mix, "Color.mix"),
    (Op::Alpha, "Color.with_alpha"),
    (Op::Lightness, "Color.with_lightness"),
    (Op::Chroma, "Color.with_chroma"),
    (Op::Hue, "Color.with_hue"),
    (Op::Rotate, "Color.rotate_hue"),
    (Op::Luminance, "Color.luminance"),
    (Op::Contrast, "Color.contrast"),
    (Op::Over, "Color.over"),
    (Op::Distance, "Color.distance"),
];
pub(super) fn number() -> Type {
    Type::Numeric(Unit::Number)
}
fn literals(values: impl IntoIterator<Item = &'static str>) -> Type {
    Type::union(
        values
            .into_iter()
            .map(|value| Type::StringLiteral(value.into())),
    )
    .expect("color option values")
}
pub(super) fn gamut_parameter() -> (&'static str, Type, bool) {
    ("gamut", literals(Gamut::ALL.map(Gamut::name)), false)
}
impl Op {
    pub fn all() -> impl Iterator<Item = Self> {
        Space::ALL
            .into_iter()
            .flat_map(|space| [Self::Construct(space), Self::Convert(space)])
            .chain(METHODS.map(|(operation, _)| operation))
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().find(|operation| operation.name() == name)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Construct(space) => space.definition().constructor,
            Self::Convert(space) => space.definition().method,
            _ => {
                METHODS
                    .iter()
                    .find(|(operation, _)| *operation == self)
                    .expect("native color operation")
                    .1
            }
        }
    }
    pub fn is_method(self) -> bool {
        !matches!(self, Self::Construct(_))
    }
    pub fn signature(self) -> (Vec<(&'static str, Type, bool)>, Type) {
        if let Self::Construct(space) | Self::Convert(space) = self {
            return space.signature(matches!(self, Self::Construct(_)));
        }
        let mut parameters = vec![("color", Type::Color, true)];
        let mut result = Type::Color;
        match self {
            Self::Mix => parameters.extend([
                ("other", Type::Color, true),
                ("amount", number(), true),
                ("space", literals(MixSpace::ALL.map(MixSpace::name)), false),
                (
                    "hue",
                    literals(HueInterpolation::ALL.map(HueInterpolation::name)),
                    false,
                ),
                gamut_parameter(),
            ]),
            Self::Alpha => parameters.push(("alpha", number(), true)),
            Self::Lightness => {
                parameters.extend([("lightness", number(), true), gamut_parameter()])
            }
            Self::Chroma => parameters.extend([("chroma", number(), true), gamut_parameter()]),
            Self::Hue => {
                parameters.extend([("hue", Type::Numeric(Unit::Angle), true), gamut_parameter()])
            }
            Self::Rotate => parameters.extend([
                ("angle", Type::Numeric(Unit::Angle), true),
                gamut_parameter(),
            ]),
            Self::Luminance => result = number(),
            Self::Contrast | Self::Distance => {
                parameters.push(("other", Type::Color, true));
                result = number();
            }
            Self::Over => parameters.push(("background", Type::Color, true)),
            _ => unreachable!(),
        }
        (parameters, result)
    }
    pub fn work(self) -> usize {
        if matches!(self, Self::Convert(_)) {
            3
        } else {
            Ramp::MAX_SAMPLE_WORK
        }
    }
    pub fn execute(self, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
        if let Self::Construct(space) = self {
            return space.construct(args);
        }
        if let Self::Convert(space) = self {
            return space.convert(args);
        }
        let color = color(args, "color")?;
        let rgba = color.srgba()?;
        match self {
            Self::Mix => {
                let other = self::color(args, "other")?;
                let amount = required(args, "amount")?.number()?.value;
                if !amount.is_finite() || !(Fraction::from(0)..=Fraction::from(1)).contains(&amount)
                {
                    return Err("Color.mix amount must be finite within 0..=1".into());
                }
                if amount == Fraction::from(0) {
                    return Ok(Value::Color(color.clone()));
                }
                if amount == Fraction::from(1) {
                    return Ok(Value::Color(other.clone()));
                }
                let options = MixOptions {
                    space: args
                        .get("space")
                        .map(Value::string)
                        .transpose()?
                        .unwrap_or("oklab")
                        .parse()?,
                    hue: args
                        .get("hue")
                        .map(Value::string)
                        .transpose()?
                        .unwrap_or("shorter")
                        .parse()?,
                    gamut: gamut(args)?,
                };
                let mut mixed = from_rgba(mix_rgba(
                    rgba,
                    other.srgba()?,
                    math::to_f64(amount)?,
                    options,
                )?)?;
                if color.0[3] == other.0[3] {
                    mixed.0[3] = color.0[3];
                }
                Ok(Value::Color(mixed))
            }
            Self::Alpha => {
                bounded(args, "alpha", None)?;
                let mut color = color.clone();
                color.0[3] = required(args, "alpha")?.number()?.value;
                Ok(Value::Color(color))
            }
            Self::Lightness | Self::Chroma | Self::Hue | Self::Rotate => {
                let mut lch = oklab_to_oklch(rgba_to_oklab(rgba));
                match self {
                    Self::Lightness => lch[0] = bounded(args, "lightness", None)?,
                    Self::Chroma => {
                        lch[1] = scalar(args, "chroma", None)?;
                        if lch[1] < 0.0 {
                            return Err("chroma must be nonnegative".into());
                        }
                    }
                    Self::Hue => lch[2] = angle(args, "hue")?,
                    Self::Rotate => lch[2] = (lch[2] + angle(args, "angle")?).rem_euclid(360.0),
                    _ => unreachable!(),
                }
                let mut adjusted =
                    from_rgba(map_oklab(oklch_to_oklab(lch), rgba[3], gamut(args)?)?)?;
                adjusted.0[3] = color.0[3];
                Ok(Value::Color(adjusted))
            }
            Self::Luminance => value(relative_luminance(rgba)?, Unit::Number),
            Self::Contrast => {
                let other = self::color(args, "other")?;
                if color.0[3] != Fraction::from(1) || other.0[3] != Fraction::from(1) {
                    return Err(
                        "contrast requires opaque colors; use .over(background) first".into(),
                    );
                }
                value(contrast(rgba, other.srgba()?)?, Unit::Number)
            }
            Self::Over => {
                let background = self::color(args, "background")?;
                let background_rgba = background.srgba()?;
                if color.0[3] == Fraction::from(0) && background.0[3] == Fraction::from(0) {
                    return Ok(Value::Color(Color([Fraction::from(0); 4])));
                }
                if color.0[3] == Fraction::from(1) || background.0[3] == Fraction::from(0) {
                    return Ok(Value::Color(color.clone()));
                }
                if color.0[3] == Fraction::from(0) {
                    return Ok(Value::Color(background.clone()));
                }
                from_rgba(over(rgba, background_rgba)?).map(Value::Color)
            }
            Self::Distance => {
                let a = rgba_to_oklab(rgba);
                let b = rgba_to_oklab(self::color(args, "other")?.srgba()?);
                value((a.l - b.l).hypot(a.a - b.a).hypot(a.b - b.b), Unit::Number)
            }
            _ => unreachable!(),
        }
    }
}
pub(super) fn required<'a>(
    args: &'a imbl::OrdMap<String, Value>,
    name: &str,
) -> Result<&'a Value, String> {
    args.get(name)
        .ok_or_else(|| format!("missing color argument '{name}'"))
}
pub(super) fn scalar(
    args: &imbl::OrdMap<String, Value>,
    name: &str,
    default: Option<f64>,
) -> Result<f32, String> {
    let value = match args.get(name) {
        Some(value) => {
            let number = value.number()?;
            if number.unit != Unit::Number {
                return Err(format!(
                    "color argument '{name}' requires a unitless Number"
                ));
            }
            math::to_f64(number.value)?
        }
        None => default.ok_or_else(|| format!("missing color argument '{name}'"))?,
    };
    let value = value as f32;
    if !value.is_finite() {
        return Err(format!(
            "color argument '{name}' must be finite and representable"
        ));
    }
    Ok(value)
}
pub(super) fn bounded(
    args: &imbl::OrdMap<String, Value>,
    name: &str,
    default: Option<f64>,
) -> Result<f32, String> {
    if let Some(value) = args.get(name) {
        let value = value.number()?.value;
        if !value.is_finite() || !(Fraction::from(0)..=Fraction::from(1)).contains(&value) {
            return Err(format!(
                "color argument '{name}' must be finite within 0..=1"
            ));
        }
    }
    let value = scalar(args, name, default)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("color argument '{name}' must be within 0..=1"));
    }
    Ok(value)
}
pub(super) fn angle(args: &imbl::OrdMap<String, Value>, name: &str) -> Result<f32, String> {
    let value = required(args, name)?.number()?;
    if value.unit != Unit::Angle {
        return Err(format!(
            "color argument '{name}' requires an angle, such as 45deg"
        ));
    }
    // Dividing a tiny rational by a full turn can overflow its denominator even
    // though it already is its own remainder.
    let wrapped = if value.value > Fraction::from(-360) && value.value < Fraction::from(360) {
        value.value
    } else {
        value
            .binary(
                "%",
                Number {
                    value: Fraction::from(360),
                    unit: Unit::Angle,
                },
            )?
            .value
    };
    Ok(math::to_f64(wrapped)?.rem_euclid(360.0) as f32)
}
pub(super) fn color<'a>(
    args: &'a imbl::OrdMap<String, Value>,
    name: &str,
) -> Result<&'a Color, String> {
    if let Value::Color(color) = required(args, name)? {
        Ok(color)
    } else {
        Err(format!("color argument '{name}' requires Color"))
    }
}
pub(super) fn gamut(args: &imbl::OrdMap<String, Value>) -> Result<Gamut, String> {
    args.get("gamut")
        .map(Value::string)
        .transpose()?
        .unwrap_or("chroma")
        .parse()
}
pub(super) fn value(value: f32, unit: Unit) -> Result<Value, String> {
    Ok(Value::Number(Number {
        value: math::functions::approximate(f64::from(value))?,
        unit,
    }))
}
