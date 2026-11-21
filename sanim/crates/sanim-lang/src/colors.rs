//! The native color registry is shared with retained reactive playback.
use crate::{
    native,
    prelude::{Builtin, parameter},
};
use sanim_core::{Type, math::color::Op};
use std::sync::OnceLock;

fn declaration(operation: Op) -> Builtin {
    let (parameters, result) = operation.signature();
    Builtin::pure(
        operation.name(),
        parameters
            .into_iter()
            .map(|(name, ty, required)| parameter(name, ty, required))
            .collect(),
        result,
        native::configuration,
    )
}
pub(crate) fn register(output: &mut Vec<Builtin>) {
    output.extend(
        Op::all()
            .filter(|operation| !operation.is_method())
            .map(declaration),
    );
}
pub(crate) fn methods() -> &'static [Builtin] {
    static METHODS: OnceLock<Vec<Builtin>> = OnceLock::new();
    METHODS.get_or_init(|| {
        Op::all()
            .filter(|operation| operation.is_method())
            .map(declaration)
            .collect()
    })
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    methods()
        .iter()
        .filter(move |builtin| builtin.parameters[0].ty.accepts(receiver))
}
