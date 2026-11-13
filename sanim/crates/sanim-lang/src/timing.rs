//! One timeline contract for native calls, user calls, and duration-only evaluation.
use crate::{
    ast::{Expr, ExprKind},
    prelude::Builtin,
};
use sanim_core::{Fraction, Number, TimelineEffect, Unit, Value};

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub default: Fraction,
    pub positive: bool,
    pub parameter: usize,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            default: Fraction::from(0),
            positive: false,
            parameter: 0,
        }
    }
}
impl Timing {
    pub(crate) fn validate(self, duration: Fraction) -> Result<Fraction, String> {
        if duration < Fraction::from(0) || self.positive && duration == Fraction::from(0) {
            return Err(if self.positive {
                "timeline duration must be positive"
            } else {
                "timeline duration must be nonnegative"
            }
            .into());
        }
        sanim_core::math::timeline_end(Fraction::from(0), duration)?;
        Ok(duration)
    }
    pub(crate) fn value(self, value: Option<&Value>) -> Result<Fraction, String> {
        let duration = if let Some(value) = value {
            let number = value.number()?;
            if number.unit != Unit::Duration {
                return Err(
                    "timeline duration must have Duration units, such as 300ms or 2s".into(),
                );
            }
            number.value
        } else {
            return Ok(self.default);
        };
        self.validate(duration)
    }
}
impl Builtin {
    pub(crate) fn duration_argument<'a, T>(
        &self,
        args: &'a [(Option<String>, T)],
    ) -> Option<&'a T> {
        self.duration_argument_after(args, 0)
    }
    fn duration_argument_after<'a, T>(
        &self,
        args: &'a [(Option<String>, T)],
        implicit: usize,
    ) -> Option<&'a T> {
        if let Some((_, value)) = args
            .iter()
            .find(|(name, _)| name.as_deref() == Some("duration"))
        {
            return Some(value);
        }
        let parameter = self.timing.parameter.checked_sub(implicit)?;
        if args.len() <= parameter {
            return None;
        }
        args.iter()
            .filter(|(name, _)| name.is_none())
            .nth(parameter)
            .map(|(_, value)| value)
    }
    pub(crate) fn easing_argument<'a, T>(&self, args: &'a [(Option<String>, T)]) -> Option<&'a T> {
        self.easing_argument_after(args, 0)
    }
    fn easing_argument_after<'a, T>(
        &self,
        args: &'a [(Option<String>, T)],
        implicit: usize,
    ) -> Option<&'a T> {
        if let Some((_, value)) = args
            .iter()
            .find(|(name, _)| name.as_deref() == Some("ease"))
        {
            return Some(value);
        }
        let index = self
            .parameters
            .iter()
            .position(|p| p.name == "ease")?
            .checked_sub(implicit)?;
        args.iter()
            .filter(|(name, _)| name.is_none())
            .nth(index)
            .map(|(_, value)| value)
    }
    pub(crate) fn call_schedules(&self, args: &[(Option<String>, Expr)]) -> bool {
        self.call_schedules_after(args, 0)
    }
    fn call_schedules_after(&self, args: &[(Option<String>, Expr)], implicit: usize) -> bool {
        self.schedule
            || self
                .duration_argument_after(args, implicit)
                .is_some_and(|expr| !known_zero(expr))
    }
    /// The generic signature depends on its duration argument; omitted arguments resolve to default.
    pub fn timeline_duration(&self) -> TimelineEffect {
        TimelineEffect::Dependent
    }
}

/// A checked native call, including the implicit receiver of relationship methods.
/// Builtin registration determines method eligibility; there is no second name list.
pub(crate) struct BuiltinCall<'a> {
    pub builtin: &'static Builtin,
    pub receiver: Option<&'a Expr>,
    args: &'a [(Option<String>, Expr)],
}
impl<'a> BuiltinCall<'a> {
    pub(crate) fn easing_argument(&self) -> Option<&'a Expr> {
        self.builtin
            .easing_argument_after(self.args, usize::from(self.receiver.is_some()))
    }

    pub fn duration_argument(&self) -> Option<&'a Expr> {
        self.builtin
            .duration_argument_after(self.args, usize::from(self.receiver.is_some()))
    }
    pub fn schedules(&self) -> bool {
        self.builtin
            .call_schedules_after(self.args, usize::from(self.receiver.is_some()))
    }
}
pub(crate) fn builtin_call<'a>(
    callee: &'a Expr,
    args: &'a [(Option<String>, Expr)],
) -> Option<BuiltinCall<'a>> {
    let (name, receiver) = match &callee.kind {
        ExprKind::Name(name) => (name, None),
        ExprKind::Member(receiver, name) => (name, Some(receiver.as_ref())),
        _ => return None,
    };
    let builtin = crate::prelude::builtin(name)?;
    if receiver.is_some() && builtin.result != sanim_core::Type::Predicate {
        return None;
    }
    Some(BuiltinCall {
        builtin,
        receiver,
        args,
    })
}
pub(crate) fn known_zero(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::Number(text) if Number::parse(text).is_ok_and(|n| n.unit == Unit::Duration && n.value == Fraction::from(0)))
}
pub(crate) fn envelope<T>(args: &[(Option<String>, T)]) -> Option<&T> {
    args.iter()
        .find(|(name, _)| name.as_deref() == Some("duration"))
        .map(|(_, value)| value)
}

/// Shared optional easing control. It affects value interpolation, never time arithmetic.
pub(crate) fn easing(value: Option<&Value>) -> Result<sanim_core::Easing, String> {
    value
        .map(Value::string)
        .transpose()?
        .map(sanim_core::Easing::parse)
        .transpose()
        .map(|value| value.unwrap_or_default())
}
pub(crate) fn ease_argument<T>(args: &[(Option<String>, T)]) -> Option<&T> {
    args.iter()
        .find(|(name, _)| name.as_deref() == Some("ease"))
        .map(|(_, value)| value)
}
