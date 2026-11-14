//! Higher-order list calls are checked against declared callable signatures, not sampled values.
use super::*;
use crate::list_functions::{Operation, ordered, record};
use sanim_core::ListLength;

impl Checker<'_> {
    pub(super) fn list_function(
        &mut self,
        expr: &Expr,
        builtin: &prelude::Builtin,
        args: &[(Option<String>, Expr)],
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let definition =
            crate::list_functions::definition(builtin.name).expect("registered list operation");
        let parameters = builtin
            .parameters
            .iter()
            .map(|p| (p.name, p.ty.clone(), p.required))
            .collect::<Vec<_>>();
        let ordered_args = self.arguments(args, &parameters, expr.span)?;
        let mut actuals = Vec::with_capacity(ordered_args.len());
        for (parameter, value) in builtin.parameters.iter().zip(&ordered_args) {
            actuals.push(if let Some(value)=value {
                let ty=self.expression(value,env,None,variables)?;
                if definition.callback.is_none_or(|callback|callback.name!=parameter.name) {
                    if ty.contains_function() { return Err(self.error("S111","only the declared callback parameter accepts a function; other List arguments must be values",value.span)); }
                    self.expect(&parameter.ty,&ty,value.span)?;
                } else if !direct_callee(value) || ty.signatures().is_empty() {
                    return Err(self.error("S106",format!("{} requires a declared function name, such as items.map(double); found {ty}",parameter.name),value.span));
                }
                ty
            } else { Type::Unit });
        }
        let Type::List(inner, input_length) = &actuals[0] else {
            return Err(self.error("S106", "List method receiver must be List<T>", expr.span));
        };
        let input_length = *input_length;
        let inner = inner.as_ref().clone();
        let argument = |name| {
            builtin
                .parameters
                .iter()
                .position(|parameter| parameter.name == name)
                .map(|index| &actuals[index])
        };
        let initial = argument("initial")
            .cloned()
            .unwrap_or_else(|| inner.clone());
        let seed = builtin
            .parameters
            .iter()
            .position(|parameter| parameter.name == "initial")
            .and_then(|index| ordered_args[index])
            .and_then(|value| match &value.kind {
                ExprKind::String(value) => Some(Type::StringLiteral(value.clone().into())),
                _ => super::clauses::literal_number(value).map(|number| {
                    Type::RefinedNumeric(sanim_core::math::domain::NumericDomain::singleton(number))
                }),
            })
            .unwrap_or_else(|| initial.clone());
        let accumulating = matches!(
            definition.operation,
            Operation::Reduce | Operation::Fold | Operation::Scan
        );
        let mut accumulator_types = Vec::new();
        let callback = if let Some(callback) = definition
            .callback
            .filter(|callback| ordered_args[callback.index].is_some())
        {
            let expression = ordered_args[callback.index].expect("present callback");
            if expression.qualified_name().is_some_and(|name| {
                self.schemas.contains_key(&name)
                    || name.rsplit_once('.').is_some_and(|(parent, _)| {
                        matches!(self.schemas.get(parent), Some(Type::Enum { .. }))
                    })
            }) {
                return Err(self.error("S106","List callbacks must be declared fn helpers; wrap the type constructor in a function",expression.span));
            }
            let arguments = if callback.arity == 1 {
                vec![inner.clone()]
            } else {
                vec![seed.clone(), inner.clone()]
            };
            let expected_result = match definition.operation {
                Operation::Filter
                | Operation::Any
                | Operation::All
                | Operation::Find
                | Operation::Position => Some(Type::Bool),
                Operation::Reduce | Operation::Fold | Operation::Scan => None,
                Operation::Map => expected.and_then(|ty| {
                    if let Type::List(inner, _) = ty {
                        Some(inner.as_ref().clone())
                    } else {
                        None
                    }
                }),
                Operation::FlatMap => expected.cloned(),
                _ => None,
            };
            let mut results = Vec::new();
            let mut substitutions = None;
            let mut reasons = Vec::new();
            let mut schedules = false;
            let mut mutates = false;
            let mut actual_arguments = arguments.iter().cloned().map(Some).collect::<Vec<_>>();
            if accumulating {
                actual_arguments[0] = None;
            }
            let mut checked_clauses = Vec::new();
            for signature in
                super::clauses::candidates(actuals[callback.index].signatures(), &actual_arguments)
            {
                let checked = (|| -> Result<(Type, BTreeMap<String, Type>), Diagnostic> {
                    let mut bindings = BTreeMap::new();
                    for (index, (parameter, actual)) in
                        signature.parameters.iter().zip(&arguments).enumerate()
                    {
                        if parameter.kind == BindingKind::Sig {
                            return Err(self.error("S106","List callbacks receive snapshot values; use a let or mut parameter instead of sig",expression.span));
                        }
                        infer(&parameter.ty, actual, &mut bindings, self.type_budget)
                            .map_err(|reason| self.error("S106", reason, expression.span))?;
                        let wanted = self.substitute(&parameter.ty, &bindings, expression.span)?;
                        if !(accumulating && index == 0
                            || *actual == Type::Unknown && input_length == ListLength::Exact(0)
                            || actuals[callback.index].signatures().len() > 1
                                && wanted.overlaps(actual))
                        {
                            self.expect(&wanted, actual, expression.span)?;
                        }
                    }
                    if let Some(expected) = &expected_result {
                        infer(&signature.result, expected, &mut bindings, self.type_budget)
                            .map_err(|reason| self.error("S106", reason, expression.span))?;
                    }
                    self.complete_generics(signature, &mut bindings, expression.span)?;
                    let result = self.substitute(&signature.result, &bindings, expression.span)?;
                    if let Some(expected) = &expected_result {
                        self.expect(expected, &result, expression.span)?;
                    }
                    Ok((result, bindings))
                })();
                match checked {
                    Ok((result, bindings)) => {
                        let domains = signature
                            .parameters
                            .iter()
                            .take(arguments.len())
                            .map(|parameter| {
                                self.substitute(&parameter.ty, &bindings, expression.span)
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        checked_clauses.push((signature, result, bindings, domains));
                    }
                    Err(error) => reasons.push(error.message),
                }
            }
            // Follow accumulator transitions without evaluating callbacks. A clause can
            // return another clause's input domain; only their combined contract must close.
            let mut reachable = seed.clone();
            let mut domains = Vec::new();
            loop {
                let mut progressed = false;
                let mut pending = Vec::new();
                for (signature, result, bindings, parameters) in checked_clauses {
                    if accumulating && !parameters[0].overlaps(&reachable) {
                        pending.push((signature, result, bindings, parameters));
                        continue;
                    }
                    if let Some(previous) = &substitutions
                        && previous != &bindings
                    {
                        return Err(self.error("S106","callback clauses infer conflicting generic substitutions; use one compatible generic schema across clauses",expression.span));
                    }
                    substitutions = Some(bindings);
                    if accumulating {
                        accumulator_types.push(parameters[0].clone());
                        reachable = Type::union([reachable, result.clone()])
                            .map_err(|reason| self.error("S106", reason, expression.span))?;
                    }
                    domains.push(parameters);
                    results.push(result);
                    schedules |= signature.schedules;
                    mutates |= signature.mutates;
                    progressed = true;
                }
                checked_clauses = pending;
                if !accumulating || !progressed || checked_clauses.is_empty() {
                    break;
                }
            }
            if results.is_empty() {
                return Err(self.error(
                    "S106",
                    format!(
                        "{} callback does not accept the list's {} arguments: {}",
                        builtin.name,
                        arguments
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        if reasons.is_empty() {format!("expected a declared callback accepting {} positional arguments; found {}",arguments.len(),actuals[callback.index])} else {reasons.join("; ")}
                    ),
                    expression.span,
                ));
            }
            if input_length != ListLength::Exact(0) {
                let mut covered_arguments = arguments.clone();
                if accumulating {
                    let contract = Type::union(accumulator_types.iter().cloned())
                        .map_err(|reason| self.error("S106", reason, expression.span))?;
                    if !contract.accepts(&reachable) {
                        return Err(self.error("S106", format!("{} accumulator contract is not closed: reachable values {reachable} must all be accepted by {contract}",builtin.name),expression.span));
                    }
                    covered_arguments[0] = contract;
                }
                if !callback_coverage(&domains, &covered_arguments, self.type_budget)
                    .map_err(|reason| self.error("S106", reason, expression.span))?
                {
                    return Err(self.error("S106",format!("{} callback clauses do not cover every combination of argument types {}; add clauses accepting the missing cases or use one helper with a complete input contract",builtin.name,covered_arguments.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")),expression.span));
                }
            }
            self.call_types.insert(
                (expr.span.start, expr.span.end),
                self.copy_environment(&substitutions.unwrap_or_default(), expr.span)?,
            );
            if let Some(function) = &self.function {
                if schedules {
                    self.effects.insert(function.clone());
                }
                if mutates {
                    self.mutations.insert(function.clone());
                }
                if let ExprKind::Name(name) = &expression.kind {
                    self.calls
                        .entry(function.clone())
                        .or_default()
                        .insert(name.clone());
                }
            }
            Some(
                Type::union(results)
                    .map_err(|reason| self.error("S106", reason, expression.span))?,
            )
        } else {
            None
        };
        use Operation::*;
        let count = |name, default: Option<usize>| {
            let at = builtin
                .parameters
                .iter()
                .position(|parameter| parameter.name == name)?;
            crate::list_functions::constant_count(ordered_args[at], default)
        };
        let length = match definition.operation {
            Map | Scan | Enumerate | Reverse | Sort => input_length,
            Filter => {
                if input_length == ListLength::Exact(0) {
                    input_length
                } else {
                    ListLength::Unknown
                }
            }
            Unique => {
                if matches!(input_length, ListLength::Exact(0 | 1)) {
                    input_length
                } else {
                    ListLength::Unknown
                }
            }
            Zip => {
                if let Type::List(_, other) = argument("other").expect("zip schema") {
                    input_length.min(*other)
                } else {
                    unreachable!("checked zip list")
                }
            }
            Take => input_length
                .min(count("count", None).map_or(ListLength::Unknown, ListLength::Exact)),
            Drop => match (input_length, count("count", None)) {
                (ListLength::Exact(length), Some(count)) => {
                    ListLength::Exact(length.saturating_sub(count))
                }
                (ListLength::Exact(0), _) => ListLength::Exact(0),
                _ => ListLength::Unknown,
            },
            Slice => {
                let input = if let ListLength::Exact(length) = input_length {
                    Some(length)
                } else {
                    None
                };
                match (input, count("start", Some(0)), count("end", input)) {
                    (Some(length), Some(start), Some(end)) => {
                        ListLength::Exact(end.min(length).saturating_sub(start.min(length)))
                    }
                    (Some(0), _, _) | (_, _, Some(0)) => ListLength::Exact(0),
                    (_, Some(start), Some(end)) if start >= end => ListLength::Exact(0),
                    _ => ListLength::Unknown,
                }
            }
            _ => ListLength::Unknown,
        };
        let list = |ty| Type::List(Box::new(ty), length);
        let accumulator = if accumulator_types.is_empty() {
            initial.clone()
        } else {
            Type::union(accumulator_types)
                .map_err(|reason| self.error("S106", reason, expr.span))?
        };
        let result = match definition.operation {
            Map => list(callback.expect("required callback")),
            FlatMap => {
                let alternatives = match callback.expect("required callback") {
                    Type::Union(types) => types,
                    ty => vec![ty],
                };
                let mut elements = Vec::with_capacity(alternatives.len());
                let mut each = None;
                for alternative in alternatives {
                    let Type::List(inner, length) = alternative else {
                        return Err(self.error(
                            "S106",
                            format!("flat_map callback must return List<U>, found {alternative}"),
                            expr.span,
                        ));
                    };
                    each = Some(each.map_or(length, |previous| {
                        if previous == length {
                            length
                        } else {
                            ListLength::Unknown
                        }
                    }));
                    elements.push(*inner);
                }
                Type::List(
                    Box::new(
                        Type::union(elements)
                            .map_err(|reason| self.error("S106", reason, expr.span))?,
                    ),
                    input_length
                        .checked_mul(each.unwrap_or(ListLength::Unknown))
                        .map_err(|reason| self.error("S106", reason, expr.span))?,
                )
            }
            Filter | Take | Drop | Slice | Reverse => list(inner.clone()),
            Reduce => accumulator,
            Fold => {
                if input_length == ListLength::Exact(0) {
                    if let (Type::List(_, length), Type::List(inner, _)) = (&initial, &accumulator)
                    {
                        Type::List(inner.clone(), *length)
                    } else {
                        initial
                    }
                } else {
                    accumulator
                }
            }
            Scan => list(accumulator),
            Any | All => Type::Bool,
            Find => Type::Option(Box::new(inner.clone())),
            Position => Type::Option(Box::new(Type::Numeric(Unit::Number))),
            Enumerate => list(record([
                ("index", Type::Numeric(Unit::Number)),
                ("value", inner.clone()),
            ])),
            Zip => {
                let Type::List(other, _) = argument("other").expect("required other") else {
                    unreachable!("checked list argument")
                };
                list(record([
                    ("left", inner.clone()),
                    ("right", other.as_ref().clone()),
                ]))
            }
            Sort | Unique => {
                let key = callback.as_ref().unwrap_or(&inner);
                if !ordered(key) {
                    return Err(self.error("S106",format!("{} needs ordered numeric or String keys, found {key}; use key= a declared projection function for records",builtin.name),expr.span));
                }
                list(inner.clone())
            }
            BisectLeft | BisectRight => {
                if !ordered(&inner) {
                    return Err(self.error("S106",format!("{} requires an ascending list of ordered numeric or String values; found {inner}",builtin.name),expr.span));
                }
                let actual = argument("value").expect("required search value");
                if inner != Type::Unknown {
                    self.expect(&inner, actual, expr.span)?;
                }
                Type::Numeric(Unit::Number)
            }
        };
        self.result_types.insert(
            (expr.span.start, expr.span.end),
            self.copy_type(&result, expr.span)?,
        );
        Ok(result)
    }
}

/// Prove coverage of the argument product, rather than independent parameter unions.
/// Explicit alternatives distribute; numeric intervals use canonical union normalization.
fn callback_coverage(
    domains: &[Vec<Type>],
    actual: &[Type],
    budget: &std::cell::Cell<usize>,
) -> Result<bool, String> {
    let remaining = budget
        .get()
        .checked_sub(1)
        .ok_or("callback coverage exceeds the type complexity budget")?;
    budget.set(remaining);
    if domains.iter().any(|domain| {
        domain
            .iter()
            .zip(actual)
            .all(|(expected, actual)| expected.accepts(actual))
    }) {
        return Ok(true);
    }
    if let Some((index, Type::Union(alternatives))) = actual
        .iter()
        .enumerate()
        .find(|(_, ty)| matches!(ty, Type::Union(_)))
    {
        for alternative in alternatives {
            let mut remaining = budget.get();
            for ty in actual {
                sanim_core::type_budget::charge(ty, &mut remaining)?;
            }
            sanim_core::type_budget::charge(alternative, &mut remaining)?;
            budget.set(remaining);
            let mut branch = actual.to_vec();
            branch[index] = alternative.clone();
            if !callback_coverage(domains, &branch, budget)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    for index in 0..actual.len() {
        let mut merged = Vec::new();
        for domain in domains.iter().filter(|domain| {
            domain
                .iter()
                .zip(actual)
                .enumerate()
                .all(|(other, (expected, actual))| other == index || expected.accepts(actual))
        }) {
            let mut remaining = budget.get();
            sanim_core::type_budget::charge(&domain[index], &mut remaining)?;
            budget.set(remaining);
            merged.push(domain[index].clone());
        }
        let domains = merged;
        if !domains.is_empty() && Type::union(domains)?.accepts(&actual[index]) {
            return Ok(true);
        }
    }
    Ok(false)
}
