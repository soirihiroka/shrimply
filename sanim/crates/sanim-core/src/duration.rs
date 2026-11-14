//! Bounded, exact timeline formulas, separate from value types.
mod budget;
use crate::{Fraction, Span, math};
pub use budget::DurationBudget;
use num_traits::{CheckedAdd, CheckedMul, ToPrimitive};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fmt, sync::Arc};

pub const MAX_DURATION_TERMS: usize = 128;
pub const MAX_DURATION_NODES: usize = 512;
pub const MAX_DURATION_DEPTH: usize = 32;
pub const MAX_DURATION_ALTERNATIVES: usize = 32;
pub const MAX_DURATION_SYMBOL_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DurationSymbol {
    Parameter(Arc<str>),
    Length(Arc<str>),
}
impl fmt::Display for DurationSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter(name) => f.write_str(name),
            Self::Length(name) => write!(f, "len({name})"),
        }
    }
}

/// Immutable children share their allocations across function signatures and edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationExpr(Arc<Expression>);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Expression {
    Constant(Fraction),
    Symbol(DurationSymbol),
    Sum(Arc<[DurationExpr]>),
    Product(Arc<[DurationExpr]>),
    Maximum(Arc<[DurationExpr]>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Sum,
    Product,
    Maximum,
}
impl DurationExpr {
    pub fn constant(value: Fraction) -> Option<Self> {
        math::finite(&value).then(|| Self(Arc::new(Expression::Constant(value))))
    }
    pub fn parameter(name: impl Into<Arc<str>>) -> Option<Self> {
        Self::symbol(DurationSymbol::Parameter(name.into()))
    }
    pub fn length(name: impl Into<Arc<str>>) -> Option<Self> {
        Self::symbol(DurationSymbol::Length(name.into()))
    }
    fn symbol(symbol: DurationSymbol) -> Option<Self> {
        let (DurationSymbol::Parameter(name) | DurationSymbol::Length(name)) = &symbol;
        (!name.is_empty() && name.len() <= MAX_DURATION_SYMBOL_BYTES)
            .then(|| Self(Arc::new(Expression::Symbol(symbol))))
    }
    pub fn value(&self) -> Option<Fraction> {
        if let Expression::Constant(value) = self.0.as_ref() {
            Some(*value)
        } else {
            None
        }
    }
    pub fn sum(terms: impl IntoIterator<Item = Self>) -> Option<Self> {
        Self::combine(terms, Operation::Sum)
    }
    pub fn product(terms: impl IntoIterator<Item = Self>) -> Option<Self> {
        Self::combine(terms, Operation::Product)
    }
    pub fn maximum(terms: impl IntoIterator<Item = Self>) -> Option<Self> {
        Self::combine(terms, Operation::Maximum)
    }
    fn combine(terms: impl IntoIterator<Item = Self>, kind: Operation) -> Option<Self> {
        let mut output = Vec::new();
        let mut constant = None;
        let mut visited = 0;
        for term in terms {
            visited += 1;
            if visited > MAX_DURATION_NODES {
                return None;
            }
            let nested = match (kind, term.0.as_ref()) {
                (Operation::Sum, Expression::Sum(v))
                | (Operation::Product, Expression::Product(v))
                | (Operation::Maximum, Expression::Maximum(v)) => Some(v),
                _ => None,
            };
            let values = nested.map_or_else(|| std::slice::from_ref(&term), |v| v.as_ref());
            if values.len() > MAX_DURATION_TERMS {
                return None;
            }
            for value in values {
                if let Some(value) = value.value() {
                    constant = Some(match constant {
                        None => value,
                        Some(previous) => match kind {
                            Operation::Sum => value.checked_add(&previous)?,
                            Operation::Product => value.checked_mul(&previous)?,
                            _ => value.max(previous),
                        },
                    });
                } else {
                    output.push(value.clone());
                }
                if output.len() > MAX_DURATION_TERMS {
                    return None;
                }
            }
        }
        let zero = Fraction::from(0);
        let one = Fraction::from(1);
        if kind == Operation::Product && constant == Some(zero) {
            return Self::constant(zero);
        }
        if let Some(value) = constant
            && (output.is_empty()
                || (kind == Operation::Sum && value != zero)
                || (kind == Operation::Product && value != one)
                || kind == Operation::Maximum)
        {
            output.push(Self::constant(value)?);
        }
        output.sort_by(Self::compare);
        if kind == Operation::Maximum {
            output.dedup();
        }
        let result = match output.len() {
            0 => {
                return Self::constant(if kind == Operation::Product {
                    one
                } else {
                    zero
                });
            }
            1 => return output.pop(),
            _ => Self(Arc::new(match kind {
                Operation::Sum => Expression::Sum(output.into()),
                Operation::Product => Expression::Product(output.into()),
                _ => Expression::Maximum(output.into()),
            })),
        };
        result.complexity().map(|_| result)
    }
    fn compare(a: &Self, b: &Self) -> Ordering {
        fn rank(value: &Expression) -> u8 {
            match value {
                Expression::Constant(_) => 0,
                Expression::Symbol(_) => 1,
                Expression::Sum(_) => 2,
                Expression::Product(_) => 3,
                Expression::Maximum(_) => 4,
            }
        }
        rank(&a.0)
            .cmp(&rank(&b.0))
            .then_with(|| match (a.0.as_ref(), b.0.as_ref()) {
                (Expression::Constant(a), Expression::Constant(b)) => {
                    a.partial_cmp(b).unwrap_or(Ordering::Equal)
                }
                (Expression::Symbol(a), Expression::Symbol(b)) => a.cmp(b),
                (Expression::Sum(a), Expression::Sum(b))
                | (Expression::Product(a), Expression::Product(b))
                | (Expression::Maximum(a), Expression::Maximum(b)) => {
                    a.len().cmp(&b.len()).then_with(|| {
                        a.iter()
                            .zip(b.iter())
                            .map(|(a, b)| Self::compare(a, b))
                            .find(|v| *v != Ordering::Equal)
                            .unwrap_or(Ordering::Equal)
                    })
                }
                _ => Ordering::Equal,
            })
    }
    /// Counts references too: exponentially expanded shared DAGs stay bounded to resolve.
    pub fn complexity(&self) -> Option<usize> {
        let mut stack = vec![(self, 1)];
        let mut count = 0;
        while let Some((value, depth)) = stack.pop() {
            count += 1;
            if count > MAX_DURATION_NODES || depth > MAX_DURATION_DEPTH {
                return None;
            }
            match value.0.as_ref() {
                Expression::Constant(value) if !math::finite(value) => return None,
                Expression::Symbol(
                    DurationSymbol::Parameter(name) | DurationSymbol::Length(name),
                ) if name.is_empty() || name.len() > MAX_DURATION_SYMBOL_BYTES => return None,
                Expression::Sum(values)
                | Expression::Product(values)
                | Expression::Maximum(values) => {
                    if values.len() > MAX_DURATION_TERMS {
                        return None;
                    }
                    stack.extend(values.iter().map(|v| (v, depth + 1)));
                }
                _ => {}
            }
        }
        Some(count)
    }
    fn resolve(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<Fraction>,
    ) -> Result<Option<Fraction>, String> {
        if self.complexity().is_none() {
            return Ok(None);
        }
        self.resolve_inner(lookup)
    }
    fn resolve_inner(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<Fraction>,
    ) -> Result<Option<Fraction>, String> {
        match self.0.as_ref() {
            Expression::Constant(value) => Ok(Some(*value)),
            Expression::Symbol(symbol) => match lookup(symbol) {
                Some(value) if !math::finite(&value) => {
                    Err(format!("duration input {symbol} must be finite"))
                }
                Some(value)
                    if matches!(symbol, DurationSymbol::Length(_)) && value < Fraction::from(0) =>
                {
                    Err(format!("duration input {symbol} cannot be negative"))
                }
                value => Ok(value),
            },
            Expression::Sum(values) | Expression::Product(values) | Expression::Maximum(values) => {
                let mut result = None;
                for value in values.iter() {
                    let Some(value) = value.resolve_inner(lookup)? else {
                        return Ok(None);
                    };
                    result = Some(match result {
                        None => value,
                        Some(previous) => match self.0.as_ref() {
                            Expression::Sum(_) => value
                                .checked_add(&previous)
                                .ok_or("symbolic duration addition overflow")?,
                            Expression::Product(_) => value
                                .checked_mul(&previous)
                                .ok_or("symbolic duration multiplication overflow")?,
                            _ => value.max(previous),
                        },
                    });
                }
                Ok(result)
            }
        }
    }
    pub fn substitute(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<Self>,
    ) -> Option<Self> {
        self.complexity()?;
        self.substitute_inner(lookup)
    }
    fn substitute_inner(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<Self>,
    ) -> Option<Self> {
        match self.0.as_ref() {
            Expression::Constant(_) => Some(self.clone()),
            Expression::Symbol(symbol) => {
                let result = lookup(symbol).unwrap_or_else(|| self.clone());
                result.complexity().map(|_| result)
            }
            Expression::Sum(values) | Expression::Product(values) | Expression::Maximum(values) => {
                let terms: Option<Vec<_>> =
                    values.iter().map(|v| v.substitute_inner(lookup)).collect();
                match self.0.as_ref() {
                    Expression::Sum(_) => Self::sum(terms?),
                    Expression::Product(_) => Self::product(terms?),
                    _ => Self::maximum(terms?),
                }
            }
        }
    }
}
impl fmt::Display for DurationExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_ref() {
            Expression::Constant(value) => write!(f, "{value}"),
            Expression::Symbol(symbol) => write!(f, "{symbol}"),
            Expression::Sum(values) | Expression::Product(values) | Expression::Maximum(values) => {
                let (start, separator, end) = match self.0.as_ref() {
                    Expression::Sum(_) => ("(", " + ", ")"),
                    Expression::Product(_) => ("(", " * ", ")"),
                    _ => ("max(", ", ", ")"),
                };
                f.write_str(start)?;
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        f.write_str(separator)?;
                    }
                    write!(f, "{value}")?;
                }
                f.write_str(end)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineEffect {
    Fixed(Fraction),
    Symbolic(DurationExpr),
    Alternatives(Arc<[TimelineEffect]>),
    Unknown,
}
impl Default for TimelineEffect {
    fn default() -> Self {
        Self::Fixed(Fraction::from(0))
    }
}
impl TimelineEffect {
    pub fn from_expr(expr: DurationExpr) -> Self {
        if expr.complexity().is_none() {
            Self::Unknown
        } else {
            expr.value().map_or(Self::Symbolic(expr), Self::Fixed)
        }
    }
    pub fn expression(&self) -> Option<DurationExpr> {
        match self {
            Self::Fixed(value) => DurationExpr::constant(*value),
            Self::Symbolic(expr) => Some(expr.clone()),
            _ => None,
        }
    }
    pub fn then(self, next: Self) -> Result<Self, String> {
        match (&self, &next) {
            (Self::Fixed(a), Self::Fixed(b)) => return math::timeline_end(*a, *b).map(Self::Fixed),
            (Self::Alternatives(values), _) => {
                return Self::alternatives(
                    values
                        .iter()
                        .cloned()
                        .map(|v| v.then(next.clone()))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            (_, Self::Alternatives(values)) => {
                return Self::alternatives(
                    values
                        .iter()
                        .cloned()
                        .map(|v| self.clone().then(v))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            _ => {}
        }
        Ok(match (self.expression(), next.expression()) {
            (Some(a), Some(b)) => DurationExpr::sum([a, b]).map_or(Self::Unknown, Self::from_expr),
            _ => Self::Unknown,
        })
    }
    pub fn repeated(self, count: DurationExpr) -> Self {
        if count.value() == Some(Fraction::from(0)) {
            return Self::default();
        }
        match self {
            Self::Alternatives(values) => {
                // Each iteration can choose independently; multiplying each branch
                // would omit mixed totals (1|2 repeated twice includes 3).
                let Some(repetitions) = count
                    .value()
                    .and_then(|v| v.to_u64().filter(|n| Fraction::from(*n) == v))
                else {
                    return Self::Unknown;
                };
                if repetitions > MAX_DURATION_ALTERNATIVES as u64 {
                    return Self::Unknown;
                }
                let mut total = Self::default();
                for _ in 0..repetitions {
                    total = match total.then(Self::Alternatives(values.clone())) {
                        Ok(value) => value,
                        Err(_) => return Self::Unknown,
                    };
                    if total == Self::Unknown {
                        break;
                    }
                }
                total
            }
            other => other
                .expression()
                .and_then(|v| DurationExpr::product([v, count]))
                .map_or(Self::Unknown, Self::from_expr),
        }
    }
    pub fn parallel(self, other: Self) -> Self {
        match (&self, &other) {
            (Self::Alternatives(values), _) => {
                return Self::alternatives(
                    values.iter().cloned().map(|v| v.parallel(other.clone())),
                )
                .unwrap_or(Self::Unknown);
            }
            (_, Self::Alternatives(values)) => {
                return Self::alternatives(
                    values.iter().cloned().map(|v| self.clone().parallel(v)),
                )
                .unwrap_or(Self::Unknown);
            }
            _ => {}
        }
        match (self.expression(), other.expression()) {
            (Some(a), Some(b)) => {
                DurationExpr::maximum([a, b]).map_or(Self::Unknown, Self::from_expr)
            }
            _ => Self::Unknown,
        }
    }
    pub fn alternatives(values: impl IntoIterator<Item = Self>) -> Result<Self, String> {
        let mut output = Vec::new();
        let mut visited = 0;
        for value in values {
            visited += 1;
            if visited > MAX_DURATION_NODES {
                return Ok(Self::Unknown);
            }
            let values = match &value {
                Self::Alternatives(v) => v.as_ref(),
                value => std::slice::from_ref(value),
            };
            if values.len() > MAX_DURATION_ALTERNATIVES {
                return Ok(Self::Unknown);
            }
            for value in values {
                if let Self::Fixed(value) = value {
                    math::timeline_end(Fraction::from(0), *value)?;
                }
                if !output.contains(value) {
                    output.push(value.clone());
                }
                if output.len() > MAX_DURATION_ALTERNATIVES {
                    return Ok(Self::Unknown);
                }
            }
        }
        output.sort_by_cached_key(ToString::to_string);
        Ok(match output.len() {
            0 => Self::default(),
            1 => output.pop().expect("one duration"),
            _ => Self::Alternatives(output.into()),
        })
    }
    pub fn substitute(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<DurationExpr>,
    ) -> Result<Self, String> {
        match self {
            Self::Symbolic(expr) => Ok(expr
                .substitute(lookup)
                .map_or(Self::Unknown, Self::from_expr)),
            Self::Alternatives(values) => Self::alternatives(
                values
                    .iter()
                    .map(|v| v.substitute(lookup))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            _ => Ok(self.clone()),
        }
    }
    /// None means unresolved; every fully resolved alternative is checked against the limit.
    pub fn resolve(
        &self,
        lookup: &mut impl FnMut(&DurationSymbol) -> Option<Fraction>,
    ) -> Result<Option<Vec<Fraction>>, String> {
        let values = match self {
            Self::Fixed(value) => vec![*value],
            Self::Symbolic(expr) => match expr.resolve(lookup)? {
                Some(value) => vec![value],
                None => return Ok(None),
            },
            Self::Alternatives(values) => {
                if values.len() > MAX_DURATION_ALTERNATIVES {
                    return Ok(None);
                }
                let mut resolved = Vec::new();
                for value in values.iter() {
                    let Some(values) = value.resolve(lookup)? else {
                        return Ok(None);
                    };
                    resolved.extend(values);
                }
                resolved.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                resolved.dedup();
                resolved
            }
            Self::Unknown => return Ok(None),
        };
        for value in &values {
            math::timeline_end(Fraction::from(0), *value)?;
        }
        Ok(Some(values))
    }
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}
impl fmt::Display for TimelineEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed(value) => write!(f, "{value}s"),
            Self::Symbolic(expr) => write!(f, "{expr}s"),
            Self::Unknown => f.write_str("unknown"),
            Self::Alternatives(values) => {
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{value}")?;
                }
                Ok(())
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDuration {
    pub span: Span,
    pub effect: TimelineEffect,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloned_formulas_share_storage_and_release_when_last_revision_drops() {
        let expression = DurationExpr::product([
            DurationExpr::parameter("count").unwrap(),
            DurationExpr::constant(Fraction::new(1u64, 4u64)).unwrap(),
        ])
        .unwrap();
        let weak = Arc::downgrade(&expression.0);
        let revisions: Vec<_> = (0..4096).map(|_| expression.clone()).collect();
        assert!(revisions.iter().all(|v| Arc::ptr_eq(&expression.0, &v.0)));
        drop(expression);
        assert!(weak.upgrade().is_some());
        drop(revisions);
        assert!(weak.upgrade().is_none());
    }
}

#[cfg(test)]
mod budget_lifetime_tests {
    use super::*;
    #[test]
    fn publication_accounting_does_not_keep_a_formula_value_alive() {
        let expr = DurationExpr::parameter("source_rows").unwrap();
        let weak = Arc::downgrade(&expr.0);
        let effect = TimelineEffect::Symbolic(expr);
        let mut budget = DurationBudget::default();
        let mut remaining = 1024 * 1024;
        budget.charge(&effect, &mut remaining).unwrap();
        drop(effect);
        assert!(weak.upgrade().is_none());
        drop(budget);
    }
}
