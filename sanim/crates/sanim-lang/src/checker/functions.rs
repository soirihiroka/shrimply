//! One function-body checker for named declarations and anonymous expressions.
use super::*;

impl Checker<'_> {
    pub(super) fn charge_callable_origins(
        &self,
        origins: &BTreeSet<String>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.environment_bytes(
            std::mem::size_of::<BTreeSet<String>>()
                + origins
                    .iter()
                    .map(|name| name.len() + std::mem::size_of::<String>() * 2)
                    .sum::<usize>(),
            span,
        )
    }

    /// Inferred callable copies keep their lexical identity until transitive effects settle.
    pub(super) fn refresh_callable_effects(
        &mut self,
        env: &mut Environment,
    ) -> Result<(), Diagnostic> {
        let facts: BTreeMap<_, _> =
            env.iter()
                .filter(|(_, ty)| !ty.signatures().is_empty())
                .map(|(name, ty)| {
                    (
                        name.to_string(),
                        ty.signatures().iter().any(|signature| signature.mutates),
                    )
                })
                .chain(self.anonymous_names.iter().map(|(name, location)| {
                    (name.clone(), self.anonymous_functions[location].mutates)
                }))
                .collect();
        let refresh = |ty: &mut Type, origins: &BTreeSet<String>| {
            let mutates = origins
                .iter()
                .filter_map(|origin| facts.get(origin))
                .any(|mutates| *mutates);
            for signature in ty.signatures_mut() {
                signature.mutates = mutates;
            }
        };
        for (location, origin) in &self.callable_origins {
            if let Some(ty) = self.result_types.get_mut(location) {
                refresh(ty, origin);
            }
        }
        for (location, origin) in &self.callable_bindings {
            if let Some(ty) = self.binding_types.get_mut(location) {
                refresh(ty, origin);
            }
        }
        for (location, generics) in &self.callable_substitutions {
            if let Some(substitutions) = self.call_types.get_mut(location) {
                for (generic, origin) in generics {
                    if let Some(ty) = substitutions.get_mut(generic) {
                        refresh(ty, origin);
                    }
                }
            }
        }
        let exports: Vec<_> = self.binding_scopes[0]
            .iter()
            .filter(|(_, binding)| !binding.callable.is_empty())
            .map(|(name, binding)| (name.clone(), binding.callable.clone()))
            .collect();
        for (name, origin) in exports {
            refresh(
                self.mutate_environment(env, &name, Span::default())?,
                &origin,
            );
        }
        Ok(())
    }

    pub(super) fn check_function(
        &mut self,
        statement: &Statement,
        env: &Environment,
        signature: &Signature,
        entry_guards: bool,
        outer_variables: &BTreeMap<String, Type>,
    ) -> Result<(), Diagnostic> {
        let StatementKind::Function {
            name,
            parameters,
            body,
            ..
        } = &statement.kind
        else {
            unreachable!("function declaration")
        };
        if self.interface.is_some()
            && signature
                .capabilities
                .contains(sanim_core::FnCapability::Sig)
        {
            return Err(self.error(
                "S141",
                "signals are unavailable in expressions",
                statement.span,
            ));
        }
        let previous = self.function.replace(name.clone());
        let previous_capabilities = self.function_capabilities.replace(signature.capabilities);
        let checked = (|| {
            for parameter in parameters {
                match parameter.kind {
                    BindingKind::Mut => self.require_capability(
                        sanim_core::FnCapability::Mut,
                        parameter.name_span,
                        "mutable parameters must be explicitly opted in",
                    )?,
                    BindingKind::Sig => self.require_capability(
                        sanim_core::FnCapability::Sig,
                        parameter.name_span,
                        "live signal parameters must be explicitly opted in",
                    )?,
                    BindingKind::Let => {}
                }
            }
            if signature.result.contains_signal() {
                self.require_capability(
                    sanim_core::FnCapability::Sig,
                    statement.span,
                    "live signal return values must be explicitly opted in",
                )?;
            }
            let mut variables = outer_variables.clone();
            variables.extend(generic_variables(&signature.generics));
            let mut saw_default = false;
            for parameter in parameters {
                let ty = &parameter.ty;
                let default = &parameter.default;
                if let Some(default) = default {
                    saw_default = true;
                    let expected = self.resolve(ty, &variables, 0, default.span)?;
                    let actual = self.expression(default, env, Some(&expected), &variables)?;
                    self.expect(&expected, &actual, default.span)?;
                } else if saw_default {
                    return Err(self.error(
                        "S124",
                        "required parameters must precede parameters with defaults",
                        statement.span,
                    ));
                }
            }
            let signature = signature.clone();
            let mut scope = self.fork_environment(env, statement.span)?;
            // A single-clause signal accepts the complete source schema.
            // Overload guards prove only the sample used to dispatch.

            for parameter in &signature.parameters {
                let name = &parameter.name;
                let ty = &parameter.ty;
                self.fresh(name, &scope, statement.span)?;
                self.insert_environment(
                    &mut scope,
                    name.clone(),
                    if parameter.kind == BindingKind::Sig && entry_guards {
                        ty.clone().signal_schema()
                    } else {
                        ty.clone()
                    },
                    statement.span,
                )?;
            }

            self.binding_scopes.push(BTreeMap::new());
            for parameter in &signature.parameters {
                self.bind_kind(&parameter.name, parameter.kind);
            }
            let returns = self.block(body, &mut scope, Some(&signature.result), &variables)?;
            self.binding_scopes.pop();
            if *signature.result != Type::Unit && !returns {
                return Err(self.error(
                    "S110",
                    format!(
                        "function '{name}' must return {:?} on every path",
                        signature.result
                    ),
                    statement.span,
                ));
            }

            Ok(())
        })();
        self.function = previous;
        self.function_capabilities = previous_capabilities;
        checked
    }
}

fn check_signal_capture(
    checker: &Checker<'_>,
    expression: &Expr,
    env: &Environment,
) -> Result<(), Diagnostic> {
    if let ExprKind::Call { callee, .. } = &expression.kind
        && crate::analysis::lookup(callee, &(env, &checker.result_types)).is_some_and(|ty| {
            ty.signatures()
                .iter()
                .any(|signature| matches!(signature.result.as_ref(), Type::Signal(_)))
        })
    {
        return Ok(());
    }
    let mut invalid = None;
    expression.visit(&mut |expr| {
        if matches!(expr.kind, ExprKind::Reflect { .. }) || checker.call_state_effect(expr, env) {
            invalid.get_or_insert(expr.span);
        }
    });
    if let Some(span) = invalid {
        return Err(checker.error("S131","sig captures must be pure: compute scheduling, mutation, assertions and reflected defaults into a let binding before capturing the result",span));
    }
    Ok(())
}
pub(super) fn check_signal_purity(
    checker: &Checker<'_>,
    statement: &Statement,
    env: &Environment,
) -> Result<(), Diagnostic> {
    fn statements(
        checker: &Checker<'_>,
        body: &[Statement],
        env: &Environment,
        live_return: bool,
    ) -> Result<(), Diagnostic> {
        for statement in body {
            let mut closure_error = None;
            statement.visit_own_expressions(&mut |expr| {
                if let ExprKind::Function(declaration) = &expr.kind
                    && let Err(error) =
                        statements(checker, std::slice::from_ref(declaration), env, false)
                {
                    closure_error.get_or_insert(error);
                }
            });
            if let Some(error) = closure_error {
                return Err(error);
            }
            match &statement.kind {
                StatementKind::Bind {
                    kind: BindingKind::Sig,
                    value,
                    ..
                } => check_signal_capture(checker, value, env)?,
                StatementKind::Return(Some(value)) if live_return => {
                    check_signal_capture(checker, value, env)?
                }
                StatementKind::Function {
                    parameters,
                    name,
                    body,
                    ..
                } => {
                    for parameter in parameters.iter().filter(|p| p.kind == BindingKind::Sig) {
                        if let Some(default) = &parameter.default {
                            check_signal_capture(checker, default, env)?;
                        }
                    }
                    statements(
                        checker,
                        body,
                        env,
                        env.get(name.as_str())
                            .map(|ty| ty.as_ref())
                            .or_else(|| {
                                checker
                                    .anonymous_names
                                    .get(name)
                                    .and_then(|location| checker.result_types.get(location))
                            })
                            .is_some_and(|ty| {
                                ty.signatures().iter().any(|signature| {
                                    matches!(signature.result.as_ref(), Type::Signal(_))
                                })
                            }),
                    )?;
                }
                StatementKind::For { body, .. } | StatementKind::Repeat { body, .. } => {
                    statements(checker, body, env, live_return)?
                }
                StatementKind::If { yes, no, .. } => {
                    statements(checker, yes, env, live_return)?;
                    statements(checker, no, env, live_return)?;
                }
                StatementKind::Match { cases, .. } => {
                    for case in cases {
                        statements(checker, &case.body, env, live_return)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    statements(checker, std::slice::from_ref(statement), env, false)?;
    let mut invalid = None;
    let mut visit = |expr: &Expr| {
        let ExprKind::Call { callee, args, .. } = &expr.kind else {
            return;
        };
        let scope = (env, &checker.result_types);
        let Some(signature) =
            crate::analysis::lookup(callee, &scope).and_then(|ty| ty.signatures().first())
        else {
            return;
        };
        let mut positional = 0;
        for (label, value) in args {
            let parameter = if let Some(label) = label {
                signature.parameters.iter().find(|p| &p.name == label)
            } else {
                let parameter = signature.parameters.get(positional);
                positional += 1;
                parameter
            };
            if parameter.is_some_and(|p| p.kind == BindingKind::Sig)
                && let Err(error) = check_signal_capture(checker, value, env)
            {
                invalid.get_or_insert(error);
            }
        }
    };
    statement.visit_all_expressions(&mut visit);
    invalid.map_or(Ok(()), Err)
}
