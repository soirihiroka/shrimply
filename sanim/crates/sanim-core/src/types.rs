use crate::{Element, Fraction, List, Number, Unit};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    /// Internal inference hole, never a user-declarable type or a dynamic top type.
    Unknown,
    Unit,
    Bool,
    String,
    Template,
    Signal(Box<Type>),
    Sql,
    SqlIdent,
    Numeric(Unit),
    Fraction,
    Vector(usize, Option<Unit>),
    Matrix4,
    Color,
    Element,
    Predicate,
    Font,
    Data,
    Dict,
    DataFrame {
        row: Box<Type>,
        groups: Vec<String>,
    },
    Series(Box<Type>),
    List(Box<Type>),
    Union(Vec<Type>),
    Option(Box<Type>),
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
        if let Type::Union(alternatives) = ty {
            return !alternatives.is_empty() && alternatives.iter().all(|ty| self.accepts(ty));
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind {
    #[default]
    Let,
    Mut,
    Sig,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionParameter {
    pub name: String,
    pub ty: Type,
    pub kind: BindingKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalRef {
    pub id: u64,
    pub ty: Arc<Type>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub duration: crate::TimelineEffect,
    pub schedules: bool,
    #[serde(default)]
    pub mutates: bool,
    pub duration_parameters: std::collections::BTreeSet<usize>,
    pub generics: Vec<GenericParameter<Type, TypeBound>>,
    pub default_count: usize,
    pub parameters: Vec<FunctionParameter>,
    pub result: Box<Type>,
}

impl Type {
    /// Flatten and order alternatives once so equivalent unions share one schema.
    pub fn union(types: impl IntoIterator<Item = Type>) -> Result<Self, String> {
        fn append(ty: Type, output: &mut Vec<Type>) {
            if let Type::Union(alternatives) = ty {
                for alternative in alternatives {
                    append(alternative, output);
                }
            } else {
                output.push(ty);
            }
        }
        let mut alternatives = vec![];
        for ty in types {
            append(ty, &mut alternatives);
        }
        alternatives
            .sort_by_cached_key(|ty| serde_json::to_vec(ty).expect("serializable semantic type"));
        alternatives.dedup();
        match alternatives.len() {
            0 => Err("a union type requires at least one alternative".into()),
            1 => Ok(alternatives
                .pop()
                .expect("one normalized union alternative")),
            _ => Ok(Self::Union(alternatives)),
        }
    }
    pub fn unresolved(&self) -> bool {
        self.incomplete(true)
    }
    /// Runtime schemas must resolve generic parameters as well as inferred units.
    pub fn is_concrete(&self) -> bool {
        !self.incomplete(false)
    }
    fn incomplete(&self, symbolic_parameters: bool) -> bool {
        match self {
            Self::Unknown | Self::Vector(_, None) => true,
            Self::Variable(_) | Self::BoundedVariable(..) => !symbolic_parameters,
            Self::List(t)
            | Self::Option(t)
            | Self::Array(t)
            | Self::Series(t)
            | Self::Signal(t) => t.incomplete(symbolic_parameters),
            Self::DataFrame { row, .. } => row.incomplete(symbolic_parameters),
            Self::Record(fields) => fields.values().any(|ty| ty.incomplete(symbolic_parameters)),
            Self::Union(alternatives) => alternatives
                .iter()
                .any(|ty| ty.incomplete(symbolic_parameters)),
            Self::Enum { variants, .. } => variants
                .values()
                .flatten()
                .any(|ty| ty.incomplete(symbolic_parameters)),
            _ => false,
        }
    }
    pub fn contains_function(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Function(_)))
    }
    pub fn contains_signal(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Signal(_)))
    }
    pub fn contains_element(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Element))
    }
    fn contains(&self, predicate: &impl Fn(&Type) -> bool) -> bool {
        if predicate(self) {
            return true;
        }
        match self {
            Self::List(t)
            | Self::Option(t)
            | Self::Array(t)
            | Self::Series(t)
            | Self::Signal(t) => t.contains(predicate),
            Self::DataFrame { row, .. } => row.contains(predicate),
            Self::Record(fields) => fields.values().any(|ty| ty.contains(predicate)),
            Self::Union(alternatives) => alternatives.iter().any(|ty| ty.contains(predicate)),
            Self::Enum { variants, .. } => {
                variants.values().flatten().any(|ty| ty.contains(predicate))
            }
            Self::Function(signature) => {
                signature.result.contains(predicate)
                    || signature
                        .parameters
                        .iter()
                        .any(|parameter| parameter.ty.contains(predicate))
            }
            Self::BoundedVariable(_, bound) => {
                matches!(bound.as_ref(), TypeBound::Type(ty) if ty.contains(predicate))
            }
            _ => false,
        }
    }
    pub fn accepts(&self, actual: &Type) -> bool {
        if let Self::Union(alternatives) = actual {
            return !alternatives.is_empty() && alternatives.iter().all(|ty| self.accepts(ty));
        }
        if let Self::Union(alternatives) = self {
            return alternatives.iter().any(|ty| ty.accepts(actual));
        }
        self == actual
            || matches!(actual,Self::BoundedVariable(_,bound) if matches!(bound.as_ref(),TypeBound::Type(ty) if self.accepts(ty)))
            || matches!(self, Type::Unknown)
            || match (self, actual) {
                (Self::Vector(n, None), Self::Vector(m, _)) => n == m,
                (Self::Signal(a), Self::Signal(b)) => a.accepts(b),
                (Self::List(a), Self::List(b))
                | (Self::Option(a), Self::Option(b))
                | (Self::Array(a), Self::Array(b))
                | (Self::Series(a), Self::Series(b)) => **b == Self::Unknown || a.accepts(b),
                (
                    Self::DataFrame { row: a, groups: ga },
                    Self::DataFrame { row: b, groups: gb },
                ) => ga == gb && a.accepts(b),
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
            Self::List(t) | Self::Array(t) | Self::Option(t) => t.readable(),
            Self::Record(fields) => fields.values().all(Self::readable),
            Self::Union(alternatives) => {
                !alternatives.is_empty() && alternatives.iter().all(Self::readable)
            }
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
            Self::List(t) | Self::Array(t) | Self::Option(t) => t.data_convertible(),
            Self::Record(fields) => fields.values().all(Self::data_convertible),
            Self::Enum { variants, .. } => variants.values().flatten().all(Self::data_convertible),
            Self::Union(alternatives) => {
                !alternatives.is_empty() && alternatives.iter().all(Self::data_convertible)
            }
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
            Self::List(t) | Self::Array(t) | Self::Option(t) => t.reflectable(),
            Self::Record(fields) => fields.values().all(Self::reflectable),
            Self::Union(alternatives) => {
                !alternatives.is_empty() && alternatives.iter().all(Self::reflectable)
            }
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
            Self::Array(t) => Self::Array(Box::new(t.substitute(bindings))),
            Self::Signal(t) => Self::Signal(Box::new(t.substitute(bindings))),
            Self::Series(t) => Self::Series(Box::new(t.substitute(bindings))),
            Self::DataFrame { row, groups } => Self::DataFrame {
                row: Box::new(row.substitute(bindings)),
                groups: groups.clone(),
            },
            Self::Union(alternatives) => {
                Self::union(alternatives.iter().map(|ty| ty.substitute(bindings)))
                    .expect("nonempty checked union")
            }
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
            Self::Template => write!(f, "Template"),
            Self::Signal(t) => write!(f, "sig {t}"),
            Self::Sql => write!(f, "Sql"),
            Self::SqlIdent => write!(f, "SqlIdent"),
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
            Self::Predicate => write!(f, "visual relationship"),
            Self::Font => write!(f, "Font"),
            Self::Data => write!(f, "Data"),
            Self::Dict => write!(f, "Dict"),
            Self::DataFrame { row, groups } => {
                write!(f, "DataFrame<{row}>")?;
                if !groups.is_empty() {
                    write!(f, " grouped by {}", groups.join(", "))?;
                }
                Ok(())
            }
            Self::Series(t) => write!(f, "Series<{t}>"),
            Self::List(t) => write!(f, "List<{t}>"),
            Self::Union(alternatives) => {
                for (index, ty) in alternatives.iter().enumerate() {
                    if index > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{ty}")?;
                }
                Ok(())
            }
            Self::Option(t) => write!(f, "Option<{t}>"),
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
                for (i, parameter) in sig.parameters.iter().enumerate() {
                    let name = &parameter.name;
                    let ty = &parameter.ty;
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if parameter.kind != BindingKind::Let {
                        write!(
                            f,
                            "{} ",
                            if parameter.kind == BindingKind::Sig {
                                "sig"
                            } else {
                                "mut"
                            }
                        )?;
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
    List(List),
    Array {
        element_type: Arc<Type>,
        shape: Vec<usize>,
        values: imbl::Vector<Value>,
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
    Predicate(Arc<crate::verify::Predicate>),
    Function(String),
    Font(crate::Font),
    Data(Arc<serde_json::Value>),
    Dict(imbl::OrdMap<String, serde_json::Value>),
    Template(Arc<crate::Template>),
    Signal(SignalRef),
    Sql(Arc<crate::sql::Sql>),
    SqlIdent(Arc<str>),
    DataFrame(Arc<crate::dataframe::Frame>),
    Series(Arc<crate::dataframe::Series>),
}

impl Value {
    pub fn ty(&self) -> Type {
        match self {
            Self::Unit => Type::Unit,
            Self::Bool(_) => Type::Bool,
            Self::String(_) => Type::String,
            Self::Template(_) => Type::Template,
            Self::Signal(signal) => Type::Signal(Box::new(signal.ty.as_ref().clone())),
            Self::Sql(_) => Type::Sql,
            Self::SqlIdent(_) => Type::SqlIdent,
            Self::Number(n) => Type::Numeric(n.unit),
            Self::Fraction(_) => Type::Fraction,
            Self::Vec2(v) => Type::Vector(2, Some(v.unit)),
            Self::Vec3(v) => Type::Vector(3, Some(v.unit)),
            Self::Vec4(v) => Type::Vector(4, Some(v.unit)),
            Self::Matrix4(_) => Type::Matrix4,
            Self::Color(_) => Type::Color,
            Self::List(v) => Type::List(Box::new(v.element_type.as_ref().clone())),
            Self::Array { element_type, .. } => {
                Type::Array(Box::new(element_type.as_ref().clone()))
            }
            Self::Record(v) => Type::Record(v.iter().map(|(k, v)| (k.clone(), v.ty())).collect()),
            Self::Some(v) => Type::Option(Box::new(v.ty())),
            Self::None => Type::Option(Box::new(Type::Unknown)),
            Self::Variant { ty, .. } => ty.as_ref().clone(),
            Self::Element(_) => Type::Element,
            Self::Predicate(_) => Type::Predicate,
            Self::Function(_) => Type::Unknown,
            Self::Font(_) => Type::Font,
            Self::Data(_) => Type::Data,
            Self::Dict(_) => Type::Dict,
            Self::DataFrame(frame) => frame.ty(),
            Self::Series(series) => Type::Series(Box::new(series.ty.as_ref().clone())),
        }
    }
    /// Refine contextual collection schemas without changing shared source payloads.
    /// This does not introduce implicit scalar conversions.
    pub fn with_type(&self, expected: &Type) -> Result<Self, String> {
        match (expected, self) {
            (Type::List(inner), Self::List(list))
                if inner.as_ref() == list.element_type.as_ref() =>
            {
                return Ok(self.clone());
            }
            (Type::Array(inner), Self::Array { element_type, .. })
                if inner.as_ref() == element_type.as_ref() =>
            {
                return Ok(self.clone());
            }
            _ => {}
        }
        let actual = self.ty();
        if !expected.accepts(&actual) {
            return Err(format!("expected {expected}, found {actual}"));
        }
        if let Type::Union(alternatives) = expected {
            let alternative = alternatives
                .iter()
                .find(|ty| ty.accepts(&actual))
                .expect("accepted union alternative");
            return self.with_type(alternative);
        }
        if expected == &actual || expected.unresolved() {
            return Ok(self.clone());
        }
        Ok(match (expected, self) {
            (Type::List(inner), Self::List(list)) => Self::List(List::from_values(
                Arc::new(inner.as_ref().clone()),
                if list.is_empty() {
                    list.values.clone()
                } else {
                    list.values
                        .iter()
                        .map(|value| value.with_type(inner))
                        .collect::<Result<Vec<_>, _>>()?
                        .into()
                },
            )),
            (Type::Array(inner), Self::Array { shape, values, .. }) => Self::Array {
                element_type: Arc::new(inner.as_ref().clone()),
                shape: shape.clone(),
                values: values
                    .iter()
                    .map(|value| value.with_type(inner))
                    .collect::<Result<_, _>>()?,
            },
            (Type::Record(fields), Self::Record(values)) => Self::Record(
                fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), values[name].with_type(ty)?)))
                    .collect::<Result<_, String>>()?,
            ),
            (Type::Option(inner), Self::Some(value)) => {
                Self::Some(Arc::new(value.with_type(inner)?))
            }
            _ => self.clone(),
        })
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
    pub fn list(&self) -> Result<&imbl::Vector<Value>, String> {
        match self {
            Self::List(v) => Ok(&v.values),
            Self::Array { values: v, .. } => Ok(v),
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
    #[serde(flatten)]
    pub declaration: Arc<crate::reflection::Declaration>,
    pub default: Value,
    pub value: Value,
}
impl std::ops::Deref for ReflectedInput {
    type Target = crate::reflection::Declaration;
    fn deref(&self) -> &Self::Target {
        &self.declaration
    }
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
