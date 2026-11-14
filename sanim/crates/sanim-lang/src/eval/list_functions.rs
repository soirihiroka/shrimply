//! Persistent list algorithms; callbacks use the same checked invocation path as ordinary calls.
use super::*;
use crate::list_functions::Operation;
use std::collections::BTreeSet;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Number(Fraction),
    String(Arc<str>),
}
struct CallbackVisit {
    index: usize,
    ordinal: usize,
    need_result: bool,
}
impl Key {
    fn new(value: &Value) -> Result<Self, String> {
        if let Value::String(value) = value {
            Ok(Self::String(value.clone()))
        } else {
            Ok(Self::Number(value.number()?.value))
        }
    }
    fn work(&self) -> u64 {
        match self {
            Self::String(value) => value.len().max(1) as u64,
            _ => 1,
        }
    }
}
impl Runtime<'_, '_> {
    fn list_callback(
        &mut self,
        key: &str,
        values: &[Value],
        span: Span,
        visit: CallbackVisit,
    ) -> Result<Value, Diagnostic> {
        if visit.ordinal >= self.limits.max_iterations {
            return Err(self.error("S303",format!("List callback exceeded the {}-visit iteration limit at item index {}; reduce the input or increase the host limit",self.limits.max_iterations,visit.index),span));
        }
        let first = self.reactive.next;
        self.identity
            .push(format!("list{}:{}", self.instance_site(span), visit.index));
        let evaluated = values
            .iter()
            .cloned()
            .map(|value| (None, value))
            .collect::<Vec<_>>();
        let result = self.invoke(key, &evaluated, span, visit.need_result, None);
        self.identity.pop();
        self.reactive.remove_since(first);
        result.map_err(|mut error| {
            error.message = format!(
                "List callback at item index {}: {}",
                visit.index, error.message
            );
            error
        })
    }

    pub(super) fn list_function(
        &mut self,
        builtin: &prelude::Builtin,
        args: &Values,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let definition =
            crate::list_functions::definition(builtin.name).expect("registered list method");
        let Some(Value::List(items)) = args.get("items") else {
            return Err(self.error("S305", "List method requires List<T>", span));
        };
        let discard = self.duration_only
            && self
                .discarded_list_call
                .as_ref()
                .is_some_and(|(module, site)| module == &self.module && *site == span);
        let result_type = if discard {
            Type::Unit
        } else {
            self.loaded
                .iter()
                .find(|module| module.id == self.module)
                .and_then(|module| module.checked.result_types.get(&(span.start, span.end)))
                .map(|ty| ty.substitute(&self.type_bindings))
                .ok_or_else(|| {
                    self.error(
                        "S301",
                        "List call is missing its checked result schema",
                        span,
                    )
                })?
        };
        let list_type = if let Type::List(inner, _) = &result_type {
            Some(Arc::new(inner.as_ref().clone()))
        } else {
            None
        };
        let callback = definition
            .callback
            .and_then(|callback| args.get(callback.name))
            .map(|value| {
                if let Value::Function(key) = value {
                    Ok(key.as_str())
                } else {
                    Err(self.error("S305", "List callback must be a declared function", span))
                }
            })
            .transpose()?;
        if discard {
            if matches!(
                definition.operation,
                Operation::Map | Operation::FlatMap | Operation::Sort | Operation::Unique
            ) {
                if let Some(callback) = callback {
                    let need_result =
                        matches!(definition.operation, Operation::Sort | Operation::Unique);
                    for (index, item) in items.iter().enumerate() {
                        self.charge_work(1, span)?;
                        self.list_callback(
                            callback,
                            std::slice::from_ref(item),
                            span,
                            CallbackVisit {
                                index,
                                ordinal: index,
                                need_result,
                            },
                        )?;
                    }
                }
                return Ok(Value::Unit);
            }
            if definition.callback.is_none() {
                return Ok(Value::Unit);
            }
        }
        let index = |name, default| -> Result<usize, Diagnostic> {
            args.get(name)
                .map(|value| value.number().and_then(Number::usize))
                .transpose()
                .map(|value| value.unwrap_or(default))
                .map_err(|reason| {
                    self.error(
                        "S305",
                        format!(
                            "{}: {name} must be a nonnegative integer; {reason}",
                            builtin.name
                        ),
                        span,
                    )
                })
        };
        let empty = || {
            List::from_values(
                list_type.clone().expect("list output schema"),
                sanim_core::Values::new(),
            )
        };
        use Operation::*;
        if matches!(definition.operation, Take | Drop | Slice) {
            let (start, end) = match definition.operation {
                Take => (0, index("count", items.len())?.min(items.len())),
                Drop => (index("count", items.len())?.min(items.len()), items.len()),
                _ => (
                    index("start", 0)?.min(items.len()),
                    index("end", items.len())?.min(items.len()),
                ),
            };
            self.charge_work(items.len().max(1).ilog2() as u64 + 1, span)?;
            return Ok(Value::List(if start == 0 && end == items.len() {
                items.clone()
            } else if start >= end {
                empty()
            } else {
                let mut shared = items.values.clone();
                List::from_values(items.element_type.clone(), shared.slice(start..end))
            }));
        }
        if matches!(definition.operation, BisectLeft | BisectRight) {
            let mut lo = index("lo", 0)?;
            let mut hi = index("hi", items.len())?;
            if lo > hi || hi > items.len() {
                return Err(self.error(
                    "S305",
                    format!(
                        "{} requires 0 <= lo <= hi <= len; got lo={lo}, hi={hi}, len={}",
                        builtin.name,
                        items.len()
                    ),
                    span,
                ));
            }
            let needle = Key::new(args.get("value").expect("checked search value"))
                .map_err(|reason| self.error("S305", reason, span))?;
            while lo < hi {
                let middle = lo + (hi - lo) / 2;
                let value =
                    Key::new(&items[middle]).map_err(|reason| self.error("S305", reason, span))?;
                self.charge_work(value.work().max(needle.work()), span)?;
                if value < needle || definition.operation == BisectRight && value == needle {
                    lo = middle + 1;
                } else {
                    hi = middle;
                }
            }
            return Ok(Value::Number(Number {
                value: Fraction::from(lo as u64),
                unit: Unit::Number,
            }));
        }
        if definition.operation == Reduce && items.is_empty() {
            return Err(self.error("S305","List.reduce cannot reduce an empty list; use items.fold(initial, callback) to supply an empty-list result",span));
        }
        let mut output = sanim_core::Values::new();
        let mut bytes = self.limits.data.max_bytes;
        let mut values = self.limits.data.max_values;
        let mut shared = sanim_core::value_budget::Budget::default();
        let max_values = self.limits.data.max_values;
        let max_depth = self.limits.data.max_depth;
        let module = self.module.clone();
        let mut append = |value: Value| -> Result<(), Diagnostic> {
            if discard {
                return Ok(());
            }
            if output.len() >= max_values {
                return Err(Diagnostic::new(
                    "S303",
                    format!(
                        "{} output exceeds the list item limit {max_values}",
                        builtin.name
                    ),
                    &module,
                    span,
                ));
            }
            sanim_core::value_budget::charge_with_budget(
                &value,
                &mut bytes,
                &mut values,
                max_depth,
                &mut shared,
            )
            .map_err(|reason| Diagnostic::new("S303", reason, &module, span))?;
            output.push_back(value);
            Ok(())
        };
        if definition.operation == Reverse {
            if items.len() <= 1 {
                return Ok(Value::List(items.clone()));
            }
            for item in items.iter().rev() {
                self.charge_work(1, span)?;
                append(item.clone())?;
            }
            return Ok(Value::List(List::from_values(
                list_type.expect("reversed list schema"),
                output,
            )));
        }
        // The output collector owns a bounded budget, not a borrow of the evaluator.
        let callback_name = callback;
        let initial = if definition.operation == Reduce {
            items.front().cloned()
        } else {
            args.get("initial").cloned()
        };
        let mut accumulator = initial;
        let mut sortable = Vec::new();
        let mut seen = BTreeSet::new();
        let reversed = args
            .get("reverse")
            .map(Value::boolean)
            .transpose()
            .map_err(|reason| self.error("S305", reason, span))?
            .unwrap_or(false);
        let other = args
            .get("other")
            .map(Value::list)
            .transpose()
            .map_err(|reason| self.error("S305", reason, span))?;
        let mut all_unchanged = true;
        for (position, item) in items.iter().enumerate() {
            if definition.operation == Zip && position >= other.expect("checked zip input").len() {
                break;
            }
            if definition.callback.is_some_and(|callback| {
                callback.count == crate::list_functions::CallbackCount::WithoutFirst
            }) && position == 0
            {
                continue;
            }
            self.charge_work(1, span)?;
            let called = if let Some(key) = callback_name {
                let arguments = if matches!(definition.operation, Reduce | Fold | Scan) {
                    vec![
                        accumulator.as_ref().expect("initial accumulator").clone(),
                        item.clone(),
                    ]
                } else {
                    vec![item.clone()]
                };
                let skipped = usize::from(definition.callback.is_some_and(|callback| {
                    callback.count == crate::list_functions::CallbackCount::WithoutFirst
                }));
                Some(self.list_callback(
                    key,
                    &arguments,
                    span,
                    CallbackVisit {
                        index: position,
                        ordinal: position - skipped,
                        need_result: true,
                    },
                )?)
            } else {
                None
            };
            match definition.operation {
                Map => {
                    let value = called.expect("callback result");
                    all_unchanged &= value == *item;
                    append(value)?;
                }
                FlatMap => {
                    let value = called.expect("callback result");
                    let values = value
                        .list()
                        .map_err(|reason| self.error("S305", reason, span))?;
                    for value in values.iter() {
                        append(value.clone())?;
                    }
                }
                Filter => {
                    let keep = called
                        .expect("callback result")
                        .boolean()
                        .map_err(|reason| self.error("S305", reason, span))?;
                    all_unchanged &= keep;
                    if keep {
                        append(item.clone())?;
                    }
                }
                Reduce | Fold | Scan => {
                    accumulator = called;
                    if definition.operation == Scan {
                        append(accumulator.as_ref().expect("callback accumulator").clone())?;
                    }
                }
                Any | All | Find | Position => {
                    let selected = called
                        .expect("predicate result")
                        .boolean()
                        .map_err(|reason| self.error("S305", reason, span))?;
                    match definition.operation {
                        Any if selected => return Ok(Value::Bool(true)),
                        All if !selected => return Ok(Value::Bool(false)),
                        Find if selected => return Ok(Value::Some(Arc::new(item.clone()))),
                        Position if selected => {
                            return Ok(Value::Some(Arc::new(Value::Number(Number {
                                value: Fraction::from(position as u64),
                                unit: Unit::Number,
                            }))));
                        }
                        _ => {}
                    }
                }
                Enumerate => append(Value::Record(
                    [
                        (
                            "index".to_string(),
                            Value::Number(Number {
                                value: Fraction::from(position as u64),
                                unit: Unit::Number,
                            }),
                        ),
                        ("value".to_string(), item.clone()),
                    ]
                    .into_iter()
                    .collect(),
                ))?,
                Zip => append(Value::Record(
                    [
                        ("left".to_string(), item.clone()),
                        (
                            "right".to_string(),
                            other.expect("zip input")[position].clone(),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ))?,
                Sort | Unique => {
                    let key = Key::new(called.as_ref().unwrap_or(item)).map_err(|reason| {
                        self.error("S305", format!("{} key: {reason}", builtin.name), span)
                    })?;
                    self.charge_work(
                        key.work()
                            .saturating_mul(items.len().max(1).ilog2() as u64 + 1),
                        span,
                    )?;
                    if definition.operation == Sort {
                        sortable.push((key, item.clone()));
                    } else if seen.insert(key) {
                        append(item.clone())?;
                    } else {
                        all_unchanged = false;
                    }
                }
                _ => unreachable!("early-return list operation"),
            }
        }
        match definition.operation {
            Reduce | Fold => {
                return Ok(if discard {
                    Value::Unit
                } else {
                    accumulator.expect("nonempty reduction or explicit fold initial")
                });
            }
            Any => return Ok(Value::Bool(false)),
            All => return Ok(Value::Bool(true)),
            Find | Position => return Ok(Value::None),
            Sort => {
                sortable.sort_by(|(a, _), (b, _)| if reversed { b.cmp(a) } else { a.cmp(b) });
                for (_, item) in sortable {
                    append(item)?;
                }
            }
            _ => {}
        }
        if discard {
            return Ok(Value::Unit);
        }
        if matches!(definition.operation, Map | Filter | Unique)
            && all_unchanged
            && list_type.as_deref() == Some(items.element_type.as_ref())
        {
            return Ok(Value::List(items.clone()));
        }
        Ok(Value::List(List::from_values(
            list_type.expect("list result"),
            output,
        )))
    }
}
