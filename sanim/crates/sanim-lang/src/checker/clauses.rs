//! Clause selection uses checked type domains, never a second predicate language.
use super::*;

impl Checker<'_> {
    pub(super) fn declare_clauses(
        &mut self,
        module: &Module,
        env: &mut Environment,
    ) -> Result<BTreeMap<usize, usize>, Diagnostic> {
        let mut function_clauses = BTreeMap::new();
        let mut declared_functions = BTreeSet::new();
        for statement in &module.statements {
            if let StatementKind::Function {
                name,
                generics,
                parameters,
                result,
                ..
            } = &statement.kind
            {
                if !declared_functions.contains(name) {
                    self.fresh(name, env, statement.span)?;
                }
                let resolved = self.generic_parameters(generics, env, statement.span)?;
                let variables = generic_variables(&resolved);
                let mut seen = BTreeSet::new();
                let mut params = vec![];
                for parameter in parameters {
                    let name = &parameter.name;
                    let ty = &parameter.ty;
                    self.fresh(name, env, statement.span)?;
                    if !seen.insert(name) {
                        return Err(self.error(
                            "S103",
                            format!("duplicate parameter '{name}'"),
                            statement.span,
                        ));
                    }
                    let ty = self.resolve(ty, &variables, 0, statement.span)?;
                    if ty.contains_signal() {
                        return Err(self.error("S106","write sig before the parameter name, for example fn f(sig value:Number), rather than using sig in its value type",parameter.name_span));
                    }
                    params.push(sanim_core::FunctionParameter {
                        name: name.clone(),
                        ty,
                        kind: parameter.kind,
                    });
                }
                let result = self.resolve(result, &variables, 0, statement.span)?;
                let signature = Signature {
                    duration: sanim_core::TimelineEffect::default(),
                    schedules: false,
                    mutates: false,
                    duration_parameters: BTreeSet::new(),
                    generics: resolved,
                    default_count: parameters
                        .iter()
                        .filter(|parameter| parameter.default.is_some())
                        .count(),
                    parameters: params,
                    result: Box::new(result),
                };
                let index = if declared_functions.insert(name.clone()) {
                    self.insert_environment(
                        env,
                        name.clone(),
                        Type::Function(signature),
                        statement.span,
                    )?;
                    0
                } else {
                    let existing = self.mutate_environment(env, name, statement.span)?;
                    for previous in existing.signatures() {
                        // Disjointness is decidable but pairwise; charge traversal
                        // work against the same finite budget as schema expansion.
                        let mut remaining = self.type_budget.get();
                        for parameter in &previous.parameters {
                            sanim_core::type_budget::charge(&parameter.ty,&mut remaining).map_err(|reason|self.error("S105",format!("function clause analysis budget exceeded: {reason}; split '{name}' into smaller helpers"),statement.span))?;
                        }
                        self.type_budget.set(remaining);
                        if previous.generics != signature.generics
                            || previous.default_count != signature.default_count
                            || previous.parameters.len() != signature.parameters.len()
                            || previous
                                .parameters
                                .iter()
                                .zip(&signature.parameters)
                                .any(|(a, b)| a.name != b.name || a.kind != b.kind)
                        {
                            return Err(self.error("S126",format!("clauses of '{name}' must use the same parameter names, binding kinds, generic declarations and default positions"),statement.span));
                        }
                        if matches!(previous.result.as_ref(), Type::Signal(_))
                            != matches!(signature.result.as_ref(), Type::Signal(_))
                        {
                            return Err(self.error("S126",format!("clauses of '{name}' must agree whether the return value is a signal; write sig ReturnType on every clause or none"),statement.span));
                        }
                        if previous
                            .parameters
                            .iter()
                            .zip(&signature.parameters)
                            .all(|(a, b)| a.ty.overlaps(&b.ty))
                        {
                            return Err(self.error("S126",format!("ambiguous clauses for '{name}': {} overlaps {}; make at least one parameter domain disjoint (for example Int < 1 and Int >= 1)",Type::Function(previous.clone()),Type::Function(signature.clone())),statement.span));
                        }
                    }
                    let index = existing.signatures().len();
                    match existing {
                        Type::Function(previous) => {
                            *existing = Type::Overloaded(vec![previous.clone(), signature]);
                        }
                        Type::Overloaded(clauses) => clauses.push(signature),
                        _ => unreachable!(),
                    }
                    index
                };
                function_clauses.insert(statement.span.start, index);
            }
        }
        Ok(function_clauses)
    }
    pub(super) fn clause_signature(
        &mut self,
        signatures: &[Signature],
        args: &[(Option<String>, Expr)],
        env: &Environment,
        variables: &BTreeMap<String, Type>,
        span: Span,
    ) -> Result<(Signature, Vec<Option<Type>>), Diagnostic> {
        let first = &signatures[0];
        let ordered = self.arguments(
            args,
            &first
                .parameters
                .iter()
                .enumerate()
                .map(|(index, p)| {
                    (
                        p.name.as_str(),
                        p.ty.clone(),
                        index < first.parameters.len() - first.default_count,
                    )
                })
                .collect::<Vec<_>>(),
            span,
        )?;
        let actual = ordered
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .map(|value| {
                        if let Some(number) = literal_number(value) {
                            return Ok(Type::RefinedNumeric(
                                sanim_core::math::domain::NumericDomain::singleton(number),
                            ));
                        }
                        if let ExprKind::String(text) = &value.kind {
                            return Ok(Type::StringLiteral(text.clone().into()));
                        }
                        let expected = Type::union(
                            signatures
                                .iter()
                                .map(|signature| signature.parameters[index].ty.clone()),
                        )
                        .expect("nonempty clauses");
                        self.expression(value, env, Some(&expected), variables)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let candidates = candidates(signatures, &actual);
        let Some(first) = candidates.first() else {
            return Err(self.error(
                "S127",
                format!(
                    "no function clause accepts these argument types: {}; accepted clauses: {}",
                    actual
                        .iter()
                        .map(|ty| ty.as_ref().map_or("default".into(), ToString::to_string))
                        .collect::<Vec<_>>()
                        .join(", "),
                    Type::Overloaded(signatures.to_vec())
                ),
                span,
            ));
        };
        let mut combined = (*first).clone();
        for (index, parameter) in combined.parameters.iter_mut().enumerate() {
            parameter.ty = Type::union(
                candidates
                    .iter()
                    .map(|signature| signature.parameters[index].ty.clone()),
            )
            .expect("nonempty candidates");
        }
        combined.result = Box::new(if matches!(combined.result.as_ref(), Type::Signal(_)) {
            Type::Signal(Box::new(
                Type::union(candidates.iter().map(|signature| {
                    if let Type::Signal(inner) = signature.result.as_ref() {
                        inner.as_ref().clone()
                    } else {
                        unreachable!("checked common return mode")
                    }
                }))
                .expect("nonempty candidates"),
            ))
        } else {
            Type::union(
                candidates
                    .iter()
                    .map(|signature| signature.result.as_ref().clone()),
            )
            .expect("nonempty candidates")
        });
        combined.schedules = candidates.iter().any(|signature| signature.schedules);
        combined.mutates = candidates.iter().any(|signature| signature.mutates);
        combined.duration = sanim_core::TimelineEffect::alternatives(
            candidates
                .iter()
                .map(|signature| signature.duration.clone()),
        )
        .map_err(|reason| self.error("S127", reason, span))?;
        Ok((combined, actual))
    }

    pub(super) fn complete_generics(
        &self,
        signature: &Signature,
        substitutions: &mut BTreeMap<String, Type>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        for parameter in &signature.generics {
            if !substitutions.contains_key(&parameter.name) {
                let default=parameter.default.as_ref().ok_or_else(||self.error("S123",format!("cannot infer type parameter '{}'; provide explicit <Type> arguments or declare a default",parameter.name),span))?;
                substitutions.insert(
                    parameter.name.clone(),
                    self.substitute(default, substitutions, span)?,
                );
            }
            let actual = &substitutions[&parameter.name];
            if let Some(bound) = &parameter.constraint
                && !bound.substitute(substitutions).accepts(actual)
            {
                return Err(self.error(
                    "S106",
                    format!(
                        "type {actual} does not satisfy constraint {bound:?} for '{}'",
                        parameter.name
                    ),
                    span,
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn candidates<'a>(
    signatures: &'a [Signature],
    actual: &[Option<Type>],
) -> Vec<&'a Signature> {
    signatures
        .iter()
        .filter(|signature| {
            actual.len() >= signature.parameters.len() - signature.default_count
                && actual.len() <= signature.parameters.len()
                && signature
                    .parameters
                    .iter()
                    .zip(actual)
                    .all(|(parameter, actual)| {
                        actual
                            .as_ref()
                            .is_none_or(|actual| parameter.ty.overlaps(actual))
                    })
        })
        .collect()
}

pub(super) fn refined_context(ty: &Type) -> bool {
    matches!(ty, Type::RefinedNumeric(_))
        || matches!(ty,Type::Union(types) if types.iter().any(refined_context))
}

pub(super) fn literal_number(expr: &Expr) -> Option<Number> {
    match &expr.kind {
        ExprKind::Number(text) => Number::parse(text).ok(),
        ExprKind::Unary(operator, value) if matches!(operator.as_str(), "+" | "-") => {
            let mut number = literal_number(value)?;
            if operator == "-" {
                number.value = -number.value;
            }
            Some(number)
        }
        _ => None,
    }
}

pub(super) fn infer(
    parameter: &Type,
    actual: &Type,
    bindings: &mut BTreeMap<String, Type>,
    budget: &std::cell::Cell<usize>,
) -> Result<(), String> {
    match (parameter, actual) {
        (Type::Signal(inner), Type::Signal(actual)) => infer(inner, actual, bindings, budget)?,
        (Type::Signal(inner), actual) => infer(inner, actual, bindings, budget)?,
        (parameter, Type::Signal(actual)) => infer(parameter, actual, bindings, budget)?,
        (Type::Union(patterns), actual) => {
            for pattern in patterns {
                let mut candidate = bindings.clone();
                if infer(pattern, actual, &mut candidate, budget).is_ok()
                    && pattern.substitute(&candidate).accepts(actual)
                {
                    *bindings = candidate;
                    return Ok(());
                }
            }
        }

        (Type::Variable(name) | Type::BoundedVariable(name, _), actual) => {
            if let Some(previous) = bindings.get(name) {
                if !previous.accepts(actual) {
                    return Err(format!(
                        "generic '{name}' was {previous:?}, but this argument is {actual:?}"
                    ));
                }
            } else if *actual != Type::Unknown {
                let actual = if matches!(parameter,Type::BoundedVariable(_,bound) if (matches!(bound.as_ref(),TypeBound::Numeric) || matches!(bound.as_ref(),TypeBound::Type(ty) if matches!(ty.as_ref(),Type::Numeric(Unit::Number)))))
                    && actual.numeric_unit() == Some(Unit::Number)
                {
                    Type::Numeric(Unit::Number)
                } else if matches!(parameter,Type::BoundedVariable(_,bound) if matches!(bound.as_ref(),TypeBound::Type(_)))
                {
                    actual.clone()
                } else {
                    actual.clone().widen_literals()
                };
                let mut remaining = budget.get();
                sanim_core::type_budget::charge(&actual, &mut remaining)?;
                budget.set(remaining);
                bindings.insert(name.clone(), actual);
            }
        }
        (Type::List(a, _), Type::List(b, _))
        | (Type::Option(a), Type::Option(b))
        | (Type::Array(a), Type::Array(b))
        | (Type::Series(a), Type::Series(b)) => infer(a, b, bindings, budget)?,
        (Type::DataFrame { row: a, groups: ga }, Type::DataFrame { row: b, groups: gb })
            if ga == gb =>
        {
            infer(a, b, bindings, budget)?
        }
        (Type::Record(a), Type::Record(b)) => {
            for (name, ty) in a {
                if let Some(actual) = b.get(name) {
                    infer(ty, actual, bindings, budget)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
