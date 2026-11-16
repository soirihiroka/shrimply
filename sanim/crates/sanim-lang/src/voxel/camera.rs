//! Configuration declarations come from the same operation schema as retained evaluation.
use super::{declaration, parameter};
use crate::{
    native,
    prelude::{Builtin, Execution},
};
use sanim_core::{Type, code::values::Native, voxel::camera::Op};

fn builtin(operation: Op) -> Builtin {
    let native = Native::Camera(operation);
    let (parameters, result) = native.signature();
    let mut builtin = declaration(
        native.name(),
        parameters
            .into_iter()
            .map(|(name, ty, required)| parameter(name, ty, required))
            .collect(),
        result,
    );
    builtin.execute = Execution::Native(native::configuration);
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
        .expect("registered camera operation")
        .check(&actuals)
}
pub(super) fn register(out: &mut Vec<Builtin>) {
    out.extend(Op::all().filter(|op| !op.is_method()).map(builtin));
}
pub(super) fn methods() -> impl Iterator<Item = Builtin> {
    Op::all().filter(|op| op.is_method()).map(builtin)
}
