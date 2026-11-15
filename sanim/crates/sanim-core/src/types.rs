use crate::{Element, Fraction, List, Number, Unit};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    /// Internal inference hole, never a user-declarable type or a dynamic top type.
    Unknown,
    Unit,
    Forever,
    Bool,
    String,
    StringLiteral(Arc<str>),
    Code(crate::code::Language),
    Template,
    Signal(Box<Type>),
    Sql,
    SqlIdent,
    Numeric(Unit),
    RefinedNumeric(crate::math::domain::NumericDomain),
    Fraction,
    Matrix4,
    Color,
    Element,
    Animation,
    Predicate,
    Font,
    Data,
    Dict,
    DataFrame {
        row: Box<Type>,
        groups: Vec<String>,
    },
    Series(Box<Type>),
    List(Box<Type>, ListLength),
    Union(Vec<Type>),
    Option(Box<Type>),
    Array(Box<Type>),
    Record(BTreeMap<String, Type>),
    Enum {
        name: String,
        variants: BTreeMap<String, Vec<Type>>,
    },
    Function(Signature),
    Overloaded(Vec<Signature>),
    Variable(String),
    BoundedVariable(String, Box<TypeBound>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListLength {
    #[default]
    Unknown,
    Exact(usize),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Widening {
    Literals,
    Editable,
    Signal,
    ListLengths,
}
impl ListLength {
    pub fn accepts(self, actual: Self) -> bool {
        self == Self::Unknown || self == actual
    }
    pub fn allows(self, length: usize) -> bool {
        self.accepts(Self::Exact(length))
    }
    pub fn checked_add(self, other: Self) -> Result<Self, String> {
        match (self, other) {
            (Self::Exact(a), Self::Exact(b)) => a
                .checked_add(b)
                .map(Self::Exact)
                .ok_or_else(|| "List length overflow while adding sizes".into()),
            _ => Ok(Self::Unknown),
        }
    }
    pub fn checked_mul(self, other: Self) -> Result<Self, String> {
        match (self, other) {
            (Self::Exact(0), _) | (_, Self::Exact(0)) => Ok(Self::Exact(0)),
            (Self::Exact(a), Self::Exact(b)) => a
                .checked_mul(b)
                .map(Self::Exact)
                .ok_or_else(|| "List length overflow while multiplying sizes".into()),
            _ => Ok(Self::Unknown),
        }
    }
    pub fn min(self, other: Self) -> Self {
        match (self, other) {
            (Self::Exact(0), _) | (_, Self::Exact(0)) => Self::Exact(0),
            (Self::Exact(a), Self::Exact(b)) => Self::Exact(a.min(b)),
            _ => Self::Unknown,
        }
    }
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
            Self::Numeric => ty.numeric_unit() == Some(Unit::Number) || *ty == Type::Fraction,
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

/// Explicit effects a function may observe. An empty set is a pure contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FnCapability {
    Mut,
    Sig,
    Ref,
}
impl FnCapability {
    pub const ALL: [Self; 3] = [Self::Mut, Self::Sig, Self::Ref];
    pub fn name(self) -> &'static str {
        match self {
            Self::Mut => "mut",
            Self::Sig => "sig",
            Self::Ref => "ref",
        }
    }
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.name() == name)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnCapabilities(u8);
impl FnCapabilities {
    pub fn contains(self, capability: FnCapability) -> bool {
        self.0 & (1 << capability as u8) != 0
    }
    pub fn insert(&mut self, capability: FnCapability) -> bool {
        let old = self.0;
        self.0 |= 1 << capability as u8;
        old != self.0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn allows(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
    pub fn is_pure(self) -> bool {
        self.0 == 0
    }
}
impl std::fmt::Display for FnCapabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut separator = "";
        for capability in FnCapability::ALL
            .into_iter()
            .filter(|capability| self.contains(*capability))
        {
            write!(f, "{separator}{}", capability.name())?;
            separator = ", ";
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    #[serde(default)]
    pub capabilities: FnCapabilities,
    #[serde(default)]
    pub mutates: bool,
    pub generics: Vec<GenericParameter<Type, TypeBound>>,
    pub default_count: usize,
    pub parameters: Vec<FunctionParameter>,
    pub result: Box<Type>,
}

impl Signature {
    fn substitute(&self, bindings: &BTreeMap<String, Type>) -> Self {
        let bindings = bindings
            .iter()
            .filter(|(name, _)| {
                !self
                    .generics
                    .iter()
                    .any(|parameter| &parameter.name == *name)
            })
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();
        let mut signature = self.clone();
        for parameter in &mut signature.parameters {
            parameter.ty = parameter.ty.substitute(&bindings);
        }
        signature.result = Box::new(signature.result.substitute(&bindings));
        signature
    }
}

impl Type {
    pub fn signatures(&self) -> &[Signature] {
        match self {
            Self::Function(signature) => std::slice::from_ref(signature),
            Self::Overloaded(signatures) => signatures,
            _ => &[],
        }
    }
    pub fn signatures_mut(&mut self) -> &mut [Signature] {
        match self {
            Self::Function(signature) => std::slice::from_mut(signature),
            Self::Overloaded(signatures) => signatures,
            _ => &mut [],
        }
    }
    /// Parent-local 2D coordinates are ordinary homogeneous numeric Lists.
    pub fn coordinates() -> Self {
        Self::union(
            [Unit::Length, Unit::Percent]
                .map(|unit| Self::List(Box::new(Self::Numeric(unit)), ListLength::Exact(2))),
        )
        .expect("nonempty coordinate units")
    }
    /// A fixed, checked index is total; unknown cardinality still requires Option.
    pub fn fixed_index(&self, index: usize) -> Result<Option<Self>, String> {
        match self {
            Self::List(inner, ListLength::Exact(length)) => {
                if index >= *length {
                    return Err(format!(
                        "index {index} is out of bounds for List length {length}"
                    ));
                }
                Ok(Some(inner.as_ref().clone()))
            }
            Self::Union(types) => {
                let mut elements = Vec::with_capacity(types.len());
                for ty in types {
                    let Some(inner) = ty.fixed_index(index)? else {
                        return Ok(None);
                    };
                    elements.push(inner);
                }
                Self::union(elements).map(Some)
            }
            _ => Ok(None),
        }
    }
    pub fn concatenate(&self, other: &Self) -> Result<Self, String> {
        let (Self::List(left, a), Self::List(right, b)) = (self, other) else {
            return Err("concat requires two Lists".into());
        };
        let inner = if **left == Self::Unknown {
            right.as_ref().clone()
        } else if **right == Self::Unknown || left.accepts(right) {
            left.as_ref().clone()
        } else if right.accepts(left) {
            right.as_ref().clone()
        } else {
            left.join_list_lengths(right).ok_or_else(|| {
                format!("concat requires compatible element types, found {left} and {right}")
            })?
        };
        Ok(Self::List(Box::new(inner), a.checked_add(*b)?))
    }
    pub fn numeric_unit(&self) -> Option<Unit> {
        match self {
            Self::Numeric(unit) => Some(*unit),
            Self::RefinedNumeric(domain) => Some(domain.unit),
            _ => None,
        }
    }
    pub fn numeric_domain(&self) -> Option<crate::math::domain::NumericDomain> {
        match self {
            Self::Numeric(unit) => Some(crate::math::domain::NumericDomain::all(*unit, false)),
            Self::RefinedNumeric(domain) => Some(domain.clone()),
            _ => None,
        }
    }
    /// Homogeneous literals can contain rows of different lengths without
    /// erasing their element types. Only length facts and inference holes join.
    pub fn join_list_lengths(&self, other: &Self) -> Option<Self> {
        if self == other || *other == Self::Unknown {
            return Some(self.clone());
        }
        if *self == Self::Unknown {
            return Some(other.clone());
        }
        match (self, other) {
            (Self::List(a, la), Self::List(b, lb)) => Some(Self::List(
                Box::new(a.join_list_lengths(b)?),
                if la == lb { *la } else { ListLength::Unknown },
            )),
            (Self::Record(a), Self::Record(b)) if a.keys().eq(b.keys()) => Some(Self::Record(
                a.iter()
                    .map(|(name, ty)| Some((name.clone(), ty.join_list_lengths(&b[name])?)))
                    .collect::<Option<_>>()?,
            )),
            (Self::Option(a), Self::Option(b)) => {
                Some(Self::Option(Box::new(a.join_list_lengths(b)?)))
            }
            (Self::Array(a), Self::Array(b)) => {
                Some(Self::Array(Box::new(a.join_list_lengths(b)?)))
            }
            _ => None,
        }
    }
    /// Whether two checked domains may share a value. Generic holes are
    /// conservative; disjointness must be proved before accepting clauses.
    pub fn overlaps(&self, other: &Self) -> bool {
        if let Self::Union(types) = self {
            return types.iter().any(|ty| ty.overlaps(other));
        }
        if let Self::Union(types) = other {
            return types.iter().any(|ty| self.overlaps(ty));
        }
        if let (Some(left), Some(right)) = (self.numeric_domain(), other.numeric_domain()) {
            return left.intersection(&right).is_some();
        }
        if matches!(
            self,
            Self::Unknown | Self::Variable(_) | Self::BoundedVariable(..)
        ) || matches!(
            other,
            Self::Unknown | Self::Variable(_) | Self::BoundedVariable(..)
        ) {
            return true;
        }
        match (self, other) {
            // Empty collections and None inhabit both element domains.
            (Self::List(a, la), Self::List(b, lb)) => {
                (la == lb || *la == ListLength::Unknown || *lb == ListLength::Unknown)
                    && (la.allows(0) && lb.allows(0) || a.overlaps(b))
            }
            (Self::Array(_), Self::Array(_)) | (Self::Option(_), Self::Option(_)) => true,
            (Self::Series(a), Self::Series(b)) | (Self::Signal(a), Self::Signal(b)) => {
                a.overlaps(b)
            }
            (Self::DataFrame { row: a, groups: ga }, Self::DataFrame { row: b, groups: gb }) => {
                ga == gb && a.overlaps(b)
            }
            (Self::StringLiteral(left), Self::StringLiteral(right)) => left == right,
            (Self::Record(left), Self::Record(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .all(|(name, ty)| right.get(name).is_some_and(|other| ty.overlaps(other)))
            }
            _ => self.accepts(other) || other.accepts(self),
        }
    }
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
        let mut domains = Vec::new();
        alternatives.retain(|ty| {
            if let Some(domain) = ty.numeric_domain() {
                domains.push(domain);
                false
            } else {
                true
            }
        });
        domains.sort_by(|a, b| {
            a.unit
                .cmp(&b.unit)
                .then(a.integer.cmp(&b.integer))
                .then_with(|| match (a.lower, b.lower) {
                    (Some(a), Some(b)) => {
                        a.0.partial_cmp(&b.0)
                            .expect("finite checked type bounds")
                            .then(b.1.cmp(&a.1))
                    }
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                })
        });
        let mut merged: Vec<crate::math::domain::NumericDomain> = Vec::new();
        for domain in domains {
            if let Some(previous) = merged.last_mut()
                && let Some(union) = previous.union(&domain)
            {
                *previous = union;
            } else {
                merged.push(domain);
            }
        }
        let broad = merged
            .iter()
            .filter(|domain| !domain.integer && domain.lower.is_none() && domain.upper.is_none())
            .map(|domain| domain.unit)
            .collect::<std::collections::BTreeSet<_>>();
        alternatives.extend(
            merged
                .into_iter()
                .filter(|domain| !domain.integer || !broad.contains(&domain.unit))
                .map(|domain| {
                    if !domain.integer && domain.lower.is_none() && domain.upper.is_none() {
                        Self::Numeric(domain.unit)
                    } else {
                        Self::RefinedNumeric(domain)
                    }
                }),
        );
        alternatives
            .sort_by_cached_key(|ty| serde_json::to_vec(ty).expect("serializable semantic type"));
        alternatives.dedup();
        if alternatives.contains(&Self::String) {
            alternatives.retain(|ty| !matches!(ty, Self::StringLiteral(_)));
        }
        match alternatives.len() {
            0 => Err("a union type requires at least one alternative".into()),
            1 => Ok(alternatives
                .pop()
                .expect("one normalized union alternative")),
            _ => Ok(Self::Union(alternatives)),
        }
    }
    /// Literal branches remain useful to editors inside mixed numeric/enum unions.
    pub fn string_literal_choices(&self) -> Vec<&str> {
        let mut values = Vec::new();
        self.collect_string_literals(&mut values);
        values
    }
    /// Finite choices only when the entire type is a closed string-literal domain.
    pub fn string_literals(&self) -> Option<Vec<&str>> {
        let mut values = Vec::new();
        self.collect_string_literals(&mut values).then_some(values)
    }
    fn collect_string_literals<'a>(&'a self, values: &mut Vec<&'a str>) -> bool {
        match self {
            Self::StringLiteral(value) => {
                values.push(value.as_ref());
                true
            }
            Self::Union(types) => types.iter().fold(true, |closed, ty| {
                ty.collect_string_literals(values) & closed
            }),
            Self::BoundedVariable(_, bound) => {
                if let TypeBound::Type(ty) = bound.as_ref() {
                    ty.collect_string_literals(values);
                }
                false
            }
            _ => false,
        }
    }
    /// Editable values infer primitive string schemas; explicit annotations can
    /// retain a finite choice. Runtime strings themselves remain ordinary strings.
    pub fn widen_literals(self) -> Self {
        self.widen(Widening::Literals)
    }
    pub fn editable_schema(self) -> Self {
        self.widen(Widening::Editable)
    }
    pub fn signal_schema(self) -> Self {
        self.widen(Widening::Signal)
    }
    pub fn without_list_lengths(self) -> Self {
        self.widen(Widening::ListLengths)
    }
    fn widen(self, mode: Widening) -> Self {
        let editable = mode != Widening::Literals;
        let literals = mode != Widening::ListLengths;
        let live = mode == Widening::Signal;
        match self {
            Self::StringLiteral(_) if literals => Self::String,
            Self::RefinedNumeric(domain) if live || literals && domain.is_singleton() => {
                Self::Numeric(domain.unit)
            }
            Self::Union(types) => Self::union(types.into_iter().map(|ty| ty.widen(mode)))
                .expect("nonempty checked union"),
            Self::List(inner, length) => Self::List(
                Box::new(inner.widen(mode)),
                if editable {
                    ListLength::Unknown
                } else {
                    length
                },
            ),
            Self::Option(inner) => Self::Option(Box::new(inner.widen(mode))),
            Self::Array(inner) => Self::Array(Box::new(inner.widen(mode))),
            Self::Signal(inner) => Self::Signal(Box::new(inner.widen(mode))),
            Self::Record(fields) => Self::Record(
                fields
                    .into_iter()
                    .map(|(name, ty)| (name, ty.widen(mode)))
                    .collect(),
            ),
            ty => ty,
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
            Self::Unknown => true,
            Self::Variable(_) | Self::BoundedVariable(..) => !symbolic_parameters,
            Self::List(t, _)
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
        self.contains(&|ty| !ty.signatures().is_empty())
    }
    pub fn contains_signal(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Signal(_)))
    }
    pub fn contains_element(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Element))
    }
    pub fn contains_animation(&self) -> bool {
        self.contains(&|ty| matches!(ty, Self::Animation))
    }
    fn contains(&self, predicate: &impl Fn(&Type) -> bool) -> bool {
        if predicate(self) {
            return true;
        }
        match self {
            Self::List(t, _)
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
            // A callable's arguments and result are future values, not payloads it contains.
            Self::Function(_) | Self::Overloaded(_) => false,
            Self::BoundedVariable(_, bound) => {
                matches!(bound.as_ref(), TypeBound::Type(ty) if ty.contains(predicate))
            }
            _ => false,
        }
    }
    pub fn accepts(&self, actual: &Type) -> bool {
        if let (Some(expected), Some(actual)) = (self.numeric_domain(), actual.numeric_domain()) {
            return expected.accepts(&actual);
        }
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
                (Self::String, Self::StringLiteral(_)) => true,
                (Self::Function(expected), Self::Function(actual)) => {
                    expected.capabilities.allows(actual.capabilities)
                        && expected.generics == actual.generics
                        && expected.parameters.len() == actual.parameters.len()
                        && expected.default_count <= actual.default_count
                        && expected.parameters.iter().zip(&actual.parameters).all(
                            |(expected, actual)| {
                                expected.name == actual.name
                                    && expected.kind == actual.kind
                                    && actual.ty.accepts(&expected.ty)
                            },
                        )
                        && expected.result.accepts(&actual.result)
                }
                (Self::Code(crate::code::Language::Auto), Self::Code(_)) => true,
                (Self::Signal(a), Self::Signal(b)) => a.accepts(b),
                (Self::List(a, la), Self::List(b, lb)) => {
                    la.accepts(*lb) && (**b == Self::Unknown || a.accepts(b))
                }
                (Self::Option(a), Self::Option(b))
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
            | Self::StringLiteral(_)
            | Self::Code(_)
            | Self::Numeric(_)
            | Self::RefinedNumeric(_)
            | Self::Fraction
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
            Self::List(t, _) | Self::Array(t) | Self::Option(t) => t.readable(),
            Self::Record(fields) => fields.values().all(Self::readable),
            Self::Union(alternatives) => {
                !alternatives.is_empty() && alternatives.iter().all(Self::readable)
            }
            _ => false,
        }
    }
    pub fn data_convertible(&self) -> bool {
        match self {
            Self::Font | Self::Code(_) => false,
            Self::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Numeric | TypeBound::Reflectable => true,
                TypeBound::Type(ty) => ty.data_convertible(),
                TypeBound::Readable => false,
            },
            Self::List(t, _) | Self::Array(t) | Self::Option(t) => t.data_convertible(),
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
            Self::Font | Self::Code(_) => false,
            Self::BoundedVariable(_, bound) => match bound.as_ref() {
                TypeBound::Reflectable | TypeBound::Numeric => true,
                TypeBound::Type(ty) => ty.reflectable(),
                _ => false,
            },
            Self::Enum { variants, .. } => variants.values().flatten().all(Self::reflectable),
            Self::List(t, _) | Self::Array(t) | Self::Option(t) => t.reflectable(),
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
            Self::Function(signature) => Self::Function(signature.substitute(bindings)),
            Self::Overloaded(signatures) => Self::Overloaded(
                signatures
                    .iter()
                    .map(|signature| signature.substitute(bindings))
                    .collect(),
            ),
            Self::List(t, length) => Self::List(Box::new(t.substitute(bindings)), *length),
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
            Self::Forever => write!(f, "Forever"),
            Self::Bool => write!(f, "Bool"),
            Self::String => write!(f, "String"),
            Self::StringLiteral(value) => f.write_str(&crate::template::quote_string(value)),
            Self::Code(language) => write!(f, "Code<{}>", language.name()),
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
            Self::RefinedNumeric(domain) => {
                if domain.integer && domain.unit == Unit::Number {
                    write!(f, "Int")?;
                } else {
                    write!(f, "{}", Self::Numeric(domain.unit))?;
                }
                if let Some((value, inclusive)) = domain.lower {
                    write!(f, " {} {value}", if inclusive { ">=" } else { ">" })?;
                }
                if let Some((value, inclusive)) = domain.upper {
                    write!(f, " {} {value}", if inclusive { "<=" } else { "<" })?;
                }
                Ok(())
            }
            Self::Matrix4 => write!(f, "Matrix4"),
            Self::Color => write!(f, "Color"),
            Self::Element => write!(f, "Element"),
            Self::Animation => write!(f, "Animation"),
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
            Self::List(t, ListLength::Unknown) => write!(f, "List<{t}>"),
            Self::List(t, ListLength::Exact(length)) => write!(f, "List<{t},{length}>"),
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
                write!(f, "fn")?;
                if !sig.capabilities.is_pure() {
                    write!(f, "<{}>", sig.capabilities)?;
                }
                write!(f, "(")?;
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
            Self::Overloaded(signatures) => {
                for (index, signature) in signatures.iter().enumerate() {
                    if index > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", Self::Function(signature.clone()))?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub [Fraction; 4]);

impl Color {
    /// Shared Adwaita names, used by editors without copying the palette.
    pub fn names() -> impl Iterator<Item = &'static str> {
        shrimply_math_color::Color::ADWAITA
            .iter()
            .map(|(name, _)| *name)
    }
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
pub struct Vec2 {
    pub values: [f64; 2],
    pub unit: Unit,
}
impl Vec2 {
    pub fn zero(unit: Unit) -> Self {
        Self {
            values: [0.0; 2],
            unit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Unit,
    Forever,
    Bool(bool),
    String(Arc<str>),
    Code(Arc<crate::code::Document>),
    Number(Number),
    Fraction(Fraction),
    Matrix4(Box<crate::math::linear::Matrix4>),
    Color(Color),
    List(List),
    Array {
        element_type: Arc<Type>,
        shape: Vec<usize>,
        values: crate::Values,
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
    Animation(Arc<crate::plan::Animation>),
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
            Self::Forever => Type::Forever,
            Self::Bool(_) => Type::Bool,
            Self::String(_) => Type::String,
            Self::Code(document) => Type::Code(document.language),
            Self::Template(_) => Type::Template,
            Self::Signal(signal) => Type::Signal(Box::new(signal.ty.as_ref().clone())),
            Self::Sql(_) => Type::Sql,
            Self::SqlIdent(_) => Type::SqlIdent,
            Self::Number(n) => Type::Numeric(n.unit),
            Self::Fraction(_) => Type::Fraction,
            Self::Matrix4(_) => Type::Matrix4,
            Self::Color(_) => Type::Color,
            Self::List(v) => Type::List(
                Box::new(v.element_type.as_ref().clone()),
                ListLength::Exact(v.len()),
            ),
            Self::Array { element_type, .. } => {
                Type::Array(Box::new(element_type.as_ref().clone()))
            }
            Self::Record(v) => Type::Record(v.iter().map(|(k, v)| (k.clone(), v.ty())).collect()),
            Self::Some(v) => Type::Option(Box::new(v.ty())),
            Self::None => Type::Option(Box::new(Type::Unknown)),
            Self::Variant { ty, .. } => ty.as_ref().clone(),
            Self::Element(_) => Type::Element,
            Self::Animation(_) => Type::Animation,
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
        if let (Type::List(_, length), Self::List(list)) = (expected, self)
            && !length.allows(list.len())
        {
            return Err(format!(
                "expected {expected}, found List with {} items",
                list.len()
            ));
        }
        match (expected, self) {
            (Type::List(inner, _), Self::List(list))
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
        if let Type::Union(alternatives) = expected {
            for alternative in alternatives {
                if alternative.accepts(&actual) && crate::validate_value(alternative, self).is_err()
                {
                    continue;
                }
                if let Ok(value) = self.with_type(alternative) {
                    return Ok(value);
                }
            }
            // Only the diagnostic path needs all alternative failure reasons.
            return Err(crate::validate_value(expected, self)
                .expect_err("each union alternative failed validation"));
        }
        if !expected.accepts(&actual) {
            crate::validate_value(expected, self)?;
        }
        if expected == &actual || expected.unresolved() {
            return Ok(self.clone());
        }
        Ok(match (expected, self) {
            (Type::List(inner, _), Self::List(list)) => Self::List(List::from_values(
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
    /// Convert canonical numeric Lists at the native geometry boundary only.
    pub fn vector(&self) -> Result<(Vec<f64>, Unit), String> {
        let values = self.list()?;
        let mut unit = None;
        let values = values
            .iter()
            .map(|value| {
                let number = value.number()?;
                if unit.is_some_and(|unit| unit != number.unit) {
                    return Err("numeric List components must have matching units".into());
                }
                unit = Some(number.unit);
                crate::math::to_f64(number.value)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((values, unit.unwrap_or(Unit::Number)))
    }
    pub fn vec2(&self) -> Result<Vec2, String> {
        let values = self.list()?;
        if values.len() != 2 {
            return Err(format!(
                "expected a numeric List of length 2, found {} items",
                values.len()
            ));
        }
        let x = values[0].number()?;
        let y = values[1].number()?;
        if x.unit != y.unit {
            return Err("coordinate List components must have matching units".into());
        }
        Ok(Vec2 {
            values: [crate::math::to_f64(x.value)?, crate::math::to_f64(y.value)?],
            unit: x.unit,
        })
    }
    pub fn from_vector(values: Vec<f64>, unit: Unit) -> Result<Self, String> {
        let values = values
            .into_iter()
            .map(|value| {
                Ok(Value::Number(Number {
                    value: crate::math::from_f64(value)?,
                    unit,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self::List(List::typed(Type::Numeric(unit), values)))
    }
    pub fn list(&self) -> Result<&crate::Values, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn union_normalization_rejects_invalid_values_even_when_primitive_types_match() {
        let invalid = [
            (Type::Color, Value::Color(Color([Fraction::from(2); 4]))),
            (
                Type::Numeric(Unit::Number),
                Value::Number(Number {
                    value: Fraction::nan(),
                    unit: Unit::Number,
                }),
            ),
            (
                Type::List(Box::new(Type::Color), crate::ListLength::Unknown),
                Value::List(List::typed(
                    Type::Color,
                    vec![Value::Color(Color([Fraction::from(2); 4]))],
                )),
            ),
        ];
        for (ty, value) in invalid {
            let union = Type::union([ty, Type::String]).unwrap();
            assert!(
                value.with_type(&union).is_err(),
                "invalid runtime contents must not pass a union by their broad type alone"
            );
        }
    }
}
