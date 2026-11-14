//! Decidable numeric type domains. Bounds use the same exact arithmetic as values.
use crate::{Fraction, Number, Unit};
use num_traits::{CheckedAdd, CheckedSub};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericDomain {
    pub unit: Unit,
    pub integer: bool,
    pub lower: Option<(Fraction, bool)>,
    pub upper: Option<(Fraction, bool)>,
}

impl NumericDomain {
    pub fn all(unit: Unit, integer: bool) -> Self {
        Self {
            unit,
            integer,
            lower: None,
            upper: None,
        }
    }
    pub fn singleton(number: Number) -> Self {
        Self {
            unit: number.unit,
            integer: number.value.fract() == Fraction::from(0),
            lower: Some((number.value, true)),
            upper: Some((number.value, true)),
        }
    }
    pub fn is_singleton(&self) -> bool {
        matches!((self.lower,self.upper),(Some((lower,true)),Some((upper,true))) if lower==upper)
    }
    pub fn contains(&self, number: Number) -> bool {
        number.unit == self.unit
            && super::finite(&number.value)
            && (!self.integer || number.value.fract() == Fraction::from(0))
            && self.lower.is_none_or(|(bound, inclusive)| {
                number.value > bound || inclusive && number.value == bound
            })
            && self.upper.is_none_or(|(bound, inclusive)| {
                number.value < bound || inclusive && number.value == bound
            })
    }
    pub fn constrained(mut self, operator: &str, number: Number) -> Result<Self, String> {
        if number.unit != self.unit || !super::finite(&number.value) {
            return Err(
                "numeric refinement bounds must be finite and use the annotated type's unit".into(),
            );
        }
        let mut constraint = Self::all(self.unit, false);
        match operator {
            "<" | "<=" => constraint.upper = Some((number.value, operator == "<=")),
            ">" | ">=" => constraint.lower = Some((number.value, operator == ">=")),
            "==" => {
                constraint.lower = Some((number.value, true));
                constraint.upper = constraint.lower;
            }
            _ => return Err("numeric refinements support <, <=, >, >= and ==".into()),
        }
        self = self
            .intersection(&constraint)
            .ok_or("numeric refinement is empty; no value satisfies these bounds")?;
        Ok(self)
    }
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if self.unit != other.unit {
            return None;
        }
        let mut result = Self {
            unit: self.unit,
            integer: self.integer || other.integer,
            lower: self.lower,
            upper: self.upper,
        };
        if let Some(bound) = other.lower
            && result
                .lower
                .is_none_or(|previous| bound.0 > previous.0 || bound.0 == previous.0 && !bound.1)
        {
            result.lower = Some(bound);
        }
        if let Some(bound) = other.upper
            && result
                .upper
                .is_none_or(|previous| bound.0 < previous.0 || bound.0 == previous.0 && !bound.1)
        {
            result.upper = Some(bound);
        }
        // Canonical closed integer bounds also make empty integer intervals decidable.
        if result.integer {
            if let Some((value, inclusive)) = result.lower {
                result.lower = Some((
                    if !inclusive && value.fract() == Fraction::from(0) {
                        value.checked_add(&Fraction::from(1))?
                    } else {
                        value.ceil()
                    },
                    true,
                ));
            }
            if let Some((value, inclusive)) = result.upper {
                result.upper = Some((
                    if !inclusive && value.fract() == Fraction::from(0) {
                        value.checked_sub(&Fraction::from(1))?
                    } else {
                        value.floor()
                    },
                    true,
                ));
            }
        }
        if let (Some(lower), Some(upper)) = (result.lower, result.upper)
            && (lower.0 > upper.0 || lower.0 == upper.0 && !(lower.1 && upper.1))
        {
            return None;
        }
        Some(result)
    }
    pub fn accepts(&self, other: &Self) -> bool {
        self.intersection(other)
            .is_some_and(|intersection| intersection == *other)
    }
    /// Merge only when the union remains one exact interval. In particular,
    /// discrete integer holes never turn into a continuous Number range.
    pub fn union(&self, other: &Self) -> Option<Self> {
        if self.accepts(other) {
            return Some(self.clone());
        }
        if other.accepts(self) {
            return Some(other.clone());
        }
        if self.unit != other.unit || self.integer != other.integer {
            return None;
        }
        for (before, after) in [(self, other), (other, self)] {
            if let (Some((upper, upper_inclusive)), Some((lower, lower_inclusive))) =
                (before.upper, after.lower)
            {
                let separated = if self.integer {
                    upper
                        .checked_add(&Fraction::from(1))
                        .is_some_and(|next| next < lower)
                } else {
                    upper < lower || upper == lower && !(upper_inclusive || lower_inclusive)
                };
                if separated {
                    return None;
                }
            }
        }
        let lower = match (self.lower, other.lower) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 {
                a
            } else if b.0 < a.0 {
                b
            } else {
                (a.0, a.1 || b.1)
            }),
            _ => None,
        };
        let upper = match (self.upper, other.upper) {
            (Some(a), Some(b)) => Some(if a.0 > b.0 {
                a
            } else if b.0 > a.0 {
                b
            } else {
                (a.0, a.1 || b.1)
            }),
            _ => None,
        };
        Some(Self {
            unit: self.unit,
            integer: self.integer,
            lower,
            upper,
        })
    }
    pub fn difference(&self, other: &Self) -> Result<Vec<Self>, String> {
        let Some(common) = self.intersection(other) else {
            return Ok(vec![self.clone()]);
        };
        if other.integer && !self.integer && !common.is_singleton() {
            return Err("subtracting a discrete Int range from Number would require enumerating integer holes; use an Int base or subtract a single value".into());
        }
        let mut output = Vec::new();
        if let Some((value, inclusive)) = common.lower {
            let mut before = Self::all(self.unit, self.integer);
            before.upper = Some((value, !inclusive));
            if let Some(before) = self.intersection(&before) {
                output.push(before);
            }
        }
        if let Some((value, inclusive)) = common.upper {
            let mut after = Self::all(self.unit, self.integer);
            after.lower = Some((value, !inclusive));
            if let Some(after) = self.intersection(&after) {
                output.push(after);
            }
        }
        Ok(output)
    }
}
