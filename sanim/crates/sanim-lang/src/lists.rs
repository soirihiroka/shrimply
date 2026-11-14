//! Typed value edits. Timing is applied by the shared function-call effect mechanism.
use crate::{
    native::Context,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::{List, ListEdit, ListEditResult, Span, Type, Unit, Value};

// imbl's configured vector chunks contain at most 64 entries (4 with its
// small-chunks feature). Charge a conservative copied-path bound, not all
// unchanged values shared by the next persistent version.
pub(crate) const MAX_VECTOR_CHUNK_SIZE: usize = 64;

pub(crate) struct EditDelta {
    pub edit: ListEditResult,
    pub added_bytes: usize,
    pub added_items: usize,
    pub cow_bytes: usize,
}
pub(crate) struct EditReceipt {
    pub module: String,
    pub span: Span,
    pub delta: EditDelta,
}

pub(crate) fn charge_delta(
    delta: &EditDelta,
    bytes: &mut usize,
    items: &mut usize,
) -> Result<(), String> {
    *bytes = bytes
        .checked_sub(delta.added_bytes)
        .and_then(|bytes| bytes.checked_sub(delta.cow_bytes))
        .ok_or(
            "list edit copy-on-write byte budget exceeded; reduce retained updates or list size",
        )?;
    *items = items
        .checked_sub(delta.added_items)
        .ok_or("list edit item budget exceeded")?;
    Ok(())
}

fn copied_path_bytes(len: usize, paths: usize) -> Result<usize, String> {
    let levels = len.max(1).ilog(MAX_VECTOR_CHUNK_SIZE) as usize + 1;
    let leaf = MAX_VECTOR_CHUNK_SIZE.checked_mul(std::mem::size_of::<Value>());
    let branches = MAX_VECTOR_CHUNK_SIZE
        .checked_mul(std::mem::size_of::<usize>())
        .and_then(|bytes| bytes.checked_mul(levels));
    leaf.zip(branches)
        .and_then(|(leaf, branches)| leaf.checked_add(branches))
        .and_then(|bytes| bytes.checked_mul(paths))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<List>()))
        .ok_or_else(|| "list edit copy-on-write budget overflow".into())
}

pub(crate) fn register(builtins: &mut Vec<Builtin>) {
    for (operation, arguments) in [
        ("append", &["items", "value"][..]),
        ("insert", &["items", "index", "value"][..]),
        ("pop", &["items", "index"][..]),
        ("popleft", &["items"][..]),
        ("replace_at", &["items", "index", "value"][..]),
        ("move_item", &["items", "from_index", "to_index"][..]),
    ] {
        builtins.push(Builtin {
            name: operation,
            parameters: arguments
                .iter()
                .map(|name| Parameter {
                    name,
                    ty: if name.ends_with("index") {
                        Type::Numeric(Unit::Number)
                    } else {
                        Type::Unknown
                    },
                    required: !(operation == "pop" && *name == "index"),
                })
                .collect(),
            result: Type::Unknown,
            schedule: false,
            timing: Default::default(),
            content: Default::default(),
            execute: Execution::Access(execute),
            check,
        });
    }
}

fn check(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let Type::List(inner, length) = &actuals[0] else {
        return Err(format!("items must be List<T>, found {}", actuals[0]));
    };
    let inner = if let Some(index) = builtin
        .parameters
        .iter()
        .position(|parameter| parameter.name == "value")
    {
        if **inner == Type::Unknown {
            actuals[index].clone()
        } else if inner.accepts(&actuals[index]) {
            inner.as_ref().clone()
        } else if let Some(common) = inner.join_list_lengths(&actuals[index]) {
            common
        } else {
            return Err(format!(
                "value must match list element type {inner}, found {}",
                actuals[index]
            ));
        }
    } else {
        inner.as_ref().clone()
    };
    let length = match builtin.name {
        "append" | "insert" => length.checked_add(sanim_core::ListLength::Exact(1))?,
        "pop" | "popleft" => match length {
            sanim_core::ListLength::Exact(value) => {
                sanim_core::ListLength::Exact(value.saturating_sub(1))
            }
            _ => sanim_core::ListLength::Unknown,
        },
        _ => *length,
    };
    Ok(Type::List(Box::new(inner), length))
}

pub(crate) fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    context: Context<'_>,
) -> Result<Value, String> {
    let Some(Value::List(items)) = args.get("items") else {
        return Err(format!("{name}: items must be a List<T>"));
    };
    let index = |parameter: &str| -> Result<usize, String> {
        args.get(parameter)
            .ok_or_else(|| format!("{name}: missing '{parameter}'"))?
            .number()
            .and_then(sanim_core::Number::usize)
            .map_err(|reason| {
                format!("{name}: {parameter} must be a nonnegative integer; {reason}")
            })
    };
    let value = || {
        args.get("value")
            .cloned()
            .ok_or_else(|| format!("{name}: missing 'value'"))
    };
    if matches!(name, "pop" | "popleft") && items.is_empty() {
        return Err(format!(
            "{name}: cannot remove an item; the list is empty (length 0)"
        ));
    }
    let operation = match name {
        "append" => ListEdit::Append(value()?),
        "insert" => ListEdit::Insert {
            index: index("index")?,
            value: value()?,
        },
        "pop" => ListEdit::RemoveAt(if args.contains_key("index") {
            index("index")?
        } else {
            items.len() - 1
        }),
        "popleft" => ListEdit::RemoveAt(0),
        "replace_at" => ListEdit::ReplaceAt {
            index: index("index")?,
            value: value()?,
        },
        "move_item" => ListEdit::Move {
            from_index: index("from_index")?,
            to_index: index("to_index")?,
        },
        _ => unreachable!("registered list edit"),
    };
    let edit = items
        .edit(operation)
        .map_err(|reason| format!("{name}: {reason}"))?;
    if edit.output().len() > context.limits.data.max_values {
        return Err(format!(
            "{name}: adding an item would exceed the list value limit {}",
            context.limits.data.max_values
        ));
    }
    let (mut budget_bytes, mut budget_items) = (
        context.limits.data.max_bytes,
        context.limits.data.max_values,
    );
    if let Some(value) = edit.added() {
        sanim_core::validate_value(&edit.output().element_type, value)
            .map_err(|reason| format!("{name}: inserted value is invalid: {reason}"))?;
        sanim_core::value_budget::charge(
            value,
            &mut budget_bytes,
            &mut budget_items,
            context.limits.data.max_depth,
        )?;
    }
    let cow_bytes = if edit.changed() {
        copied_path_bytes(edit.output().len(), if edit.moved() { 4 } else { 2 })?
    } else {
        0
    };
    let output = edit.output().clone();
    *context.list_edit = Some(EditDelta {
        edit,
        added_bytes: context.limits.data.max_bytes - budget_bytes,
        added_items: context.limits.data.max_values - budget_items,
        cow_bytes,
    });
    Ok(Value::List(output))
}
