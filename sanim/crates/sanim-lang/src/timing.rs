//! Native animation defaults and canonical checked-call argument lookup.
use crate::{
    ast::{Expr, ExprKind},
    prelude::Builtin,
};
use sanim_core::{Fraction, Unit, Value};

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub default: Fraction,
    pub easing: sanim_core::Easing,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            default: Fraction::from(0),
            easing: Default::default(),
        }
    }
}
impl Timing {
    pub(crate) fn validate(self, duration: Fraction) -> Result<Fraction, String> {
        if duration < Fraction::from(0) {
            return Err("timeline duration must be nonnegative".into());
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
/// A checked native call, including the implicit receiver of relationship methods.
/// Builtin registration determines method eligibility; there is no second name list.
pub(crate) struct BuiltinCall<'a> {
    pub builtin: &'static Builtin,
    pub receiver: Option<&'a Expr>,
    args: &'a [(Option<String>, Expr)],
}
impl<'a> BuiltinCall<'a> {
    pub(crate) fn argument(&self, index: usize) -> Option<&'a Expr> {
        if index == 0 && self.receiver.is_some() {
            return self.receiver;
        }
        let name = self.builtin.parameters.get(index)?.name;
        self.args
            .iter()
            .find(|(label, _)| label.as_deref() == Some(name))
            .map(|(_, value)| value)
            .or_else(|| {
                self.args
                    .iter()
                    .filter(|(label, _)| label.is_none())
                    .nth(index.checked_sub(usize::from(self.receiver.is_some()))?)
                    .map(|(_, value)| value)
            })
    }
    pub(crate) fn capabilities(
        &self,
        scope: &impl crate::checker::TypeScope,
    ) -> sanim_core::FnCapabilities {
        let actuals = (0..self.builtin.parameters.len()).map(|index| match self.argument(index) {
            None => &sanim_core::Type::Unit,
            Some(argument) => {
                crate::analysis::lookup(argument, scope).unwrap_or(&sanim_core::Type::Unknown)
            }
        });
        self.builtin.capabilities_for_arguments(actuals)
    }
}
pub(crate) fn builtin_call<'a>(
    callee: &'a Expr,
    args: &'a [(Option<String>, Expr)],
    native_methods: &std::collections::BTreeMap<(usize, usize), crate::NativeMethod>,
) -> Option<BuiltinCall<'a>> {
    let (name, receiver) = match &callee.kind {
        ExprKind::Name(name) => (name, None),
        ExprKind::Member(receiver, name) => (name, Some(receiver.as_ref())),
        _ => return None,
    };
    let method = native_methods.get(&(callee.span.start, callee.span.end));
    let builtin =
        crate::prelude::builtin(method.map_or(name.as_str(), |method| method.name.as_str()))?;
    if receiver.is_some() && method.is_none() && builtin.result != sanim_core::Type::Predicate {
        return None;
    }
    Some(BuiltinCall {
        builtin,
        receiver,
        args,
    })
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
