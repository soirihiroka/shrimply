use crate::{
    native::Context,
    native_types,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::{Type, Value};
use std::sync::Arc;

pub(crate) fn expected(ty: Option<&Type>) -> bool {
    match ty {
        Some(Type::Template) => true,
        Some(Type::Union(types)) => {
            !types.contains(&Type::String) && types.iter().any(|ty| expected(Some(ty)))
        }
        _ => false,
    }
}
pub(crate) fn register(output: &mut Vec<Builtin>) {
    for (name, parameter, ty, result) in [
        ("Sql", "template", Type::Template, Type::Sql),
        ("SqlIdent", "name", Type::String, Type::SqlIdent),
    ] {
        output.push(Builtin {
            name,
            parameters: vec![Parameter {
                name: parameter,
                ty,
                required: true,
            }],
            result,
            schedule: false,
            timing: Default::default(),
            content: Default::default(),
            execute: Execution::Access(execute),
            check: native_types::fixed,
        });
    }
}
fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    match name {
        "Sql" => {
            let Some(Value::Template(template)) = args.get("template") else {
                return Err("Sql requires a structured Template; pass a quoted literal or annotate a reusable value as Template".into());
            };
            sanim_core::sql::Sql::compile(
                template,
                context.limits.data.max_bytes,
                context.limits.data.max_values,
            )
            .map(|sql| Value::Sql(Arc::new(sql)))
        }
        "SqlIdent" => {
            sanim_core::sql::identifier(args.get("name").ok_or("SqlIdent requires name")?.string()?)
                .map(Value::SqlIdent)
        }
        _ => Err("unknown template constructor".into()),
    }
}
