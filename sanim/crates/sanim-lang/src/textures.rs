//! The typed registry adapts the same pure operations used by retained sampling.
use crate::{
    native,
    prelude::{Builtin, parameter},
};
use sanim_core::{Type, code::values::Native, math::texture::Op};
use std::sync::OnceLock;
fn builtin(operation: Op) -> Builtin {
    let native = Native::Texture(operation);
    let (parameters, result) = native.signature();
    let mut builtin = Builtin::pure(
        native.name(),
        parameters
            .into_iter()
            .map(|(name, ty, required)| parameter(name, ty, required))
            .collect(),
        result,
        native::configuration,
    );
    builtin.check = check;
    builtin
}
fn check(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let actuals = builtin
        .parameters
        .iter()
        .zip(actuals)
        .filter(|(_, ty)| **ty != Type::Unit)
        .map(|(parameter, ty)| (parameter.name.to_string(), ty.clone()))
        .collect();
    Native::from_name(builtin.name)
        .expect("registered texture operation")
        .check(&actuals)
}
pub(crate) fn register(out: &mut Vec<Builtin>) {
    out.push(builtin(Op::Texture));
}
pub(crate) fn methods() -> &'static [Builtin] {
    static METHODS: OnceLock<Vec<Builtin>> = OnceLock::new();
    METHODS.get_or_init(|| {
        Op::ALL
            .into_iter()
            .filter(|operation| *operation != Op::Texture)
            .map(builtin)
            .collect()
    })
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    methods().iter().filter(move |method| {
        method
            .parameters
            .first()
            .is_some_and(|parameter| parameter.ty.accepts(receiver))
    })
}
