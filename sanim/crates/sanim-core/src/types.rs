use crate::{Element, Fraction, Motion, Number, Unit};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    /// Internal inference hole, never a user-declarable type or a dynamic top type.
    Unknown,
    Unit,
    Bool,
    String,
    Numeric(Unit),
    Fraction,
    Vector(usize, Option<Unit>),
    Matrix4,
    Color,
    Element,
    Motion,
    Predicate,
    Font,
    Data,
    Dict,
    List(Box<Type>),
    Option(Box<Type>),
    Table(Box<Type>),
    Array(Box<Type>),
    Record(BTreeMap<String, Type>),
    Enum {
        name: String,
        variants: BTreeMap<String, Vec<Type>>,
    },
    Function(Signature),
    Variable(String),
    BoundedVariable(String, Box<TypeBound>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericParameter<T, B = T> {
    pub name: String,
    pub constraint: Option<B>,
    pub default: Option<T>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeBound {
    Numeric,
    Readable,
    Reflectable,
    Type(Box<Type>),
}
impl TypeBound {
    pub fn substitute(&self, bindings: &BTreeMap<String, Type>) -> Self {
        match self {
            Self::Type(ty) => Self::Type(Box::new(ty.substitute(bindings))),
            _ => self.clone(),
        }
    }
    pub fn accepts(&self, ty: &Type) -> bool {
        if let Type::BoundedVariable(_, bound) = ty {
            return self == bound.as_ref()
                || match (self, bound.as_ref()) {
                    (Self::Readable, Self::Reflectable | Self::Numeric) => true,
                    (Self::Reflectable, Self::Numeric) => true,
                    (target, Self::Type(ty)) => target.accepts(ty),
                    _ => false,
                };
        }
        match self {
            Self::Numeric => matches!(ty, Type::Numeric(Unit::Number) | Type::Fraction),
            Self::Readable => ty.readable(),
            Self::Reflectable => ty.reflectable(),
            Self::Type(bound) => match (bound.as_ref(), ty) {
                (Type::Record(required), Type::Record(actual)) => required
                    .iter()
                    .all(|(name, t)| actual.get(name).is_some_and(|v| t.accepts(v))),
                _ => bound.accepts(ty),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub duration: crate::TimelineEffect,
    pub schedules: bool,
    pub duration_parameters: std::collections::BTreeSet<usize>,
    pub generics: Vec<GenericParameter<Type, TypeBound>>,
    pub default_count: usize,
    pub parameters: Vec<(String, Type)>,
    pub result: Box<Type>,
}

impl Type {
    pub fn unresolved(&self) -> bool {
        match self {
            Self::Unknown | Self::Vector(_, None) => true,
            Self::List(t) | Self::Option(t) | Self::Table(t) | Self::Array(t) => t.unresolved(),
            Self::Record(fields) => fields.values().any(Self::unresolved),
            _ => false,
        }
    }
    pub fn contains_function(&self) -> bool {
        match self {
            Self::Function(_) => true,
            Self::List(t) | Self::Option(t) | Self::Table(t) | Self::Array(t) => {
                t.contains_function()
            }
            Self::Record(fields) => fields.values().any(Self::contains_function),
            _ => false,
        }
    }
    pub fn accepts(&self, actual: &Type) -> bool {
        self == actual
            || matches!(actual,Self::BoundedVariable(_,bound) if matches!(bound.as_ref(),TypeBound::Type(ty) if self.accepts(ty)))
            || matches!(self, Type::Unknown)
            || match (self, actual) {
                (Self::Vector(n, None), Self::Vector(m, _)) => n == m,
                (Self::List(a), Self::List(b))
                | (Self::Option(a), Self::Option(b))
                | (Self::Table(a), Self::Table(b))
                | (Self::Array(a), Self::Array(b)) => **b == Self::Unknown || a.accepts(b),
                (Self::Record(a), Self::Record(b)) => {
                    a.len() == b.len()
                        && a.iter()
                            .all(|(name, ty)| b.get(name).is_some_and(|other| ty.accepts(other)))
                }
                _ => false,
            }
    }
    pub fn readable(&self) -> bool {
        match self {
            Self::Bool
            | Self::String
            | Self::Numeric(_)
            | Self::Fraction
            | Self::Vector(..)
            | Self::Matrix4
            | Self::Color
            | Self::Font => true,
            Self::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Readable | TypeBound::Reflectable | TypeBound::Numeric => true,
                TypeBound::Type(ty) => ty.readable(),
            },
            Self::Data => true,
            Self::Dict => true,
            Self::Enum { variants, .. } => variants.values().flatten().all(Self::readable),
            Self::List(t) | Self::Table(t) | Self::Array(t) | Self::Option(t) => t.readable(),
            Self::Record(fields) => fields.values().all(Self::readable),
            _ => false,
        }
    }
    pub fn data_convertible(&self) -> bool {
        match self {
            Self::Font => false,
            Self::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Numeric | TypeBound::Reflectable => true,
                TypeBound::Type(ty) => ty.data_convertible(),
                TypeBound::Readable => false,
            },
            Self::List(t) | Self::Table(t) | Self::Array(t) | Self::Option(t) => {
                t.data_convertible()
            }
            Self::Record(fields) => fields.values().all(Self::data_convertible),
            Self::Enum { variants, .. } => variants.values().flatten().all(Self::data_convertible),
            _ => self.readable(),
        }
    }
    pub fn reflectable(&self) -> bool {
        match self {
            Self::Font => false,
            Self::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Reflectable | TypeBound::Numeric => true,
                TypeBound::Type(ty) => ty.reflectable(),
                _ => false,
            },
            Self::Enum { variants, .. } => variants.values().flatten().all(Self::reflectable),
            Self::List(t) | Self::Table(t) | Self::Array(t) | Self::Option(t) => t.reflectable(),
            Self::Record(fields) => fields.values().all(Self::reflectable),
            _ => self.readable(),
        }
    }
    pub fn substitute(&self, bindings: &BTreeMap<String, Type>) -> Type {
        match self {
            Self::Variable(name) => bindings.get(name).cloned().unwrap_or_else(|| self.clone()),
            Self::BoundedVariable(name, bound) => {
                bindings.get(name).cloned().unwrap_or_else(|| {
                    Self::BoundedVariable(name.clone(), Box::new(bound.substitute(bindings)))
                })
            }
            Self::List(t) => Self::List(Box::new(t.substitute(bindings))),
            Self::Option(t) => Self::Option(Box::new(t.substitute(bindings))),
            Self::Table(t) => Self::Table(Box::new(t.substitute(bindings))),
            Self::Array(t) => Self::Array(Box::new(t.substitute(bindings))),
            Self::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.substitute(bindings)))
                    .collect(),
            ),
            _ => self.clone(),
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "an inferred value"),
            Self::Unit => write!(f, "Unit"),
            Self::Bool => write!(f, "Bool"),
            Self::String => write!(f, "String"),
            Self::Numeric(unit) => write!(
                f,
                "{}",
                match unit {
                    Unit::Number => "Number",
                    Unit::Length => "Length (pt)",
                    Unit::Duration => "Duration (s or ms)",
                    Unit::Angle => "Angle (deg)",
                    Unit::Rate => "Rate (fps)",
                    Unit::Percent => "Percent (%)",
                }
            ),
            Self::Fraction => write!(f, "Fraction"),
            Self::Vector(n, None) => write!(f, "Vec{n}"),
            Self::Vector(n, Some(unit)) => write!(f, "Vec{n}<{}>", Self::Numeric(*unit)),
            Self::Matrix4 => write!(f, "Matrix4"),
            Self::Color => write!(f, "Color"),
            Self::Element => write!(f, "Element"),
            Self::Motion => write!(f, "Motion"),
            Self::Predicate => write!(f, "visual relationship"),
            Self::Font => write!(f, "Font"),
            Self::Data => write!(f, "Data"),
            Self::Dict => write!(f, "Dict"),
            Self::List(t) => write!(f, "List<{t}>"),
            Self::Option(t) => write!(f, "Option<{t}>"),
            Self::Table(t) => write!(f, "Table<{t}>"),
            Self::Array(t) => write!(f, "Array<{t}>"),
            Self::Record(fields) => {
                write!(f, "{{ ")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {ty}")?;
                }
                write!(f, " }}")
            }
            Self::Enum { name, .. } | Self::Variable(name) | Self::BoundedVariable(name, _) => {
                write!(f, "{name}")
            }
            Self::Function(sig) => {
                write!(f, "fn(")?;
                for (i, (name, ty)) in sig.parameters.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {ty}")?;
                }
                write!(f, "): {}", sig.result)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub [Fraction; 4]);

impl Color {
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some((_, color)) = shrimply_math_color::Color::ADWAITA
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(text))
        {
            // Shared palette RGB values originate as bytes. Recover those exact
            // values; preserve decimal alpha rather than quantizing it to a byte.
            let mut channels = shrimply_math_color::Color::<u8>::from_srgba(color.to_array())
                .to_array()
                .map(|channel| Fraction::new(u64::from(channel), 255u64));
            channels[3] = crate::math::decimal(&color.a.to_string())?;
            return Ok(Self(channels));
        }
        let hex = match text {
            "red" => "ff0000",
            "green" => "008000",
            "blue" => "0000ff",
            "white" => "ffffff",
            "black" => "000000",
            "yellow" => "ffff00",
            "orange" => "ffa500",
            "purple" => "800080",
            "transparent" => "00000000",
            _ => text
                .strip_prefix('#')
                .ok_or_else(|| format!("unknown color '{text}'; use an Adwaita name such as 'blue3' or 'window_bg_dark', a basic color name, or #RRGGBB/#RRGGBBAA"))?,
        };
        if !hex.is_ascii() || !matches!(hex.len(), 6 | 8) {
            return Err("expected #RRGGBB or #RRGGBBAA".into());
        }
        let mut channels = [Fraction::from(1); 4];
        for (i, channel) in channels.iter_mut().enumerate().take(hex.len() / 2) {
            *channel = Fraction::new(
                u64::from(
                    u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                        .map_err(|_| "invalid hex color")?,
                ),
                255u64,
            );
        }
        Ok(Self(channels))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "[f64; N]: Serialize",
    deserialize = "[f64; N]: Deserialize<'de>"
))]
pub struct Vector<const N: usize> {
    pub values: [f64; N],
    pub unit: Unit,
}
impl<const N: usize> Vector<N> {
    pub fn zero(unit: Unit) -> Self {
        Self {
            values: [0.0; N],
            unit,
        }
    }
}
pub type Vec2 = Vector<2>;
pub type Vec3 = Vector<3>;
pub type Vec4 = Vector<4>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Unit,
    Bool(bool),
    String(Arc<str>),
    Number(Number),
    Fraction(Fraction),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Matrix4(Box<crate::math::linear::Matrix4>),
    Color(Color),
    List(Arc<[Value]>),
    Table {
        row_type: Arc<Type>,
        rows: std::sync::Arc<Vec<Value>>,
    },
    Array {
        element_type: Arc<Type>,
        shape: Vec<usize>,
        values: std::sync::Arc<Vec<Value>>,
    },
    Record(imbl::OrdMap<String, Value>),
    Some(Arc<Value>),
    None,
    Variant {
        ty: Arc<Type>,
        tag: String,
        fields: Arc<[Value]>,
    },
    Element(Arc<Element>),
    Motion(Arc<Motion>),
    Predicate(Arc<crate::verify::Predicate>),
    Function(String),
    Font(std::sync::Arc<Vec<u8>>),
    Data(Arc<serde_json::Value>),
    Dict(imbl::OrdMap<String, serde_json::Value>),
}

impl Value {
    pub fn ty(&self) -> Type {
        match self {
            Self::Unit => Type::Unit,
            Self::Bool(_) => Type::Bool,
            Self::String(_) => Type::String,
            Self::Number(n) => Type::Numeric(n.unit),
            Self::Fraction(_) => Type::Fraction,
            Self::Vec2(v) => Type::Vector(2, Some(v.unit)),
            Self::Vec3(v) => Type::Vector(3, Some(v.unit)),
            Self::Vec4(v) => Type::Vector(4, Some(v.unit)),
            Self::Matrix4(_) => Type::Matrix4,
            Self::Color(_) => Type::Color,
            Self::List(v) => Type::List(Box::new(v.first().map_or(Type::Unknown, Self::ty))),
            Self::Table { row_type, .. } => Type::Table(Box::new(row_type.as_ref().clone())),
            Self::Array { element_type, .. } => {
                Type::Array(Box::new(element_type.as_ref().clone()))
            }
            Self::Record(v) => Type::Record(v.iter().map(|(k, v)| (k.clone(), v.ty())).collect()),
            Self::Some(v) => Type::Option(Box::new(v.ty())),
            Self::None => Type::Option(Box::new(Type::Unknown)),
            Self::Variant { ty, .. } => ty.as_ref().clone(),
            Self::Element(_) => Type::Element,
            Self::Motion(_) => Type::Motion,
            Self::Predicate(_) => Type::Predicate,
            Self::Function(_) => Type::Unknown,
            Self::Font(_) => Type::Font,
            Self::Data(_) => Type::Data,
            Self::Dict(_) => Type::Dict,
        }
    }
    pub fn number(&self) -> Result<Number, String> {
        match self {
            Self::Number(n) => Ok(*n),
            Self::Fraction(value) => Ok(Number {
                value: *value,
                unit: Unit::Number,
            }),
            _ => Err("expected a numeric value".into()),
        }
    }
    pub fn string(&self) -> Result<&str, String> {
        if let Self::String(v) = self {
            Ok(v)
        } else {
            Err("expected a string".into())
        }
    }
    pub fn boolean(&self) -> Result<bool, String> {
        if let Self::Bool(v) = self {
            Ok(*v)
        } else {
            Err("expected Bool".into())
        }
    }
    pub fn vector(&self) -> Result<(&[f64], Unit), String> {
        match self {
            Self::Vec2(v) => Ok((&v.values, v.unit)),
            Self::Vec3(v) => Ok((&v.values, v.unit)),
            Self::Vec4(v) => Ok((&v.values, v.unit)),
            _ => Err("expected a vector".into()),
        }
    }
    pub fn vec2(&self) -> Result<Vec2, String> {
        if let Self::Vec2(v) = self {
            Ok(*v)
        } else {
            Err("expected Vec2".into())
        }
    }
    pub fn from_vector(values: Vec<f64>, unit: Unit) -> Result<Self, String> {
        if values.iter().any(|v| !v.is_finite()) {
            return Err("vector components must be finite".into());
        }
        Ok(match values.len() {
            2 => Self::Vec2(Vec2 {
                values: values.try_into().map_err(|_| "invalid Vec2")?,
                unit,
            }),
            3 => Self::Vec3(Vec3 {
                values: values.try_into().map_err(|_| "invalid Vec3")?,
                unit,
            }),
            4 => Self::Vec4(Vec4 {
                values: values.try_into().map_err(|_| "invalid Vec4")?,
                unit,
            }),
            _ => return Err("vectors must have 2, 3 or 4 components".into()),
        })
    }
    pub fn list(&self) -> Result<&[Value], String> {
        match self {
            Self::List(v) => Ok(v),
            Self::Table { rows: v, .. } | Self::Array { values: v, .. } => Ok(v.as_ref()),
            _ => Err("expected a collection".into()),
        }
    }
    pub fn lookup(&self, key: &Value) -> Result<Value, String> {
        let value = match self {
            Self::Dict(entries) => entries.get(key.string()?).cloned().map(|value| Self::Data(value.into())),
            Self::Data(data) => match data.as_ref() {
                serde_json::Value::Object(entries) => entries.get(key.string()?).cloned().map(|value| Self::Data(value.into())),
                serde_json::Value::Array(items) => items.get(key.number()?.usize()?).cloned().map(|value| Self::Data(value.into())),
                _ => return Err("Data indexing requires an object or array; use decode<T> to validate its shape".into()),
            },
            _ => self.list()?.get(key.number()?.usize()?).cloned(),
        };
        Ok(value.map(|v| Self::Some(Arc::new(v))).unwrap_or(Self::None))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReflectedInput {
    pub key: String,
    pub name: String,
    pub ty: Arc<Type>,
    pub default: Value,
    pub value: Value,
}

/// Shared declaration drives native enum type information, values, and tag decoding.
pub trait NativeEnum: Copy + Sized + 'static {
    const NAME: &'static str;
    const VARIANTS: &'static [(&'static str, Self)];
    fn from_name(name: &str) -> Result<Self, String> {
        Self::VARIANTS
            .iter()
            .find(|(tag, _)| *tag == name)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("unknown {} case '{name}'", Self::NAME))
    }
    fn ty() -> Type {
        Type::Enum {
            name: Self::NAME.into(),
            variants: Self::VARIANTS
                .iter()
                .map(|(name, _)| ((*name).into(), vec![]))
                .collect(),
        }
    }
}
pub fn enum_namespace(ty: &Type) -> Value {
    let Type::Enum { variants, .. } = ty else {
        panic!("native enum namespace requires enum type")
    };
    Value::Record(
        variants
            .keys()
            .map(|tag| {
                (
                    tag.clone(),
                    Value::Variant {
                        ty: ty.clone().into(),
                        tag: tag.clone(),
                        fields: vec![].into(),
                    },
                )
            })
            .collect(),
    )
}
