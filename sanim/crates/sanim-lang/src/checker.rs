mod clauses;
mod dataframe;
mod list_functions;
mod lists;
mod native;
mod templates;
use crate::{ast::*, prelude};
use clauses::infer;
use sanim_core::{Diagnostic, GenericParameter, Number, Signature, Span, Type, TypeBound, Unit};
use std::collections::{BTreeMap, BTreeSet};

pub type Environment = BTreeMap<String, Type>;
#[derive(Clone, Debug)]
pub struct Alias {
    pub parameters: Vec<GenericParameter<Type, TypeBound>>,
    pub body: TypeExpr,
}
/// Resolved once from the checked receiver, shared by runtime and editor tooling.
#[derive(Clone, Debug)]
pub struct NativeMethod {
    pub name: String,
    pub materializes: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Checked {
    /// Statement start -> index in the named export's canonical signatures.
    pub function_clauses: BTreeMap<usize, usize>,
    pub exports: Environment,
    pub schemas: BTreeMap<String, Type>,
    pub result_types: BTreeMap<(usize, usize), Type>,
    pub template_expressions: BTreeMap<(usize, usize), Type>,
    pub native_methods: BTreeMap<(usize, usize), NativeMethod>,
    pub call_types: BTreeMap<(usize, usize), BTreeMap<String, Type>>,
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
        result_types: BTreeMap::new(),
        template_expressions: BTreeMap::new(),
        native_methods: BTreeMap::new(),
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
                                .map(|(i, t)| sanim_core::FunctionParameter {
                                    name: format!("value{i}"),
                                    ty: t,
                                    kind: BindingKind::Let,
                                })
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
                        parameters: fields
                            .iter()
                            .map(|(k, v)| sanim_core::FunctionParameter {
                                name: k.clone(),
                                ty: v.clone(),
                                kind: BindingKind::Let,
                            })
                            .collect(),
                        result: Box::new(ty),
                    }),
                );
            }
        }
    }
    let function_clauses = checker.declare_clauses(module, &mut env)?;
    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. } => {}
            StatementKind::Function {
                name,
                generics: _,
                parameters,
                duration,
                body,
                ..
            } => {
                let declaration = &env[name].signatures()[function_clauses[&statement.span.start]];
                let declaration_variables = generic_variables(&declaration.generics);
                checker.function = Some(name.clone());
                let mut saw_default = false;
                for parameter in parameters {
                    let ty = &parameter.ty;
                    let default = &parameter.default;
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
                let signature =
                    env[name].signatures()[function_clauses[&statement.span.start]].clone();
                let mut scope = checker.copy_environment(&env, statement.span)?;
                for parameter in &signature.parameters {
                    let name = &parameter.name;
                    let ty = &parameter.ty;
                    checker.fresh(name, &scope, statement.span)?;
                    scope.insert(
                        name.clone(),
                        if parameter.kind == BindingKind::Sig {
                            ty.clone().signal_schema()
                        } else {
                            ty.clone()
                        },
                    );
                }
                checker.function = Some(name.clone());
                let variables = generic_variables(&signature.generics);
                if let Some(duration) = duration {
                    let actual = checker.expression(
                        duration,
                        &scope,
                        Some(&Type::Numeric(Unit::Duration)),
                        &variables,
                    )?;
                    checker.expect(&Type::Numeric(Unit::Duration), &actual, duration.span)?;
                }
                checker.binding_scopes.push(BTreeMap::new());
                for parameter in &signature.parameters {
                    checker.bind_kind(&parameter.name, parameter.kind);
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
        for signature in ty.signatures_mut() {
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
        if let StatementKind::Function {
            duration: Some(duration),
            ..
        } = &statement.kind
        {
            let mut invalid = None;
            duration.visit(&mut |expression| {
                if checker.call_state_effect(expression, &env) {
                    invalid.get_or_insert(expression.span);
                }
            });
            if let Some(span) = invalid {
                return Err(checker.error("S128","@duration(...) must be a pure Duration expression; it must not schedule timeline work or mutate state",span));
            }
        }
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
    let duration_analysis = crate::effects::analyze(
        source,
        module,
        &env,
        imports,
        &checker.native_methods,
        type_budget,
        limits,
    )?;
    for statement in &module.statements {
        if let StatementKind::Function { name, .. } = &statement.kind {
            let signature = &mut env
                .get_mut(name)
                .expect("checked function")
                .signatures_mut()[function_clauses[&statement.span.start]];
            signature.duration = duration_analysis.clauses[&statement.span.start].clone();
            signature.schedules = checker.effects.contains(name);
            signature.mutates = checker.mutations.contains(name);
            signature.duration_parameters = signature
                .parameters
                .iter()
                .enumerate()
                .filter(|(_, parameter)| duration_analysis.needed.contains(&parameter.name))
                .map(|(index, _)| index)
                .collect();
        }
    }
    Ok(Checked {
        function_clauses,
        durations: duration_analysis.nodes,
        duration_needed: duration_analysis.needed,
        value_dependencies: duration_analysis.value_dependencies,
        exports: env,
        schemas: checker.schemas,
        result_types: checker.result_types,
        template_expressions: checker.template_expressions,
        native_methods: checker.native_methods,
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
    result_types: BTreeMap<(usize, usize), Type>,
    template_expressions: BTreeMap<(usize, usize), Type>,
    native_methods: BTreeMap<(usize, usize), NativeMethod>,
    call_types: BTreeMap<(usize, usize), BTreeMap<String, Type>>,
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
        let ExprKind::Call { callee, args, .. } = &expr.kind else {
            return false;
        };
        if crate::timing::builtin_call(callee, args, &self.native_methods).is_some_and(|call| {
            call.schedules()
                || crate::list_functions::callback_type(&call, env)
                    .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.schedules))
        }) || matches!(&callee.kind,ExprKind::Name(name) if self.effects.contains(name))
        {
            return true;
        }
        crate::effects::lookup(callee, env).is_some_and(|ty| {
            ty.signatures().iter().any(|signature| {
                signature.schedules
                    || !signature
                        .parameters
                        .iter()
                        .any(|parameter| parameter.name == "duration")
                        && crate::timing::envelope(args)
                            .is_some_and(|expr| !crate::timing::known_zero(expr))
            })
        })
    }
    fn call_mutates(&self, expr: &Expr, env: &Environment) -> bool {
        let ExprKind::Call { callee, args, .. } = &expr.kind else {
            return false;
        };
        matches!(&callee.kind, ExprKind::Name(name) if self.mutations.contains(name))
            || crate::timing::builtin_call(callee, args, &self.native_methods).is_some_and(|call| {
                crate::list_functions::callback_type(&call, env)
                    .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
            })
            || crate::effects::lookup(callee, env)
                .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
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
            TypeExpr::ConstLength(_)=>return Err(self.error("S105","a numeric length is only valid as the second List type argument, for example List<Number,3>",span)),
            TypeExpr::Refined(inner, _, _) => {
                self.validate_template(inner, variables, depth + 1, span)?
            }
            TypeExpr::StringLiteral(_) => false,
            TypeExpr::Signal(inner) => self.validate_template(inner, variables, depth + 1, span)?,
            TypeExpr::Named(name, args) => {
                if matches!(prelude::types().get(name.as_str()),Some(prelude::TypeDefinition::List)) {
                    let (inner,_)=self.list_type_arguments(args,span)?;
                    return self.validate_template(inner,variables,depth+1,span);
                }
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
        let ty = self.resolve_inner(expr, variables, depth, span)?;
        let nested = match &ty {
            Type::Signal(inner) => inner.contains_signal(),
            _ => ty.contains_signal(),
        };
        if nested {
            return Err(self.error("S106","containers hold value snapshots; place sig around the entire value type, such as sig List<Number>, rather than inside a container",span));
        }
        Ok(ty)
    }
    fn resolve_inner(
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
            TypeExpr::ConstLength(_)=>Err(self.error("S105","a numeric length is only valid as the second List type argument, for example List<Number,3>",span)),
            TypeExpr::Refined(inner, operator, bound) => {
                let base = self.resolve(inner, variables, depth + 1, span)?;
                let domain = base.numeric_domain().ok_or_else(|| {
                    self.error(
                        "S106",
                        format!(
                            "numeric refinement requires a concrete numeric type, found {base}"
                        ),
                        span,
                    )
                })?;
                domain
                    .constrained(operator, *bound)
                    .map(Type::RefinedNumeric)
                    .map_err(|reason| self.error("S106", reason, span))
            }
            TypeExpr::StringLiteral(value) => {
                self.type_budget.set(
                    self.type_budget
                        .get()
                        .checked_sub(value.len())
                        .ok_or_else(|| self.error("S105", "resolved type budget exceeded", span))?,
                );
                Ok(Type::StringLiteral(value.clone().into()))
            }
            TypeExpr::Signal(inner) => Ok(Type::Signal(Box::new(self.resolve(
                inner,
                variables,
                depth + 1,
                span,
            )?))),
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
                    if matches!(definition,prelude::TypeDefinition::List) {
                        let (inner,length)=self.list_type_arguments(args,span)?;
                        return Ok(Type::List(Box::new(self.resolve(inner,variables,depth+1,span)?),length));
                    }
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
    fn narrow_branch(
        &self,
        condition: &Expr,
        truth: bool,
        env: &mut Environment,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let ExprKind::Binary(operator, left, right) = &condition.kind else {
            return Ok(());
        };
        if !matches!(operator.as_str(), "==" | "!=") {
            return Ok(());
        }
        let (target, literal) = match (&left.kind, &right.kind) {
            (_, ExprKind::String(value)) => (left, value),
            (ExprKind::String(value), _) => (right, value),
            _ => return Ok(()),
        };
        let Some(qualified) = target.qualified_name() else {
            return Ok(());
        };
        let mut names = qualified.split('.');
        let name = names.next().expect("qualified name");
        let binding = self
            .binding_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name));
        if binding.is_none_or(|binding| binding.kind != BindingKind::Let) {
            return Ok(());
        }
        let Some(ty) = env.get(name) else {
            return Ok(());
        };
        if let Some(narrowed) = crate::type_algebra::narrow_string(
            ty,
            &names.collect::<Vec<_>>(),
            literal,
            truth == (operator == "=="),
        ) {
            let mut remaining = self.type_budget.get();
            sanim_core::type_budget::charge(&narrowed, &mut remaining)
                .map_err(|message| self.error("S105", message, span))?;
            self.type_budget.set(remaining);
            env.insert(name.into(), narrowed);
        }
        Ok(())
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
                    if *kind != BindingKind::Let && matches!(expected, Some(Type::Signal(_))) {
                        return Err(self.error("S106","sig and mut declarations already choose their binding mode; use sig name:Number or mut name:Number without repeating sig in the type",statement.span));
                    }
                    let actual_expected = expected.as_ref().map(|ty| {
                        if let Type::Signal(inner) = ty {
                            inner.as_ref()
                        } else {
                            ty
                        }
                    });
                    let mut ty = self.expression(value, env, actual_expected, variables)?;
                    if *kind == BindingKind::Let
                        && expected.is_none()
                        && let ExprKind::Call { callee, .. } = &value.kind
                        && crate::effects::lookup(callee, env).is_some_and(|ty| {
                            ty.signatures().iter().any(|signature| {
                                matches!(signature.result.as_ref(), Type::Signal(_))
                            })
                        })
                    {
                        ty = Type::Signal(Box::new(ty));
                    }
                    if *kind != BindingKind::Let
                        && let Type::Signal(inner) = ty
                    {
                        ty = *inner;
                    }
                    if let Some(expected) = &expected {
                        if let Type::Signal(inner) = expected {
                            self.expect(inner, &ty, value.span)?;
                        } else {
                            self.expect(expected, &ty, value.span)?;
                        }
                    }
                    let ty = expected.unwrap_or_else(|| {
                        if *kind == BindingKind::Let
                            && let ExprKind::String(value) = &value.kind
                        {
                            Type::StringLiteral(value.clone().into())
                        } else if *kind != BindingKind::Let {
                            ty.editable_schema()
                        } else {
                            ty
                        }
                    });
                    if ty.contains_signal() && !matches!(ty, Type::Signal(_)) {
                        return Err(self.error("S106","containers hold value snapshots; put sig around the entire value type instead, for example sig List<Number>",statement.span));
                    }
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
                        return Err(self.error("S114",format!("unused {ty:?} value; bind it with 'let', insert it with add, or pass it to an animation function"),expr.span));
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
                    let expected = if let Type::Signal(inner) = expected {
                        inner.as_ref()
                    } else {
                        expected
                    };
                    let ty = self.expression(expr, env, Some(expected), variables)?;
                    self.expect(expected, &ty, expr.span)?;
                    returned = true;
                }
                StatementKind::If { condition, yes, no } => {
                    let ty = self.expression(condition, env, Some(&Type::Bool), variables)?;
                    self.expect(&Type::Bool, &ty, condition.span)?;
                    let mut yes_scope = self.copy_environment(env, statement.span)?;
                    let mut no_scope = self.copy_environment(env, statement.span)?;
                    self.narrow_branch(condition, true, &mut yes_scope, statement.span)?;
                    self.narrow_branch(condition, false, &mut no_scope, statement.span)?;
                    let a = self.scoped_block(yes, &mut yes_scope, result, variables)?;
                    let b = self.scoped_block(no, &mut no_scope, result, variables)?;
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
                        Type::List(t, _) | Type::Array(t) => *t,
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
        let mut actual = self.expression_inner(expr, env, expected, variables)?;
        if !expected.is_some_and(|ty| matches!(ty, Type::Signal(_)))
            && let Type::Signal(inner) = actual
        {
            actual = *inner;
        }
        if expected == Some(&Type::String) && actual == Type::Template {
            self.template_expressions
                .insert((expr.span.start, expr.span.end), Type::String);
            Ok(Type::String)
        } else {
            Ok(actual)
        }
    }
    fn expression_inner(
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
                    .unwrap_or_else(|| actual.editable_schema());
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
            ExprKind::Number(text) => {
                let number = Number::parse(text).map_err(|e| self.error("S119", e, expr.span))?;
                Ok(if expected.is_some_and(clauses::refined_context) {
                    Type::RefinedNumeric(sanim_core::math::domain::NumericDomain::singleton(number))
                } else {
                    Type::Numeric(number.unit)
                })
            }
            ExprKind::String(value) => {
                if expected.is_some_and(|ty| ty.string_literals().is_some()) {
                    Ok(Type::StringLiteral(value.clone().into()))
                } else if crate::templates::expected(expected) {
                    self.template_expressions
                        .insert((expr.span.start, expr.span.end), Type::Template);
                    Ok(Type::Template)
                } else {
                    Ok(Type::String)
                }
            }
            ExprKind::Template(parts) => self.template(expr, parts, env, expected, variables),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::None => Ok(expected
                .filter(|t| matches!(t, Type::Option(_)))
                .cloned()
                .unwrap_or(Type::Option(Box::new(Type::Unknown)))),
            ExprKind::Name(name) => {
                if let Some(value) = prelude::values().get(name.as_str()) {
                    Ok(value.ty())
                } else {
                    env.get(name).ok_or_else(|| {
                        let hint=expected.and_then(Type::string_literals).filter(|values|values.contains(&name.as_str())).map(|_|format!(" Did you mean {name:?}? This argument expects a quoted string value ({expected}).",expected=expected.expect("literal context"))).unwrap_or_default();
                        self.error("S120",format!("unknown name '{name}'.{hint} Standard constructors such as Circle and Color need no import; user names are case-sensitive."),expr.span)
                    }).and_then(|ty|self.copy_type(if let Type::Signal(inner)=ty {inner}else{ty},expr.span))
                }
            }
            ExprKind::List(items) => self.list_literal(items, env, expected, variables),
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
                    let wanted = match expected {
                        Some(Type::Record(fields)) => fields.get(name).cloned(),
                        Some(Type::Union(types)) => Type::union(types.iter().filter_map(|ty| {
                            if let Type::Record(fields) = ty {
                                fields.get(name).cloned()
                            } else {
                                None
                            }
                        }))
                        .ok(),
                        _ => None,
                    };
                    out.insert(
                        name.clone(),
                        self.expression(value, env, wanted.as_ref(), variables)?,
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
                if expected.is_some_and(clauses::refined_context)
                    && let Some(number) = clauses::literal_number(expr)
                {
                    return Ok(Type::RefinedNumeric(
                        sanim_core::math::domain::NumericDomain::singleton(number),
                    ));
                }
                let ty = self.expression(value, env, None, variables)?;
                sanim_core::math::operations::unary_type(op, &ty)
                    .map_err(|e| self.error("S106", e, expr.span))
            }
            ExprKind::Binary(op, a, b) => {
                let left = self.expression(a, env, None, variables)?;
                let context = if op == "+" && matches!(left, Type::List(..)) {
                    Some(self.copy_type(&left, expr.span)?.without_list_lengths())
                } else {
                    None
                };
                let right =
                    self.expression(b, env, Some(context.as_ref().unwrap_or(&left)), variables)?;
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
                if let Type::DataFrame { row, groups } = &ty {
                    return self.frame_index(row, groups, index, &idx, expr.span);
                }
                if matches!(ty, Type::Dict | Type::Data) {
                    if ty == Type::Dict {
                        self.expect(&Type::String, &idx, index.span)?;
                    } else if !Type::String.accepts(&idx) && idx != Type::Numeric(Unit::Number) {
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
                    Type::List(t, _) | Type::Array(t) => t,
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
        // Controls are removed before parameter binding, so validate the original
        // call once, before native, method, function, or constructor dispatch.
        let mut names = BTreeSet::new();
        for (name, value) in args {
            if let Some(name) = name {
                if !names.insert(name.as_str()) {
                    return Err(self.error(
                        "S124",
                        format!("argument '{name}' supplied twice"),
                        value.span,
                    ));
                }
            } else if !names.is_empty() {
                return Err(self.error(
                    "S124",
                    "positional arguments must precede named arguments",
                    value.span,
                ));
            }
        }
        if let ExprKind::Member(base, name) = &callee.kind {
            let receiver = self.expression(base, env, None, variables)?;
            if let Some(builtin) = crate::prelude::methods_for(&receiver)
                .find(|builtin| builtin.name.rsplit('.').next() == Some(name.as_str()))
            {
                self.native_methods.insert(
                    (callee.span.start, callee.span.end),
                    NativeMethod {
                        name: builtin.name.into(),
                        materializes: crate::dataframe::definition(builtin.name).is_some_and(
                            |definition| {
                                definition
                                    .materialization
                                    .applies(matches!(receiver, Type::Series(_)))
                            },
                        ),
                    },
                );
                let mut all = vec![(None, *base.clone())];
                let method_args = if builtin.name == "DataFrame.assign" {
                    crate::dataframe::named_assignments(args.clone(), |columns| Expr {
                        cache_id: crate::ast::next_expr_identity(),
                        kind: ExprKind::Record(columns),
                        span: expr.span,
                    })
                } else {
                    args.clone()
                };
                all.extend(method_args);
                return self.native(expr, builtin.name, &all, env, expected, variables);
            }
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
        if !direct_callee(callee) {
            return Err(self.error("S111","indirect function calls are forbidden; call a declared function or imported module function directly so the compiler can verify the call graph",callee.span));
        }
        if let ExprKind::Name(name) = &callee.kind
            && prelude::builtin(name).is_some()
        {
            return self.native(expr, name, args, env, expected, variables);
        }
        let ty = self.expression(callee, env, None, variables)?;
        let Some(first_signature) = ty.signatures().first() else {
            return Err(self.error("S123", format!("cannot call {ty:?}"), callee.span));
        };
        let mut signature = first_signature.clone();
        let overloaded = ty.signatures().len() > 1;
        let enveloped = !signature
            .parameters
            .iter()
            .any(|parameter| parameter.name == "duration");
        let eased = !signature
            .parameters
            .iter()
            .any(|parameter| parameter.name == "ease");
        if enveloped && let Some(duration) = crate::timing::envelope(args) {
            let ty = self.expression(
                duration,
                env,
                Some(&Type::Numeric(Unit::Duration)),
                variables,
            )?;
            self.expect(&Type::Numeric(Unit::Duration), &ty, duration.span)?;
        }
        if eased && let Some(ease) = crate::timing::ease_argument(args) {
            let ty = self.expression(ease, env, Some(&Type::String), variables)?;
            self.expect(&Type::String, &ty, ease.span)?;
        }
        let normal = args
            .iter()
            .filter(|(name, _)| {
                !(enveloped && name.as_deref() == Some("duration")
                    || eased && name.as_deref() == Some("ease"))
            })
            .cloned()
            .collect::<Vec<_>>();
        let timed = enveloped
            && crate::timing::envelope(args).is_some_and(|expr| !crate::timing::known_zero(expr));
        let args = &normal;
        let pretyped = if overloaded {
            let (selected, actual) =
                self.clause_signature(ty.signatures(), args, env, variables, expr.span)?;
            signature = selected;
            Some(actual)
        } else {
            None
        };
        if let Some(function) = &self.function {
            if signature.schedules || timed {
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
                .map(|(i, parameter)| {
                    (
                        parameter.name.as_str(),
                        parameter.ty.clone(),
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
        for (index, (declaration, value)) in signature.parameters.iter().zip(ordered).enumerate() {
            let parameter = &declaration.ty;
            let Some(value) = value else {
                continue;
            };
            let wanted = self.substitute(parameter, &substitutions, expr.span)?;
            let actual =
                if let Some(actual) = pretyped.as_ref().and_then(|types| types[index].as_ref()) {
                    actual.clone()
                } else {
                    self.expression(value, env, Some(&wanted), variables)?
                };
            if actual.contains_function() {
                return Err(self.error("S111","function values cannot be passed as arguments or stored in containers; call the function directly",value.span));
            }
            infer(parameter, &actual, &mut substitutions, self.type_budget)
                .map_err(|e| self.error("S106", e, value.span))?;
            let expected = self.substitute(parameter, &substitutions, value.span)?;
            if !(overloaded && expected.overlaps(&actual)) {
                self.expect(&expected, &actual, value.span)?;
            }
        }
        self.complete_generics(&signature, &mut substitutions, expr.span)?;
        let stored = self.copy_environment(&substitutions, expr.span)?;
        self.call_types
            .insert((expr.span.start, expr.span.end), stored);
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
        for (name, value) in args {
            let index = if let Some(name) = name {
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
            .chain(self.result_types.values())
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

fn check_signal_capture(
    checker: &Checker<'_>,
    expression: &Expr,
    env: &Environment,
) -> Result<(), Diagnostic> {
    if let ExprKind::Call { callee, .. } = &expression.kind
        && crate::effects::lookup(callee, env).is_some_and(|ty| {
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
fn check_signal_purity(
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
            match &statement.kind {
                StatementKind::Bind {
                    kind: BindingKind::Sig,
                    value,
                    ..
                } => check_signal_capture(checker, value, env)?,
                StatementKind::Return(value) if live_return => {
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
                        env.get(name).is_some_and(|ty| {
                            ty.signatures().iter().any(|signature| {
                                matches!(signature.result.as_ref(), Type::Signal(_))
                            })
                        }),
                    )?;
                }
                StatementKind::Background { body }
                | StatementKind::For { body, .. }
                | StatementKind::Repeat { body, .. } => {
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
        let Some(signature) =
            crate::effects::lookup(callee, env).and_then(|ty| ty.signatures().first())
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
    if let StatementKind::Function { body, .. } = &statement.kind {
        for child in body {
            child.visit_expressions(&mut visit);
        }
    } else {
        statement.visit_expressions(&mut visit);
    }
    invalid.map_or(Ok(()), Err)
}
