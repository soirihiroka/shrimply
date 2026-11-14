//! One declaration registry for typed persistent-list methods and their callback effects.
use crate::prelude::{Builtin, Execution, Parameter};
use sanim_core::{Type, Unit};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Map,
    Filter,
    FlatMap,
    Reduce,
    Fold,
    Scan,
    Any,
    All,
    Find,
    Position,
    Enumerate,
    Zip,
    Take,
    Drop,
    Slice,
    Reverse,
    Sort,
    Unique,
    BisectLeft,
    BisectRight,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallbackCount {
    All,
    WithoutFirst,
    EarlyExit,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Callback {
    pub name: &'static str,
    pub index: usize,
    pub arity: usize,
    pub count: CallbackCount,
}
pub(crate) struct Definition {
    pub builtin: Builtin,
    pub operation: Operation,
    pub callback: Option<Callback>,
}
type Spec = (
    &'static str,
    Operation,
    &'static [(&'static str, bool)],
    Option<Callback>,
);
pub(crate) fn definitions() -> &'static [Definition] {
    static REGISTRY: OnceLock<Vec<Definition>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        use CallbackCount::{EarlyExit, WithoutFirst};
        use Operation::*;
        let callback = |name, index, arity, count| {
            Some(Callback {
                name,
                index,
                arity,
                count,
            })
        };
        let specs: &[Spec] = &[
            (
                "List.map",
                Map,
                &[("callback", true)],
                callback("callback", 1, 1, CallbackCount::All),
            ),
            (
                "List.filter",
                Filter,
                &[("callback", true)],
                callback("callback", 1, 1, CallbackCount::All),
            ),
            (
                "List.flat_map",
                FlatMap,
                &[("callback", true)],
                callback("callback", 1, 1, CallbackCount::All),
            ),
            (
                "List.reduce",
                Reduce,
                &[("callback", true)],
                callback("callback", 1, 2, WithoutFirst),
            ),
            (
                "List.fold",
                Fold,
                &[("initial", true), ("callback", true)],
                callback("callback", 2, 2, CallbackCount::All),
            ),
            (
                "List.scan",
                Scan,
                &[("initial", true), ("callback", true)],
                callback("callback", 2, 2, CallbackCount::All),
            ),
            (
                "List.any",
                Any,
                &[("callback", true)],
                callback("callback", 1, 1, EarlyExit),
            ),
            (
                "List.all",
                Operation::All,
                &[("callback", true)],
                callback("callback", 1, 1, EarlyExit),
            ),
            (
                "List.find",
                Find,
                &[("callback", true)],
                callback("callback", 1, 1, EarlyExit),
            ),
            (
                "List.position",
                Position,
                &[("callback", true)],
                callback("callback", 1, 1, EarlyExit),
            ),
            ("List.enumerate", Enumerate, &[], None),
            ("List.zip", Zip, &[("other", true)], None),
            ("List.take", Take, &[("count", true)], None),
            ("List.drop", Drop, &[("count", true)], None),
            (
                "List.slice",
                Slice,
                &[("start", false), ("end", false)],
                None,
            ),
            ("List.reverse", Reverse, &[], None),
            (
                "List.sort",
                Sort,
                &[("key", false), ("reverse", false)],
                callback("key", 1, 1, CallbackCount::All),
            ),
            (
                "List.unique",
                Unique,
                &[("key", false)],
                callback("key", 1, 1, CallbackCount::All),
            ),
            (
                "List.bisect_left",
                BisectLeft,
                &[("value", true), ("lo", false), ("hi", false)],
                None,
            ),
            (
                "List.bisect_right",
                BisectRight,
                &[("value", true), ("lo", false), ("hi", false)],
                None,
            ),
        ];
        specs
            .iter()
            .map(|(name, operation, arguments, callback)| {
                let mut parameters = vec![Parameter {
                    name: "items",
                    ty: Type::List(Box::new(Type::Unknown), sanim_core::ListLength::Unknown),
                    required: true,
                }];
                parameters.extend(arguments.iter().map(|(name, required)| Parameter {
                    name,
                    ty: match *name {
                        "other" => {
                            Type::List(Box::new(Type::Unknown), sanim_core::ListLength::Unknown)
                        }
                        "count" | "start" | "end" | "lo" | "hi" => Type::Numeric(Unit::Number),
                        "reverse" => Type::Bool,
                        _ => Type::Unknown,
                    },
                    required: *required,
                }));
                let mut builtin = Builtin {
                    name,
                    parameters,
                    result: Type::Unknown,
                    schedule: false,
                    timing: Default::default(),
                    content: Default::default(),
                    execute: Execution::List,
                    check: crate::native_types::fixed,
                };
                crate::prelude::finalize(&mut builtin);
                Definition {
                    builtin,
                    operation: *operation,
                    callback: *callback,
                }
            })
            .collect()
    })
}
pub(crate) fn definition(name: &str) -> Option<&'static Definition> {
    definitions()
        .iter()
        .find(|definition| definition.builtin.name == name)
}
pub(crate) fn builtin(name: &str) -> Option<&'static Builtin> {
    definition(name).map(|definition| &definition.builtin)
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    let supported = matches!(receiver, Type::List(_, _));
    definitions()
        .iter()
        .filter(move |_| supported)
        .map(|definition| &definition.builtin)
}
pub(crate) fn callback_argument<'a>(
    call: &crate::timing::BuiltinCall<'a>,
) -> Option<&'a crate::ast::Expr> {
    let callback = definition(call.builtin.name)?.callback?;
    call.argument(callback.index)
}
pub(crate) fn callback_type<'a>(
    call: &crate::timing::BuiltinCall<'_>,
    env: &'a crate::checker::Environment,
) -> Option<&'a Type> {
    crate::effects::lookup(callback_argument(call)?, env)
}
pub(crate) fn ordered(ty: &Type) -> bool {
    *ty == Type::Unknown || sanim_core::math::operations::binary_type("<", ty, ty).is_ok()
}
pub(crate) fn record(fields: impl IntoIterator<Item = (&'static str, Type)>) -> Type {
    Type::Record(
        fields
            .into_iter()
            .map(|(name, ty)| (name.into(), ty))
            .collect(),
    )
}
pub(crate) fn constant_count(
    expression: Option<&crate::ast::Expr>,
    default: Option<usize>,
) -> Option<usize> {
    let Some(expression) = expression else {
        return default;
    };
    let number = crate::effects::constant(expression, &std::collections::BTreeMap::new())?
        .number()
        .ok()?;
    if number.unit == Unit::Number {
        number.usize().ok()
    } else {
        None
    }
}
