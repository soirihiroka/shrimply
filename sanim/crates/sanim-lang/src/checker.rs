use crate::{ast::*, prelude};
use sanim_core::{Diagnostic, GenericParameter, Number, Signature, Span, Type, TypeBound, Unit};
use std::collections::{BTreeMap, BTreeSet};

pub type Environment = BTreeMap<String, Type>;
#[derive(Clone, Debug)]
pub struct Alias {
    pub parameters: Vec<GenericParameter<Type, TypeBound>>,
    pub body: TypeExpr,
}
#[derive(Clone, Debug, Default)]
pub struct Checked {
    pub exports: Environment,
    pub schemas: BTreeMap<String, Type>,
    pub read_types: BTreeMap<usize, Type>,
    pub call_types: BTreeMap<usize, BTreeMap<String, Type>>,
    pub reflected: BTreeMap<usize, std::sync::Arc<sanim_core::reflection::Declaration>>,
    pub binding_types: BTreeMap<usize, Type>,
    pub effects: BTreeSet<String>,
    pub durations: Vec<sanim_core::NodeDuration>,
    pub duration_needed: BTreeSet<String>,
    pub value_dependencies: BTreeMap<String, BTreeSet<String>>,
}

pub fn check(
    source: &str,
    module: &Module,
    imports: &BTreeMap<String, std::sync::Arc<Checked>>,
    root: bool,
    type_budget: &std::cell::Cell<usize>,
    limits: &crate::Limits,
) -> Result<Checked, Diagnostic> {
    let mut checker = Checker {
        source,
        type_budget,
        aliases: BTreeMap::new(),
        schemas: BTreeMap::new(),
        reads: BTreeMap::new(),
        call_types: BTreeMap::new(),
        reflected: BTreeMap::new(),
        binding_types: BTreeMap::new(),
        binding_scopes: vec![BTreeMap::new()],
        mutations: BTreeSet::new(),
        reflection_names: crate::reflection::identities(source, module)?,
        function: None,
        calls: BTreeMap::new(),
        effects: BTreeSet::new(),
        root,
    };
    let mut env = Environment::new();
    // Type declarations are resolved in declaration order; recursive types are deliberately rejected.
    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Alias { name, generics, ty } => {
                checker.fresh(name, &env, statement.span)?;
                let resolved = checker.generic_parameters(generics, &env, statement.span)?;
                let parameters = generic_variables(&resolved);
                let resolved_type = if generics.is_empty() {
                    Some(checker.resolve(ty, &parameters, 0, statement.span)?)
                } else {
                    checker.validate_template(ty, &parameters, 0, statement.span)?;
                    None
                };
                checker.aliases.insert(
                    name.clone(),
                    Alias {
                        parameters: resolved,
                        body: ty.clone(),
                    },
                );
                if let Some(resolved_type) = resolved_type {
                    checker.schemas.insert(name.clone(), resolved_type);
                }
            }
            StatementKind::Enum { name, variants } => {
                checker.fresh(name, &env, statement.span)?;
                let mut out = BTreeMap::new();
                for (tag, fields) in variants {
                    if out.contains_key(tag) {
                        return Err(checker.error(
                            "S103",
                            format!("duplicate enum variant '{tag}'"),
                            statement.span,
                        ));
                    }
                    let fields = fields
                        .iter()
                        .map(|v| checker.resolve(v, &BTreeMap::new(), 0, statement.span))
                        .collect::<Result<Vec<_>, _>>()?;
                    out.insert(tag.clone(), fields);
                }
                if out.is_empty() {
                    return Err(checker.error("S103", "enum must have a variant", statement.span));
                }
                let ty = Type::Enum {
                    name: format!("{source}::{name}"),
                    variants: out.clone(),
                };
                checker
                    .schemas
                    .insert(name.clone(), checker.copy_type(&ty, statement.span)?);
                let mut namespace = BTreeMap::new();
                for (tag, fields) in out {
                    let result = checker.copy_type(&ty, statement.span)?;
                    let field = if fields.is_empty() {
                        result
                    } else {
                        Type::Function(Signature {
                            duration: sanim_core::TimelineEffect::default(),
                            schedules: false,
                            mutates: false,
                            duration_parameters: BTreeSet::new(),
                            generics: vec![],
                            default_count: 0,
                            parameters: fields
                                .into_iter()
                                .enumerate()
                                .map(|(i, t)| (format!("value{i}"), t))
                                .collect(),
                            result: Box::new(result),
                        })
                    };
                    namespace.insert(tag, field);
                }
                env.insert(name.clone(), Type::Record(namespace));
            }
            StatementKind::Import { path, alias } => {
                checker.fresh(alias, &env, statement.span)?;
                let imported = imports.get(path).ok_or_else(|| {
                    checker.error(
                        "S104",
                        format!("module '{path}' was not loaded"),
                        statement.span,
                    )
                })?;
                imported
                    .charge_types(type_budget)
                    .map_err(|e| checker.error("S105", e, statement.span))?;
                for (name, ty) in &imported.schemas {
                    checker
                        .schemas
                        .insert(format!("{alias}.{name}"), ty.clone());
                }
                env.insert(alias.clone(), Type::Record(imported.exports.clone()));
            }
            _ => {}
        }
    }
    for (name, alias) in &checker.aliases {
        if alias.parameters.is_empty() {
            let ty = checker.schemas[name].clone();
            if let Type::Record(fields) = &ty {
                env.insert(
                    name.clone(),
                    Type::Function(Signature {
                        duration: sanim_core::TimelineEffect::default(),
                        schedules: false,
                        mutates: false,
                        duration_parameters: BTreeSet::new(),
                        generics: vec![],
                        default_count: 0,
                        parameters: fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                        result: Box::new(ty),
                    }),
                );
            }
        }
    }
    for statement in &module.statements {
        if let StatementKind::Function {
            name,
            generics,
            parameters,
            result,
            ..
        } = &statement.kind
        {
            checker.fresh(name, &env, statement.span)?;
            let resolved = checker.generic_parameters(generics, &env, statement.span)?;
            let variables = generic_variables(&resolved);
            let mut seen = BTreeSet::new();
            let mut params = vec![];
            for (name, ty, _) in parameters {
                checker.fresh(name, &env, statement.span)?;
                if !seen.insert(name) {
                    return Err(checker.error(
                        "S103",
                        format!("duplicate parameter '{name}'"),
                        statement.span,
                    ));
                }
                params.push((
                    name.clone(),
                    checker.resolve(ty, &variables, 0, statement.span)?,
                ));
            }
            let result = checker.resolve(result, &variables, 0, statement.span)?;
            env.insert(
                name.clone(),
                Type::Function(Signature {
                    duration: sanim_core::TimelineEffect::default(),
                    schedules: false,
                    mutates: false,
                    duration_parameters: BTreeSet::new(),
                    generics: resolved,
                    default_count: parameters.iter().filter(|(_, _, d)| d.is_some()).count(),
                    parameters: params,
                    result: Box::new(result),
                }),
            );
        }
    }
    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. } => {}
            StatementKind::Function {
                name,
                generics: _,
                parameters,
                body,
                ..
            } => {
                checker.function = Some(name.clone());
                let Type::Function(declaration) = &env[name] else {
                    unreachable!()
                };
                let declaration_variables = generic_variables(&declaration.generics);
                let mut saw_default = false;
                for (_, ty, default) in parameters {
                    if let Some(default) = default {
                        saw_default = true;
                        let expected =
                            checker.resolve(ty, &declaration_variables, 0, default.span)?;
                        let actual = checker.expression(
                            default,
                            &env,
                            Some(&expected),
                            &declaration_variables,
                        )?;
                        checker.expect(&expected, &actual, default.span)?;
                    } else if saw_default {
                        return Err(checker.error(
                            "S124",
                            "required parameters must precede parameters with defaults",
                            statement.span,
                        ));
                    }
                }
                let Type::Function(signature) = env[name].clone() else {
                    unreachable!()
                };
                let mut scope = checker.copy_environment(&env, statement.span)?;
                for (name, ty) in &signature.parameters {
                    checker.fresh(name, &scope, statement.span)?;
                    scope.insert(name.clone(), ty.clone());
                }
                checker.function = Some(name.clone());
                let variables = generic_variables(&signature.generics);
                checker.binding_scopes.push(BTreeMap::new());
                for (parameter, _) in &signature.parameters {
                    checker.bind_kind(parameter, BindingKind::Let);
                }
                let returns =
                    checker.block(body, &mut scope, Some(&signature.result), &variables)?;
                checker.binding_scopes.pop();
                if *signature.result != Type::Unit && !returns {
                    return Err(checker.error(
                        "S110",
                        format!(
                            "function '{name}' must return {:?} on every path",
                            signature.result
                        ),
                        statement.span,
                    ));
                }
                checker.function = None;
            }
            _ => {
                checker.block(
                    std::slice::from_ref(statement),
                    &mut env,
                    None,
                    &BTreeMap::new(),
                )?;
            }
        }
    }
    // Reject all direct/indirect recursion and propagate scheduling effects.
    fn visit(
        name: &str,
        calls: &BTreeMap<String, BTreeSet<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        depth: usize,
        limit: usize,
    ) -> bool {
        if depth > limit {
            return false;
        }
        if done.contains(name) {
            return true;
        }
        if !active.insert(name.into()) {
            return false;
        }
        if calls.get(name).is_some_and(|next| {
            next.iter()
                .any(|v| !visit(v, calls, active, done, depth + 1, limit))
        }) {
            return false;
        }
        active.remove(name);
        done.insert(name.into());
        true
    }
    let mut done = BTreeSet::new();
    for name in checker.calls.keys() {
        if !visit(
            name,
            &checker.calls,
            &mut BTreeSet::new(),
            &mut done,
            0,
            limits.max_call_depth,
        ) {
            return Err(checker.error(
                "S111",
                format!("recursive or excessively deep call chain involving '{name}'; use bounded for/repeat"),
                Span::default(),
            ));
        }
    }
    loop {
        let before = (checker.effects.len(), checker.mutations.len());
        for (name, calls) in &checker.calls {
            if calls.iter().any(|v| checker.effects.contains(v)) {
                checker.effects.insert(name.clone());
            }
            if calls.iter().any(|v| checker.mutations.contains(v)) {
                checker.mutations.insert(name.clone());
            }
        }
        if before == (checker.effects.len(), checker.mutations.len()) {
            break;
        }
    }
    for (name, ty) in &mut env {
        if let Type::Function(signature) = ty {
            signature.schedules = checker.effects.contains(name);
            signature.mutates = checker.mutations.contains(name);
        }
    }
    for statement in &module.statements {
        let mut invalid = None;
        statement.visit_expressions(&mut |expr| {
            if let ExprKind::Reflect { value, .. } = &expr.kind {
                value.visit(&mut |node| {
                    if checker.call_state_effect(node, &env) {
                        invalid.get_or_insert(node.span);
                    }
                });
            }
        });
        if let Some(span) = invalid {
            return Err(checker.error("S113","a reflected default must not mutate captured cells, schedule animations, change the scene, or run assertions; compute a pure default value and schedule it outside reflect(...) instead",span));
        }
    }
    for statement in &module.statements {
        check_signal_purity(&checker, statement, &env)?;
    }
    if !root {
        let mut invalid = None;
        for statement in &module.statements {
            statement.visit_expressions(&mut |expr| {
                if checker.call_state_effect(expr, &env) {
                    invalid.get_or_insert(expr.span);
                }
            });
        }
        if let Some(span) = invalid {
            return Err(checker.error("S115","imported modules cannot schedule or assert while being imported; call a helper from the root script instead",span));
        }
    }
    let duration_analysis =
        crate::effects::analyze(source, module, &env, imports, type_budget, limits)?;
    for (name, effect) in &duration_analysis.functions {
        if let Some(Type::Function(signature)) = env.get_mut(name) {
            signature.duration = *effect;
            signature.schedules = checker.effects.contains(name);
            signature.mutates = checker.mutations.contains(name);
            signature.duration_parameters = signature
                .parameters
                .iter()
                .enumerate()
                .filter(|(_, (name, _))| duration_analysis.needed.contains(name))
                .map(|(index, _)| index)
                .collect();
        }
    }
    Ok(Checked {
        durations: duration_analysis.nodes,
        duration_needed: duration_analysis.needed,
        value_dependencies: duration_analysis.value_dependencies,
        exports: env,
        schemas: checker.schemas,
        read_types: checker.reads,
        call_types: checker.call_types,
        reflected: checker.reflected,
        binding_types: checker.binding_types,
        effects: checker.effects,
    })
}

#[derive(Clone)]
struct BindingInfo {
    kind: BindingKind,
    function: Option<String>,
}
struct Checker<'a> {
    source: &'a str,
    type_budget: &'a std::cell::Cell<usize>,
    aliases: BTreeMap<String, Alias>,
    schemas: BTreeMap<String, Type>,
    reads: BTreeMap<usize, Type>,
    call_types: BTreeMap<usize, BTreeMap<String, Type>>,
    reflected: BTreeMap<usize, std::sync::Arc<sanim_core::reflection::Declaration>>,
    reflection_names: BTreeMap<usize, String>,
    binding_types: BTreeMap<usize, Type>,
    binding_scopes: Vec<BTreeMap<String, BindingInfo>>,
    mutations: BTreeSet<String>,
    function: Option<String>,
    calls: BTreeMap<String, BTreeSet<String>>,
    effects: BTreeSet<String>,
    root: bool,
}
impl Checker<'_> {
    fn error(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, message, self.source, span)
    }
    fn call_schedules(&self, expr: &Expr, env: &Environment) -> bool {
        let ExprKind::Call { callee, .. } = &expr.kind else {
            return false;
        };
        if let ExprKind::Name(name) = &callee.kind
            && (prelude::builtin(name).is_some_and(|builtin| builtin.schedule)
                || self.effects.contains(name))
        {
            return true;
        }
        matches!(crate::effects::lookup(callee,env),Some(Type::Function(signature)) if signature.schedules)
    }
    fn call_mutates(&self, expr: &Expr, env: &Environment) -> bool {
        let ExprKind::Call { callee, .. } = &expr.kind else {
            return false;
        };
        matches!(&callee.kind, ExprKind::Name(name) if self.mutations.contains(name))
            || matches!(crate::effects::lookup(callee, env), Some(Type::Function(signature)) if signature.mutates)
    }
    fn call_state_effect(&self, expr: &Expr, env: &Environment) -> bool {
        self.call_schedules(expr, env) || self.call_mutates(expr, env)
    }
    fn bind_kind(&mut self, name: &str, kind: BindingKind) {
        self.binding_scopes
            .last_mut()
            .expect("binding scope")
            .insert(
                name.into(),
                BindingInfo {
                    kind,
                    function: self.function.clone(),
                },
            );
    }
    fn writable(&mut self, name: &str, env: &Environment, span: Span) -> Result<Type, Diagnostic> {
        if prelude::protected(name) {
            return Err(self.error(
                "S101",
                format!("'{name}' is a protected Sanim name and cannot be assigned"),
                span,
            ));
        }
        let ty = env.get(name).ok_or_else(|| self.error("S120", format!("cannot assign unknown binding '{name}'; declare it first with mut {name} = value"), span))?;
        let binding = self
            .binding_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name));
        match binding.map(|binding| binding.kind) {
            Some(BindingKind::Mut) => {}
            Some(BindingKind::Sig) => return Err(self.error("S130", format!("signal '{name}' is read-only; assign one of its mut dependencies instead"), span)),
            _ => return Err(self.error("S130", format!("binding '{name}' is immutable; declare it with mut {name} = value if it needs reassignment. let bindings and function parameters are snapshots."), span)),
        }
        if let Some(function) = &self.function
            && binding.is_some_and(|binding| binding.function.as_ref() != Some(function))
        {
            self.mutations.insert(function.clone());
        }
        self.copy_type(ty, span)
    }
    fn scoped_block(
        &mut self,
        body: &[Statement],
        env: &mut Environment,
        result: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<bool, Diagnostic> {
        self.binding_scopes.push(BTreeMap::new());
        let checked = self.block(body, env, result, variables);
        self.binding_scopes.pop();
        checked
    }
    fn fresh(&self, name: &str, env: &Environment, span: Span) -> Result<(), Diagnostic> {
        if prelude::protected(name) {
            return Err(self.error("S101",format!("'{name}' is a protected Sanim name and cannot be redefined, including in parameters. Choose a different name, for example '{}_value'.",name.to_ascii_lowercase()),span));
        }
        if env.contains_key(name)
            || self.aliases.contains_key(name)
            || self.schemas.contains_key(name)
        {
            return Err(self.error("S102",format!("'{name}' is already defined; shadowing is forbidden. Choose a distinct binding name."),span));
        }
        Ok(())
    }
    fn generic_parameters(
        &self,
        parameters: &[GenericParameter<TypeExpr>],
        env: &Environment,
        span: Span,
    ) -> Result<Vec<GenericParameter<Type, TypeBound>>, Diagnostic> {
        let mut seen = BTreeSet::new();
        let mut variables = BTreeMap::new();
        let mut out = vec![];
        let mut default_seen = false;
        for parameter in parameters {
            self.fresh(&parameter.name, env, span)?;
            if !seen.insert(&parameter.name) {
                return Err(self.error(
                    "S103",
                    format!("duplicate type parameter '{}'", parameter.name),
                    span,
                ));
            }
            let constraint = parameter
                .constraint
                .as_ref()
                .map(|expr| {
                    if let TypeExpr::Named(name, args) = expr
                        && let Some(prelude::TypeDefinition::Bound(bound)) =
                            prelude::types().get(name.as_str())
                    {
                        if !args.is_empty() {
                            return Err(self.error(
                                "S105",
                                "constraints take no type arguments",
                                span,
                            ));
                        }
                        return Ok(bound.clone());
                    }
                    self.resolve(expr, &variables, 0, span)
                        .map(|ty| TypeBound::Type(Box::new(ty)))
                })
                .transpose()?;
            let default = parameter
                .default
                .as_ref()
                .map(|ty| self.resolve(ty, &variables, 0, span))
                .transpose()?;
            if default_seen && default.is_none() {
                return Err(self.error(
                    "S123",
                    "generic parameters without defaults must precede parameters with defaults",
                    span,
                ));
            }
            default_seen |= default.is_some();
            if let (Some(bound), Some(ty)) = (&constraint, &default)
                && !bound.accepts(ty)
            {
                return Err(self.error(
                    "S106",
                    format!(
                        "default type {ty} does not satisfy constraint {bound:?} for '{}'",
                        parameter.name
                    ),
                    span,
                ));
            }
            let parameter = GenericParameter {
                name: parameter.name.clone(),
                constraint,
                default,
            };
            variables.insert(parameter.name.clone(), generic_variable(&parameter));
            out.push(parameter);
        }
        Ok(out)
    }
    fn copy_environment(&self, env: &Environment, span: Span) -> Result<Environment, Diagnostic> {
        env.iter()
            .map(|(name, ty)| self.copy_type(ty, span).map(|ty| (name.clone(), ty)))
            .collect()
    }
    fn substitute(
        &self,
        ty: &Type,
        bindings: &BTreeMap<String, Type>,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        let mut remaining = self.type_budget.get();
        sanim_core::type_budget::charge_substitution(ty, bindings, &mut remaining)
            .map_err(|e| self.error("S105", e, span))?;
        self.type_budget.set(remaining);
        Ok(ty.substitute(bindings))
    }
    fn copy_type(&self, ty: &Type, span: Span) -> Result<Type, Diagnostic> {
        let mut remaining = self.type_budget.get();
        sanim_core::type_budget::charge(ty, &mut remaining)
            .map_err(|e| self.error("S105", e, span))?;
        self.type_budget.set(remaining);
        Ok(ty.clone())
    }
    // Generic aliases retain their source type expression until concrete arguments
    // are supplied. Validate names now; resolve field algebra once the fields exist.
    fn validate_template(
        &self,
        expr: &TypeExpr,
        variables: &BTreeMap<String, Type>,
        depth: usize,
        span: Span,
    ) -> Result<bool, Diagnostic> {
        if depth > 64 {
            return Err(self.error("S105", "generic type nesting limit exceeded", span));
        }
        self.type_budget.set(
            self.type_budget
                .get()
                .checked_sub(std::mem::size_of::<TypeExpr>())
                .ok_or_else(|| self.error("S105", "generic type budget exceeded", span))?,
        );
        let depends = match expr {
            TypeExpr::Named(name, args) => {
                if variables.contains_key(name) {
                    if !args.is_empty() {
                        return Err(self.error(
                            "S105",
                            format!("type parameter '{name}' takes no arguments"),
                            span,
                        ));
                    }
                    true
                } else {
                    if !prelude::types().contains_key(name.as_str())
                        && !self.aliases.contains_key(name)
                        && !self.schemas.contains_key(name)
                    {
                        return Err(self.error("S105", format!("unknown type '{name}'"), span));
                    }
                    let mut dependent = false;
                    for arg in args {
                        dependent |= self.validate_template(arg, variables, depth + 1, span)?;
                    }
                    dependent
                }
            }
            TypeExpr::Record(fields) => {
                let mut seen = BTreeSet::new();
                let mut dependent = false;
                for (name, value) in fields {
                    if !seen.insert(name) {
                        return Err(self.error("S103", format!("duplicate field '{name}'"), span));
                    }
                    dependent |= self.validate_template(value, variables, depth + 1, span)?;
                }
                dependent
            }
            TypeExpr::Intersection(values) | TypeExpr::Union(values) => {
                let mut dependent = false;
                for value in values {
                    dependent |= self.validate_template(value, variables, depth + 1, span)?;
                }
                dependent
            }
            TypeExpr::Difference(left, right) => {
                self.validate_template(left, variables, depth + 1, span)?
                    | self.validate_template(right, variables, depth + 1, span)?
            }
        };
        if !depends {
            self.resolve(expr, variables, depth + 1, span)?;
        }
        Ok(depends)
    }
    fn resolve(
        &self,
        expr: &TypeExpr,
        variables: &BTreeMap<String, Type>,
        depth: usize,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        if depth > 64 {
            return Err(self.error("S105", "type expansion limit exceeded", span));
        }
        self.type_budget.set(
            self.type_budget
                .get()
                .checked_sub(std::mem::size_of::<Type>())
                .ok_or_else(|| self.error("S105", "resolved type budget exceeded", span))?,
        );
        match expr {
            TypeExpr::Intersection(values) => {
                let mut values = values.iter();
                let first = values.next().ok_or_else(|| {
                    self.error("S105", "record composition needs at least one type", span)
                })?;
                let mut result = self.resolve(first, variables, depth + 1, span)?;
                for value in values {
                    let right = self.resolve(value, variables, depth + 1, span)?;
                    result = crate::type_algebra::intersection(result, right, self.type_budget)
                        .map_err(|message| self.error("S105", message, span))?;
                }
                Ok(result)
            }
            TypeExpr::Union(values) => Type::union(
                values
                    .iter()
                    .map(|value| self.resolve(value, variables, depth + 1, span))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|message| self.error("S105", message, span)),
            TypeExpr::Difference(left, right) => {
                let left = self.resolve(left, variables, depth + 1, span)?;
                let right = self.resolve(right, variables, depth + 1, span)?;
                crate::type_algebra::difference(left, right, self.type_budget)
                    .map_err(|message| self.error("S105", message, span))
            }
            TypeExpr::Record(fields) => {
                let mut out = BTreeMap::new();
                for (name, ty) in fields {
                    if out.contains_key(name) {
                        return Err(self.error("S103", format!("duplicate field '{name}'"), span));
                    }
                    out.insert(name.clone(), self.resolve(ty, variables, depth + 1, span)?);
                }
                Ok(Type::Record(out))
            }
            TypeExpr::Named(name, args) => {
                if let Some(ty) = variables.get(name).or_else(|| self.schemas.get(name)) {
                    if !args.is_empty() {
                        return Err(self.error(
                            "S105",
                            format!("type '{name}' takes no arguments"),
                            span,
                        ));
                    }
                    return self.copy_type(ty, span);
                }
                if let Some(definition) = prelude::types().get(name.as_str()) {
                    let args = args
                        .iter()
                        .map(|arg| self.resolve(arg, variables, depth + 1, span))
                        .collect::<Result<Vec<_>, _>>()?;
                    return definition
                        .instantiate(args)
                        .map_err(|e| self.error("S105", format!("type '{name}' {e}"), span));
                }
                if let Some(alias) = self.aliases.get(name) {
                    if args.len() > alias.parameters.len() {
                        return Err(self.error(
                            "S105",
                            format!(
                                "type '{name}' accepts at most {} type arguments",
                                alias.parameters.len()
                            ),
                            span,
                        ));
                    }
                    let mut vars = variables.clone();
                    for (index, param) in alias.parameters.iter().enumerate() {
                        let actual = if let Some(arg) = args.get(index) {
                            self.resolve(arg, variables, depth + 1, span)?
                        } else if let Some(default) = &param.default {
                            self.substitute(default, &vars, span)?
                        } else {
                            return Err(self.error(
                                "S105",
                                format!(
                                    "type '{name}' requires a type argument for '{}'",
                                    param.name
                                ),
                                span,
                            ));
                        };
                        if let Some(bound) = &param.constraint
                            && !bound.substitute(&vars).accepts(&actual)
                        {
                            return Err(self.error(
                                "S106",
                                format!(
                                    "type {actual} does not satisfy constraint {bound:?} for '{}'",
                                    param.name
                                ),
                                span,
                            ));
                        }
                        vars.insert(param.name.clone(), actual);
                    }
                    return self.resolve(&alias.body, &vars, depth + 1, span);
                }
                Err(self.error("S105", format!("unknown type '{name}'"), span))
            }
        }
    }
    fn expect(&self, expected: &Type, actual: &Type, span: Span) -> Result<(), Diagnostic> {
        if expected.accepts(actual) {
            Ok(())
        } else {
            Err(self.error("S106",format!("type mismatch: expected {expected}, found {actual}. Units and record fields must match; no lossy conversion is applied."),span))
        }
    }
    fn block(
        &mut self,
        body: &[Statement],
        env: &mut Environment,
        result: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<bool, Diagnostic> {
        let mut returned = false;
        for statement in body {
            if returned {
                return Err(self.error(
                    "S112",
                    "unreachable statement after return",
                    statement.span,
                ));
            }
            match &statement.kind {
                StatementKind::Bind {
                    kind,
                    name,
                    annotation,
                    value,
                } => {
                    self.fresh(name, env, statement.span)?;
                    let expected = annotation
                        .as_ref()
                        .map(|t| self.resolve(t, variables, 0, statement.span))
                        .transpose()?;
                    let ty = self.expression(value, env, expected.as_ref(), variables)?;
                    if let Some(expected) = &expected {
                        self.expect(expected, &ty, value.span)?;
                    }
                    let ty = expected.unwrap_or(ty);
                    if ty.unresolved() {
                        return Err(self.error("S107",format!("cannot infer a complete static type for '{name}'; add an annotation, for example let {name}: List<Number> = [] or let {name}: Option<Number> = None"),statement.span));
                    }
                    if ty.contains_function() {
                        return Err(self.error("S111","functions cannot be copied into bindings or containers; call them directly so the compiler can verify the complete call graph is acyclic",statement.span));
                    }
                    self.binding_types
                        .insert(statement.span.start, self.copy_type(&ty, statement.span)?);
                    self.bind_kind(name, *kind);
                    env.insert(name.clone(), ty);
                }
                StatementKind::Assign { name, value } => {
                    let expected = self.writable(name, env, statement.span)?;
                    let actual = self.expression(value, env, Some(&expected), variables)?;
                    self.expect(&expected, &actual, value.span)?;
                    if !self.root && self.function.is_none() {
                        return Err(self.error("S115", "imported modules may initialize mutable cells but cannot reassign them while importing; perform the edit in a helper called from the root script", statement.span));
                    }
                }
                StatementKind::Animate {
                    name,
                    value,
                    duration,
                    easing,
                } => {
                    let expected = self.writable(name, env, statement.span)?;
                    if !matches!(
                        expected,
                        Type::Numeric(_) | Type::Vector(2, Some(_)) | Type::Color
                    ) {
                        return Err(self.error("S131", format!("animate requires a numeric, Vec2, or Color mut binding, found {expected}"), statement.span));
                    }
                    let actual = self.expression(value, env, Some(&expected), variables)?;
                    self.expect(&expected, &actual, value.span)?;
                    let duration_type = self.expression(
                        duration,
                        env,
                        Some(&Type::Numeric(Unit::Duration)),
                        variables,
                    )?;
                    self.expect(
                        &Type::Numeric(Unit::Duration),
                        &duration_type,
                        duration.span,
                    )?;
                    if let Some(easing) = easing {
                        let easing_type =
                            self.expression(easing, env, Some(&Type::String), variables)?;
                        self.expect(&Type::String, &easing_type, easing.span)?;
                        if let ExprKind::String(name) = &easing.kind {
                            sanim_core::Easing::parse(name)
                                .map_err(|message| self.error("S131", message, easing.span))?;
                        }
                    }
                    if let Some(function) = &self.function {
                        self.effects.insert(function.clone());
                    }
                    if !self.root && self.function.is_none() {
                        return Err(self.error("S115", "imported modules cannot animate cells during import; call a helper from the root script", statement.span));
                    }
                }
                StatementKind::Background { body } => {
                    if crate::effects::has_return(body) {
                        return Err(self.error("S132", "background jobs cannot return from their enclosing function; finish the bounded job body, then return on the main timeline", statement.span));
                    }
                    self.scoped_block(
                        body,
                        &mut self.copy_environment(env, statement.span)?,
                        result,
                        variables,
                    )?;
                }
                StatementKind::Expr(expr) => {
                    let ty = self.expression(expr, env, None, variables)?;
                    if ty != Type::Unit {
                        return Err(self.error("S114",format!("unused {ty:?} value; bind it with 'let' or schedule it with play/add"),expr.span));
                    }
                    if !self.root && self.function.is_none() {
                        return Err(self.error("S115","imported modules may define helpers but cannot execute scheduling or assertion statements",statement.span));
                    }
                }
                StatementKind::Return(expr) => {
                    let Some(expected) = result else {
                        return Err(self.error(
                            "S110",
                            "return is only valid inside fn",
                            statement.span,
                        ));
                    };
                    let ty = self.expression(expr, env, Some(expected), variables)?;
                    self.expect(expected, &ty, expr.span)?;
                    returned = true;
                }
                StatementKind::If { condition, yes, no } => {
                    let ty = self.expression(condition, env, Some(&Type::Bool), variables)?;
                    self.expect(&Type::Bool, &ty, condition.span)?;
                    let a = self.scoped_block(
                        yes,
                        &mut self.copy_environment(env, statement.span)?,
                        result,
                        variables,
                    )?;
                    let b = self.scoped_block(
                        no,
                        &mut self.copy_environment(env, statement.span)?,
                        result,
                        variables,
                    )?;
                    returned = a && b;
                }
                StatementKind::For {
                    name,
                    iterable,
                    body,
                } => {
                    self.fresh(name, env, statement.span)?;
                    let ty = self.expression(iterable, env, None, variables)?;
                    let inner = match ty {
                        Type::List(t) | Type::Array(t) => *t,
                        _ => {
                            return Err(self.error(
                                "S116",
                                "for requires a finite List or Array",
                                iterable.span,
                            ));
                        }
                    };
                    let mut scope = self.copy_environment(env, statement.span)?;
                    scope.insert(name.clone(), inner);
                    self.scoped_block(body, &mut scope, result, variables)?;
                }
                StatementKind::Repeat {
                    count,
                    period,
                    body,
                } => {
                    let ty = self.expression(count, env, None, variables)?;
                    self.expect(&Type::Numeric(Unit::Number), &ty, count.span)?;
                    if let Some(period) = period {
                        let ty = self.expression(period, env, None, variables)?;
                        self.expect(&Type::Numeric(Unit::Duration), &ty, period.span)?;
                    }
                    self.scoped_block(
                        body,
                        &mut self.copy_environment(env, statement.span)?,
                        result,
                        variables,
                    )?;
                }
                StatementKind::Match { value, cases } => {
                    let ty = self.expression(value, env, None, variables)?;
                    let expected_enum = ty.clone();
                    let Type::Enum { variants, .. } = ty else {
                        return Err(self.error("S117", "match requires an enum value", value.span));
                    };
                    let mut covered = BTreeSet::new();
                    let mut all = true;
                    for case in cases {
                        if let Some(qualifier) = &case.qualifier {
                            let resolved = self.resolve(
                                &TypeExpr::Named(qualifier.clone(), vec![]),
                                variables,
                                0,
                                case.span,
                            )?;
                            self.expect(&expected_enum, &resolved, case.span)?;
                        }
                        let fields = variants.get(&case.variant).ok_or_else(|| {
                            self.error(
                                "S117",
                                format!("unknown variant '{}'", case.variant),
                                case.span,
                            )
                        })?;
                        if !covered.insert(case.variant.clone()) {
                            return Err(self.error(
                                "S117",
                                format!("duplicate case '{}'", case.variant),
                                case.span,
                            ));
                        }
                        if fields.len() != case.bindings.len() {
                            return Err(self.error(
                                "S117",
                                format!(
                                    "case '{}' requires {} bindings",
                                    case.variant,
                                    fields.len()
                                ),
                                case.span,
                            ));
                        }
                        let mut scope = self.copy_environment(env, statement.span)?;
                        for (name, ty) in case.bindings.iter().zip(fields) {
                            self.fresh(name, &scope, case.span)?;
                            scope.insert(name.clone(), ty.clone());
                        }
                        all &= self.scoped_block(&case.body, &mut scope, result, variables)?;
                    }
                    let missing = variants
                        .keys()
                        .filter(|v| !covered.contains(*v))
                        .cloned()
                        .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        return Err(self.error(
                            "S117",
                            format!("non-exhaustive match; add cases for {}", missing.join(", ")),
                            value.span,
                        ));
                    }
                    returned = all;
                }
                _ => {
                    return Err(self.error(
                        "S118",
                        "definitions and imports are only valid at module scope",
                        statement.span,
                    ));
                }
            }
        }
        Ok(returned)
    }
    fn expression(
        &mut self,
        expr: &Expr,
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        match &expr.kind {
            ExprKind::Reflect { value, options } => {
                let actual = self.expression(value, env, expected, variables)?;
                let ty = expected
                    .filter(|ty| crate::reflection::concrete_schema(ty) && ty.accepts(&actual))
                    .cloned()
                    .unwrap_or(actual);
                if !crate::reflection::concrete_schema(&ty) {
                    return Err(self.error("S113","cannot infer a concrete static type for this reflected default; symbolic generic parameters and untyped empty values cannot be exposed. Add a concrete annotation or explicit generic arguments, for example let optional:Option<Number>=reflect(None)",expr.span));
                }
                if !ty.reflectable() {
                    return Err(self.error("S113",format!("{ty} cannot be exposed as an editable input; reflect a concrete data value such as Color, String, Number, a list, or a record"),expr.span));
                }
                let name=self.reflection_names.get(&expr.span.start).ok_or_else(||self.error("S113","reflect(...) must execute exactly once at module scope; bind the input above the function, loop, or branch",expr.span))?;
                let declaration = options
                    .resolve(self.source, name, ty.clone(), expr.span)
                    .map_err(|message| self.error("S113", message, expr.span))?;
                self.reflected.insert(expr.span.start, declaration);
                Ok(ty)
            }
            ExprKind::Number(text) => Ok(Type::Numeric(
                Number::parse(text)
                    .map_err(|e| self.error("S119", e, expr.span))?
                    .unit,
            )),
            ExprKind::String(_) => Ok(Type::String),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Color(text) => {
                sanim_core::Color::parse(text).map_err(|e| self.error("S119", e, expr.span))?;
                Ok(Type::Color)
            }
            ExprKind::None => Ok(expected
                .filter(|t| matches!(t, Type::Option(_)))
                .cloned()
                .unwrap_or(Type::Option(Box::new(Type::Unknown)))),
            ExprKind::Name(name) => {
                if let Some(value) = prelude::values().get(name.as_str()) {
                    Ok(value.ty())
                } else {
                    env.get(name).ok_or_else(||self.error("S120",format!("unknown name '{name}'. Standard constructors such as Circle and Color need no import; user names are case-sensitive."),expr.span)).and_then(|ty|self.copy_type(ty,expr.span))
                }
            }
            ExprKind::List(items) => {
                let wanted = match expected {
                    Some(Type::List(t)) => Some(t.as_ref()),
                    _ => None,
                };
                let mut inner = wanted.cloned().unwrap_or(Type::Unknown);
                for item in items {
                    let ty = self.expression(item, env, wanted, variables)?;
                    if inner == Type::Unknown {
                        inner = ty;
                    } else {
                        self.expect(&inner, &ty, item.span)?;
                    }
                }
                Ok(Type::List(Box::new(inner)))
            }
            ExprKind::Record(fields) => {
                let mut out = BTreeMap::new();
                for (name, value) in fields {
                    if out.contains_key(name) {
                        return Err(self.error(
                            "S103",
                            format!("duplicate field '{name}'"),
                            value.span,
                        ));
                    }
                    let wanted = if let Some(Type::Record(fields)) = expected {
                        fields.get(name)
                    } else {
                        None
                    };
                    out.insert(
                        name.clone(),
                        self.expression(value, env, wanted, variables)?,
                    );
                }
                Ok(Type::Record(out))
            }
            ExprKind::Dict(entries) => {
                for (key, value) in entries {
                    let key_type = self.expression(key, env, Some(&Type::String), variables)?;
                    self.expect(&Type::String, &key_type, key.span)?;
                    self.data_expression(value, env, variables)?;
                }
                Ok(Type::Dict)
            }
            ExprKind::Unary(op, value) => {
                let ty = self.expression(value, env, None, variables)?;
                sanim_core::math::operations::unary_type(op, &ty)
                    .map_err(|e| self.error("S106", e, expr.span))
            }
            ExprKind::Binary(op, a, b) => {
                let left = self.expression(a, env, None, variables)?;
                let right = self.expression(b, env, Some(&left), variables)?;
                sanim_core::math::operations::binary_type(op, &left, &right)
                    .map_err(|e| self.error("S106", e, expr.span))
            }
            ExprKind::Member(base, name) => {
                let ty = self.expression(base, env, None, variables)?;
                crate::type_algebra::member(ty, name)
                    .map_err(|message| self.error("S121", message, expr.span))
            }
            ExprKind::Index(base, index) => {
                let ty = self.expression(base, env, None, variables)?;
                let idx = self.expression(index, env, None, variables)?;
                if matches!(ty, Type::Dict | Type::Data) {
                    if ty == Type::Dict {
                        self.expect(&Type::String, &idx, index.span)?;
                    } else if idx != Type::String && idx != Type::Numeric(Unit::Number) {
                        return Err(self.error(
                            "S122",
                            "Data keys must be String or array indices must be Number",
                            index.span,
                        ));
                    }
                    return Ok(Type::Data);
                }
                self.expect(&Type::Numeric(Unit::Number), &idx, index.span)?;
                let inner = match ty {
                    Type::List(t) | Type::Array(t) => t,
                    _ => {
                        return Err(self.error(
                            "S122",
                            "indexing requires a finite collection",
                            expr.span,
                        ));
                    }
                };
                // Indexing is total: a possibly absent item is always Option<T>.
                Ok(Type::Option(inner))
            }
            ExprKind::Call { .. } => self.call(expr, env, expected, variables),
        }
    }
    fn call(
        &mut self,
        expr: &Expr,
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let ExprKind::Call {
            callee,
            types,
            args,
        } = &expr.kind
        else {
            unreachable!()
        };
        if !direct_callee(callee) {
            return Err(self.error("S111","indirect function calls are forbidden; call a declared function or imported module function directly so the compiler can verify the call graph",callee.span));
        }
        if let ExprKind::Member(base, name) = &callee.kind {
            let receiver = self.expression(base, env, None, variables)?;
            if receiver == Type::Element
                && prelude::builtin(name).is_some_and(|v| v.result == Type::Predicate)
            {
                if !types.is_empty() {
                    return Err(self.error(
                        "S123",
                        "relationship methods take no type arguments",
                        expr.span,
                    ));
                }
                let mut all = vec![(None, *base.clone())];
                all.extend_from_slice(args);
                return self.native(expr, name, &all, env, expected, variables);
            }
        }
        if let ExprKind::Name(name) = &callee.kind
            && prelude::builtin(name).is_some()
        {
            return self.native(expr, name, args, env, expected, variables);
        }
        let ty = self.expression(callee, env, None, variables)?;
        let Type::Function(signature) = ty else {
            return Err(self.error("S123", format!("cannot call {ty:?}"), callee.span));
        };
        if let Some(function) = &self.function {
            if signature.schedules {
                self.effects.insert(function.clone());
            }
            if signature.mutates {
                self.mutations.insert(function.clone());
            }
        }
        if let (Some(function), ExprKind::Name(name)) = (&self.function, &callee.kind) {
            self.calls
                .entry(function.clone())
                .or_default()
                .insert(name.clone());
        }
        let ordered = self.arguments(
            args,
            &signature
                .parameters
                .iter()
                .enumerate()
                .map(|(i, (n, t))| {
                    (
                        n.as_str(),
                        t.clone(),
                        i < signature.parameters.len() - signature.default_count,
                    )
                })
                .collect::<Vec<_>>(),
            expr.span,
        )?;
        let mut substitutions = BTreeMap::new();
        if !types.is_empty() {
            if types.len() > signature.generics.len() {
                return Err(self.error(
                    "S123",
                    format!("expected {} type arguments", signature.generics.len()),
                    expr.span,
                ));
            }
            for (parameter, ty) in signature.generics.iter().zip(types) {
                substitutions.insert(
                    parameter.name.clone(),
                    self.resolve(ty, variables, 0, expr.span)?,
                );
            }
        }
        if let Some(expected) = expected {
            infer(
                &signature.result,
                expected,
                &mut substitutions,
                self.type_budget,
            )
            .map_err(|e| self.error("S106", e, expr.span))?;
        }
        for ((_, parameter), value) in signature.parameters.iter().zip(ordered) {
            let Some(value) = value else {
                continue;
            };
            let wanted = self.substitute(parameter, &substitutions, expr.span)?;
            let actual = self.expression(value, env, Some(&wanted), variables)?;
            if actual.contains_function() {
                return Err(self.error("S111","function values cannot be passed as arguments or stored in containers; call the function directly",value.span));
            }
            infer(parameter, &actual, &mut substitutions, self.type_budget)
                .map_err(|e| self.error("S106", e, value.span))?;
            self.expect(
                &self.substitute(parameter, &substitutions, value.span)?,
                &actual,
                value.span,
            )?;
        }
        for parameter in &signature.generics {
            if !substitutions.contains_key(&parameter.name) {
                let default=parameter.default.as_ref().ok_or_else(||self.error("S123",format!("cannot infer type parameter '{}'; provide explicit <Type> arguments or declare a default",parameter.name),expr.span))?;
                substitutions.insert(
                    parameter.name.clone(),
                    self.substitute(default, &substitutions, expr.span)?,
                );
            }
            let actual = &substitutions[&parameter.name];
            if let Some(bound) = &parameter.constraint
                && !bound.substitute(&substitutions).accepts(actual)
            {
                return Err(self.error(
                    "S106",
                    format!(
                        "type {actual} does not satisfy constraint {bound:?} for '{}'",
                        parameter.name
                    ),
                    expr.span,
                ));
            }
        }
        let stored = self.copy_environment(&substitutions, expr.span)?;
        self.call_types.insert(expr.span.start, stored);
        self.substitute(&signature.result, &substitutions, expr.span)
    }
    fn arguments<'a>(
        &self,
        args: &'a [(Option<String>, Expr)],
        parameters: &[(&str, Type, bool)],
        span: Span,
    ) -> Result<Vec<Option<&'a Expr>>, Diagnostic> {
        let mut ordered = vec![None; parameters.len()];
        let mut positional = 0;
        let mut named = false;
        for (name, value) in args {
            let index = if let Some(name) = name {
                named = true;
                parameters
                    .iter()
                    .position(|(n, _, _)| *n == name)
                    .ok_or_else(|| {
                        self.error(
                            "S124",
                            format!(
                                "unknown argument '{name}'; accepted: {}",
                                parameters
                                    .iter()
                                    .map(|p| p.0)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            value.span,
                        )
                    })?
            } else {
                if named {
                    return Err(self.error(
                        "S124",
                        "positional arguments must precede named arguments",
                        value.span,
                    ));
                }
                let i = positional;
                positional += 1;
                i
            };
            if index >= ordered.len() {
                return Err(self.error("S124", "too many arguments", value.span));
            }
            if ordered[index].is_some() {
                return Err(self.error(
                    "S124",
                    format!("argument '{}' supplied twice", parameters[index].0),
                    value.span,
                ));
            }
            ordered[index] = Some(value);
        }
        for ((name, _, required), value) in parameters.iter().zip(&ordered) {
            if *required && value.is_none() {
                return Err(self.error(
                    "S124",
                    format!("missing required argument '{name}'"),
                    span,
                ));
            }
        }
        Ok(ordered)
    }
    fn data_expression(
        &mut self,
        expr: &Expr,
        env: &Environment,
        variables: &BTreeMap<String, Type>,
    ) -> Result<(), Diagnostic> {
        match &expr.kind {
            ExprKind::None => Ok(()),
            ExprKind::List(items) => {
                for item in items {
                    self.data_expression(item, env, variables)?;
                }
                Ok(())
            }
            ExprKind::Record(fields) => {
                let mut names = BTreeSet::new();
                for (name, value) in fields {
                    if !names.insert(name) {
                        return Err(self.error(
                            "S103",
                            format!("duplicate field '{name}'"),
                            value.span,
                        ));
                    }
                    self.data_expression(value, env, variables)?;
                }
                Ok(())
            }
            _ => {
                let ty = self.expression(expr, env, None, variables)?;
                if !ty.data_convertible() {
                    return Err(self.error(
                        "S106",
                        format!("{ty} cannot be converted to unstructured data"),
                        expr.span,
                    ));
                }
                Ok(())
            }
        }
    }
    fn native(
        &mut self,
        expr: &Expr,
        name: &str,
        args: &[(Option<String>, Expr)],
        env: &Environment,
        expected: Option<&Type>,
        variables: &BTreeMap<String, Type>,
    ) -> Result<Type, Diagnostic> {
        let ExprKind::Call { types, .. } = &expr.kind else {
            unreachable!()
        };
        let builtin = prelude::builtin(name)
            .ok_or_else(|| self.error("S120", format!("unknown function '{name}'"), expr.span))?;
        if builtin.schedule
            && let Some(function) = &self.function
        {
            self.effects.insert(function.clone());
        }
        if builtin.result == Type::Data && builtin.name == "Data" {
            if !types.is_empty() {
                return Err(self.error("S123", "Data does not take type arguments", expr.span));
            }
            let ordered = self.arguments(args, &[("value", Type::Unknown, true)], expr.span)?;
            self.data_expression(ordered[0].expect("required data argument"), env, variables)?;
            return Ok(Type::Data);
        }
        if matches!(
            builtin.execute,
            prelude::Execution::Decode | prelude::Execution::Query
        ) {
            if matches!(builtin.execute, prelude::Execution::Decode)
                && types.is_empty()
                && expected.is_none()
            {
                return Err(self.error(
                    "S125",
                    "write decode<ResultType>(data) or annotate the binding",
                    expr.span,
                ));
            }
            let result = if types.len() == 1 {
                self.resolve(&types[0], variables, 0, expr.span)?
            } else if types.is_empty() {
                let expected = expected
                    .cloned()
                    .unwrap_or(Type::List(Box::new(Type::Dict)));
                if matches!(builtin.execute, prelude::Execution::Query) {
                    let Type::List(row) = expected else {
                        return Err(self.error("S125", "query produces List<Row>", expr.span));
                    };
                    *row
                } else {
                    expected
                }
            } else {
                return Err(self.error(
                    "S125",
                    format!("{name} requires one type argument"),
                    expr.span,
                ));
            };
            if !result.readable() || result == Type::Font {
                return Err(self.error(
                    "S125",
                    format!("{result} is not a data schema"),
                    expr.span,
                ));
            }
            let params = builtin
                .parameters
                .iter()
                .map(|p| (p.name, p.ty.clone(), p.required))
                .collect::<Vec<_>>();
            let ordered = self.arguments(args, &params, expr.span)?;
            let mut actuals = vec![];
            for (param, arg) in builtin.parameters.iter().zip(ordered) {
                if let Some(arg) = arg {
                    let actual = self.expression(arg, env, Some(&param.ty), variables)?;
                    self.expect(&param.ty, &actual, arg.span)?;
                    actuals.push(actual);
                } else {
                    actuals.push(Type::Unit);
                }
            }
            if matches!(builtin.execute, prelude::Execution::Decode) {
                if !matches!(actuals[0], Type::Data | Type::Dict) {
                    return Err(self.error("S106", "decode<T> requires Data or Dict", expr.span));
                }
                self.reads.insert(expr.span.start, result.clone());
                return Ok(result);
            }
            sanim_data::sqlite::validate_row_type(&result)
                .map_err(|message| self.error("S125", message, expr.span))?;
            let Type::Record(tables) = &actuals[1] else {
                return Err(self.error(
                    "S106",
                    "query inputs must be named lists, for example tables={samples=samples}",
                    expr.span,
                ));
            };
            for (name, table) in tables {
                let Type::List(row) = table else {
                    return Err(self.error(
                        "S106",
                        format!("SQL input '{name}' must be a List<T>, found {table}"),
                        expr.span,
                    ));
                };
                sanim_data::sqlite::validate_row_type(row).map_err(|message| {
                    self.error("S125", format!("SQL input '{name}': {message}"), expr.span)
                })?;
            }
            if !matches!(actuals[2], Type::Unit | Type::Dict | Type::Record(_)) {
                return Err(self.error(
                    "S106",
                    "SQL params must be Dict or a named record",
                    expr.span,
                ));
            }
            let result = Type::List(Box::new(result));
            self.reads.insert(expr.span.start, result.clone());
            return Ok(result);
        }
        if matches!(builtin.execute, prelude::Execution::Read) {
            let ty = if types.len() == 1 {
                self.resolve(&types[0], variables, 0, expr.span)?
            } else if types.is_empty() {
                expected.cloned().ok_or_else(||self.error("S125","cannot infer read result. Write read<List<Row>>(\"file.csv\") or annotate the binding.",expr.span))?
            } else {
                return Err(self.error("S125", "read requires one type argument", expr.span));
            };
            if !ty.readable() {
                return Err(self.error(
                    "S125",
                    format!("{ty:?} is not a readable data type"),
                    expr.span,
                ));
            }
            let ordered = self.arguments(args, &[("path", Type::String, true)], expr.span)?;
            let path = ordered[0].ok_or_else(|| self.error("S125", "missing path", expr.span))?;
            let actual = self.expression(path, env, Some(&Type::String), variables)?;
            self.expect(&Type::String, &actual, path.span)?;
            self.reads.insert(expr.span.start, ty.clone());
            return Ok(ty);
        }
        if !types.is_empty() {
            return Err(self.error(
                "S123",
                format!("'{name}' does not accept explicit type arguments"),
                expr.span,
            ));
        }
        let mut normal = vec![];
        for (name, value) in args {
            if matches!(builtin.execute, prelude::Execution::Play) && name.is_none() {
                let ty = self.expression(value, env, Some(&Type::Motion), variables)?;
                self.expect(&Type::Motion, &ty, value.span)?;
            } else {
                normal.push((name.clone(), value.clone()));
            }
        }
        if matches!(builtin.execute, prelude::Execution::Play)
            && args.iter().all(|(name, _)| name.is_some())
        {
            return Err(self.error("S124", "play requires at least one motion", expr.span));
        }
        let parameters = builtin
            .parameters
            .iter()
            .map(|p| (p.name, p.ty.clone(), p.required))
            .collect::<Vec<_>>();
        let ordered = self.arguments(&normal, &parameters, expr.span)?;
        let mut actuals = vec![];
        for (parameter, value) in builtin.parameters.iter().zip(ordered) {
            if let Some(value) = value {
                let ty = self.expression(value, env, Some(&parameter.ty), variables)?;
                if ty.contains_function() {
                    return Err(self.error("S111","function values cannot be passed as arguments or stored in containers; call the function directly",value.span));
                }
                self.expect(&parameter.ty, &ty, value.span)?;
                actuals.push(ty);
            } else {
                actuals.push(Type::Unit);
            }
        }
        (builtin.check)(builtin, &actuals)
            .map_err(|e| self.error("S106", format!("{name}: {e}"), expr.span))
    }
}

fn infer(
    parameter: &Type,
    actual: &Type,
    bindings: &mut BTreeMap<String, Type>,
    budget: &std::cell::Cell<usize>,
) -> Result<(), String> {
    match (parameter, actual) {
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
                let mut remaining = budget.get();
                sanim_core::type_budget::charge(actual, &mut remaining)?;
                budget.set(remaining);
                bindings.insert(name.clone(), actual.clone());
            }
        }
        (Type::List(a), Type::List(b))
        | (Type::Option(a), Type::Option(b))
        | (Type::Array(a), Type::Array(b)) => infer(a, b, bindings, budget)?,
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

fn generic_variable(parameter: &GenericParameter<Type, TypeBound>) -> Type {
    parameter.constraint.as_ref().map_or_else(
        || Type::Variable(parameter.name.clone()),
        |bound| Type::BoundedVariable(parameter.name.clone(), Box::new(bound.clone())),
    )
}
fn generic_variables(parameters: &[GenericParameter<Type, TypeBound>]) -> BTreeMap<String, Type> {
    parameters
        .iter()
        .map(|p| (p.name.clone(), generic_variable(p)))
        .collect()
}

fn direct_callee(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(_) => true,
        ExprKind::Member(base, _) => direct_callee(base),
        _ => false,
    }
}

impl Checked {
    pub(crate) fn charge_types(&self, budget: &std::cell::Cell<usize>) -> Result<(), String> {
        let mut remaining = budget.get();
        for ty in self
            .exports
            .values()
            .chain(self.schemas.values())
            .chain(self.read_types.values())
            .chain(self.binding_types.values())
            .chain(
                self.reflected
                    .values()
                    .map(|declaration| declaration.ty.as_ref()),
            )
            .chain(self.call_types.values().flat_map(|values| values.values()))
        {
            sanim_core::type_budget::charge(ty, &mut remaining)?;
        }
        budget.set(remaining);
        Ok(())
    }
}

fn check_signal_purity(
    checker: &Checker<'_>,
    statement: &Statement,
    env: &Environment,
) -> Result<(), Diagnostic> {
    match &statement.kind {
        StatementKind::Bind {
            kind: BindingKind::Sig,
            value,
            ..
        } => {
            let mut invalid = None;
            value.visit(&mut |expr| {
                if matches!(expr.kind, ExprKind::Reflect { .. })
                    || checker.call_state_effect(expr, env)
                {
                    invalid.get_or_insert(expr.span);
                }
            });
            if let Some(span) = invalid {
                return Err(checker.error("S131", "sig expressions must be pure: they cannot mutate captured cells, schedule animations, assert, or declare reflected inputs. Bind reflection with let/mut first, then derive a sig from that value.", span));
            }
        }
        StatementKind::Function { body, .. }
        | StatementKind::Background { body }
        | StatementKind::For { body, .. }
        | StatementKind::Repeat { body, .. } => {
            for child in body {
                check_signal_purity(checker, child, env)?;
            }
        }
        StatementKind::If { yes, no, .. } => {
            for child in yes.iter().chain(no) {
                check_signal_purity(checker, child, env)?;
            }
        }
        StatementKind::Match { cases, .. } => {
            for child in cases.iter().flat_map(|case| &case.body) {
                check_signal_purity(checker, child, env)?;
            }
        }
        _ => {}
    }
    Ok(())
}
