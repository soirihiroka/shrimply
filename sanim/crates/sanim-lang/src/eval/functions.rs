//! Named and anonymous callables share invocation, validation and persistent captures.
use super::*;

impl Function {
    pub(super) fn from_statement(module: String, statement: &Statement, clause: usize) -> Self {
        let StatementKind::Function {
            capabilities,
            name,
            parameters,
            body,
            ..
        } = &statement.kind
        else {
            unreachable!("function AST");
        };
        let mut references = BTreeSet::new();
        crate::ast::function_references(statement, &mut references);
        Self {
            capabilities: *capabilities,
            references,
            module,
            name: name.clone(),
            parameters: parameters.clone(),
            body: body.clone(),
            clause,
            span: statement.span,
        }
    }
}

impl Runtime<'_, '_> {
    pub(super) fn function_group(&self, key: &str) -> Option<&Arc<FunctionGroup>> {
        self.closures.get(key).or_else(|| self.functions.get(key))
    }
    pub(super) fn closure(
        &mut self,
        statement: &Statement,
        env: &Env,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let StatementKind::Function { name, .. } = &statement.kind else {
            unreachable!("anonymous function AST");
        };
        let location = (self.module.clone(), span.start, span.end);
        let definition = if let Some(definition) = self.anonymous_definitions.get(&location) {
            definition.clone()
        } else {
            let definition = Arc::new(Function::from_statement(self.module.clone(), statement, 0));
            let serialized =
                serde_json::to_vec(&definition).expect("serializable function definition");
            let fingerprint = digest(&serialized);
            let syntax_bytes = serialized
                .len()
                .saturating_add(std::mem::size_of::<ClosureDefinition>())
                .saturating_add(
                    definition
                        .references
                        .iter()
                        .map(|name| name.len() + std::mem::size_of::<String>() * 2)
                        .sum::<usize>(),
                );
            self.retained_bytes =
                self.retained_bytes
                    .checked_sub(syntax_bytes)
                    .ok_or_else(|| {
                        self.error(
                            "S303",
                            "anonymous function definition exceeds the retained-data budget",
                            span,
                        )
                    })?;
            let definition = Arc::new(ClosureDefinition {
                function: definition,
                fingerprint,
            });
            self.anonymous_definitions
                .insert(location, definition.clone());
            definition
        };
        let captures: Env = definition
            .function
            .references
            .iter()
            .filter_map(|name| {
                env.get(name)
                    .cloned()
                    .map(|binding| (name.clone(), binding))
            })
            .collect();
        self.charge_capture_types(span)?;
        let key = format!("{}::{}#{}", self.module, name, self.next_closure);
        let overhead = std::mem::size_of::<FunctionGroup>()
            + captures.len() * (std::mem::size_of::<Binding>() + std::mem::size_of::<String>() * 2)
            + captures.keys().map(String::len).sum::<usize>()
            + key.len() * 2
            + definition.fingerprint.len();
        self.retained_bytes = self.retained_bytes.checked_sub(overhead).ok_or_else(|| {
            self.error(
                "S303",
                "anonymous function capture storage exceeds the preparation byte budget",
                span,
            )
        })?;
        self.retained_items = self.retained_items.checked_sub(1).ok_or_else(|| {
            self.error(
                "S303",
                "anonymous functions exceed the preparation value budget",
                span,
            )
        })?;
        for binding in captures.values() {
            match binding {
                Binding::Reactive(id) | Binding::Deferred(id) => self.reactive.retain(*id),
                Binding::Value(Value::Signal(signal)) => self.reactive.retain(signal.id),
                _ => {}
            }
        }
        self.next_closure = self
            .next_closure
            .checked_add(1)
            .expect("bounded closure identity overflow");
        self.closures.insert(
            key.clone(),
            Arc::new(FunctionGroup {
                clauses: vec![definition.function.clone()],
                fingerprint: definition.fingerprint.clone(),
                captures: Some(captures),
                types: self.type_bindings.clone(),
            }),
        );
        Ok(Value::Function(key))
    }
    pub(super) fn function_signature(&self, function: &Function) -> Option<&Signature> {
        self.loaded
            .iter()
            .find(|module| module.id == function.module)
            .and_then(|module| {
                module
                    .checked
                    .anonymous_functions
                    .get(&(function.span.start, function.span.end))
                    .or_else(|| {
                        module
                            .checked
                            .exports
                            .get(&function.name)
                            .and_then(|ty| ty.signatures().get(function.clause))
                    })
            })
    }
    pub(super) fn record_function(&mut self, key: &str, group: &FunctionGroup) {
        if !self.used_functions.contains_key(key) {
            self.used_functions.insert(
                key.to_owned(),
                (group.fingerprint.clone(), self.root_statement),
            );
        }
    }
    pub(super) fn call_type_bindings(&self, span: Span, captured: &TypeBindings) -> TypeBindings {
        let inferred = self
            .loaded
            .iter()
            .find(|module| module.id == self.module)
            .and_then(|module| module.checked.call_types.get(&(span.start, span.end)));
        if inferred.is_none_or(BTreeMap::is_empty) && captured.is_empty() {
            return self.type_bindings.clone();
        }
        if inferred.is_none_or(BTreeMap::is_empty) && self.type_bindings.is_empty() {
            return captured.clone();
        }
        let mut bindings = captured.as_ref().clone();
        if let Some(inferred) = inferred {
            bindings.extend(
                inferred
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.substitute(&self.type_bindings))),
            );
        } else {
            bindings.extend(
                self.type_bindings
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.clone())),
            );
        }
        Arc::new(bindings)
    }
    pub(super) fn charge_capture_types(&mut self, span: Span) -> Result<(), Diagnostic> {
        self.retained_resources
            .charge_types(&self.type_bindings, &mut self.retained_bytes)
            .map_err(|error| self.error("S303", error, span))
    }
    pub(super) fn typed_value(
        &self,
        value: &Value,
        expected: &Type,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if let Value::Function(key) = value {
            let group = self
                .function_group(key)
                .ok_or_else(|| self.error("S140", "function value is no longer available", span))?;
            let signature = group
                .clauses
                .first()
                .and_then(|function| self.function_signature(function))
                .ok_or_else(|| {
                    self.error(
                        "S140",
                        "function value is missing its checked signature",
                        span,
                    )
                })?;
            let actual = Type::Function(signature.clone()).substitute(&group.types);
            return if expected.accepts(&actual) {
                Ok(value.clone())
            } else {
                Err(self.error("S140", format!("expected {expected}, found {actual}"), span))
            };
        }
        value
            .with_type(expected)
            .map_err(|error| self.error("S140", error, span))
    }
    pub(super) fn select_function(
        &mut self,
        group: &FunctionGroup,
        evaluated: &Values,
        substitutions: &BTreeMap<String, Type>,
        span: Span,
    ) -> Result<Arc<Function>, Diagnostic> {
        let first = group
            .clauses
            .first()
            .ok_or_else(|| self.error("S140", "function has no checked clauses", span))?;
        if group.clauses.len() == 1 {
            return Ok(first.clone());
        }
        let mut arguments = Values::new();
        for (name, value) in evaluated {
            arguments.insert(name.clone(), self.sample_signal(value.clone(), span)?);
        }
        let mut selected = None;
        for function in &group.clauses {
            let signature = self.function_signature(function).ok_or_else(|| {
                self.error(
                    "S140",
                    "function clause is missing its checked signature",
                    span,
                )
            })?;
            let parameters = signature
                .parameters
                .iter()
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        parameter.ty.substitute(substitutions),
                    )
                })
                .collect::<Vec<_>>();
            let mut matches = true;
            for (name, ty) in parameters {
                let Some(value) = arguments.get(&name) else {
                    continue;
                };
                let available = self
                    .limits
                    .max_operations
                    .saturating_sub(self.logical_operations);
                let mut remaining = available;
                let mut exhausted = false;
                let result =
                    sanim_core::validation::validate_typed_value(&ty, value, &mut |value| {
                        let work = match value {
                            Value::String(text) => text.len().max(1) as u64,
                            _ => 1,
                        };
                        remaining = remaining.checked_sub(work).ok_or_else(|| {
                            exhausted = true;
                            "function clause validation exceeded the operation budget".to_string()
                        })?;
                        Ok(())
                    });
                self.charge_work(
                    if exhausted {
                        available
                    } else {
                        available - remaining
                    },
                    span,
                )?;
                if exhausted {
                    return Err(self.error("S303", "function clause validation exceeded the operation budget; narrow the input schema to avoid repeated data scans", span));
                }
                if result.is_err() {
                    matches = false;
                    break;
                }
            }
            if matches {
                if selected.is_some() {
                    return Err(self.error("S140",format!("function '{}' matches multiple clauses for these arguments; supply an explicit argument to distinguish defaulted clauses",first.name),span));
                }
                selected = Some(function.clone());
            }
        }
        selected.ok_or_else(||self.error("S140",format!("function '{}' has no clause matching the supplied values; checked alternatives: {}",first.name,group.clauses.iter().filter_map(|function|self.function_signature(function)).map(|signature|signature.parameters.iter().map(|parameter|format!("{}: {}",parameter.name,parameter.ty)).collect::<Vec<_>>().join(", ")).collect::<Vec<_>>().join(" | ")),span))
    }
}
