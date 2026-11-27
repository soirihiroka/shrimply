mod capabilities;
mod clauses;
mod environment;
mod functions;
pub(crate) use environment::{Environment, TypeScope};
mod dataframe;
mod list_functions;
mod lists;
mod native;
mod templates;
use crate::{ast::*, prelude};
use clauses::{generic_variable, generic_variables, infer};
use functions::check_signal_purity;
use sanim_core::{Diagnostic, GenericParameter, Number, Signature, Span, Type, TypeBound, Unit};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Alias {
    pub parameters: Vec<GenericParameter<Type, TypeBound>>,
    pub body: TypeExpr,
}
/// Resolved once from the checked receiver, shared by runtime and editor tooling.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NativeMethod {
    pub name: String,
    pub materializes: bool,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Checked {
    /// Statement start -> index in the named export's canonical signatures.
    pub function_clauses: BTreeMap<usize, usize>,
    pub anonymous_functions: BTreeMap<(usize, usize), Signature>,
    pub exports: BTreeMap<String, Type>,
    pub dynamic_exports: BTreeSet<String>,
    pub schemas: BTreeMap<String, Type>,
    pub result_types: BTreeMap<(usize, usize), Type>,
    pub template_expressions: BTreeMap<(usize, usize), Type>,
    pub native_methods: BTreeMap<(usize, usize), NativeMethod>,
    pub call_types: BTreeMap<(usize, usize), BTreeMap<String, Type>>,
    pub reflected: BTreeMap<usize, std::sync::Arc<sanim_core::reflection::Declaration>>,
    pub binding_types: BTreeMap<usize, Type>,
    /// Present when entry execution can collect or return an Animation value.
    pub output_type: Option<Type>,
}

pub fn check(
    source: &str,
    module: &Module,
    imports: &BTreeMap<String, std::sync::Arc<Checked>>,
    root: bool,
    type_budget: &std::cell::Cell<usize>,
    limits: &crate::Limits,
) -> Result<Checked, Diagnostic> {
    check_module(source, module, imports, root, type_budget, limits, None)
}

pub(crate) fn check_expression(
    source: &str,
    module: &Module,
    type_budget: &std::cell::Cell<usize>,
    limits: &crate::Limits,
    interface: &crate::expression::Interface,
) -> Result<Checked, Diagnostic> {
    check_module(
        source,
        module,
        &BTreeMap::new(),
        true,
        type_budget,
        limits,
        Some(interface),
    )
}

fn check_module(
    source: &str,
    module: &Module,
    imports: &BTreeMap<String, std::sync::Arc<Checked>>,
    root: bool,
    type_budget: &std::cell::Cell<usize>,
    limits: &crate::Limits,
    interface: Option<&crate::expression::Interface>,
) -> Result<Checked, Diagnostic> {
    let mut checker = Checker {
        source,
        interface,
        type_budget,
        aliases: BTreeMap::new(),
        schemas: BTreeMap::new(),
        result_types: BTreeMap::new(),
        template_expressions: BTreeMap::new(),
        native_methods: BTreeMap::new(),
        call_types: BTreeMap::new(),
        reflected: BTreeMap::new(),
        binding_types: BTreeMap::new(),
        anonymous_functions: BTreeMap::new(),
        anonymous_names: BTreeMap::new(),
        callable_origins: BTreeMap::new(),
        callable_bindings: BTreeMap::new(),
        callable_substitutions: BTreeMap::new(),
        binding_scopes: vec![BTreeMap::new()],
        mutations: BTreeSet::new(),
        reflection_names: crate::reflection::identities(source, module)?,
        function: None,
        function_capabilities: None,
        external_reads: BTreeSet::new(),
        calls: BTreeMap::new(),
        root,
        collects_output: false,
    };
    let mut env = Environment::new();
    if let Some(interface) = interface {
        for (name, ty) in &interface.inputs {
            checker.check_expression_type(ty, Span::default())?;
            checker.bind_kind(name, BindingKind::Let);
            checker.insert_environment(&mut env, name.clone(), ty.clone(), Span::default())?;
        }
        checker.check_expression_type(&interface.result, Span::default())?;
    }
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
                            capabilities: Default::default(),
                            mutates: false,
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
                checker.insert_environment(
                    &mut env,
                    name.clone(),
                    Type::Record(namespace),
                    statement.span,
                )?;
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
                    .charge_import_interface(type_budget, alias)
                    .map_err(|e| checker.error("S105", e, statement.span))?;
                for (name, ty) in &imported.schemas {
                    checker
                        .schemas
                        .insert(format!("{alias}.{name}"), ty.clone());
                }
                for name in &imported.dynamic_exports {
                    let name = format!("{alias}.{name}");
                    checker.environment_bytes(
                        name.len() + std::mem::size_of::<String>(),
                        statement.span,
                    )?;
                    checker.external_reads.insert(name);
                }
                checker.insert_environment(
                    &mut env,
                    alias.clone(),
                    Type::Record(imported.exports.clone()),
                    statement.span,
                )?;
            }
            _ => {}
        }
    }
    for (name, alias) in &checker.aliases {
        if alias.parameters.is_empty() {
            let ty = checker.schemas[name].clone();
            if let Type::Record(fields) = &ty {
                checker.insert_environment(
                    &mut env,
                    name.clone(),
                    Type::Function(Signature {
                        capabilities: Default::default(),
                        mutates: false,
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
                    Span::default(),
                )?;
            }
        }
    }
    let function_clauses = checker.declare_clauses(module, &mut env, &BTreeMap::new())?;
    let mut entry_returns = false;
    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. } => {}
            StatementKind::Function { name, .. } => {
                let signatures = env[name.as_str()].signatures();
                checker.check_function(
                    statement,
                    &env,
                    &signatures[function_clauses[&statement.span.start]],
                    signatures.len() > 1,
                    &BTreeMap::new(),
                )?;
            }
            _ => {
                if interface.is_some() && entry_returns {
                    return Err(checker.error(
                        "S112",
                        "unreachable statement after return",
                        statement.span,
                    ));
                }
                entry_returns |= checker.block(
                    std::slice::from_ref(statement),
                    &mut env,
                    interface.map(|interface| &interface.result),
                    &BTreeMap::new(),
                )?;
            }
        }
    }
    if interface.is_some() && !entry_returns {
        return Err(checker.error(
            "S110",
            "expression must return a value on every path",
            module
                .statements
                .last()
                .map_or(Span::default(), |statement| statement.span),
        ));
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
        let before = checker.mutations.len();
        for (name, calls) in &checker.calls {
            if calls.iter().any(|v| checker.mutations.contains(v)) {
                checker.mutations.insert(name.clone());
            }
        }
        if before == checker.mutations.len() {
            break;
        }
    }
    for name in checker.mutations.clone() {
        if let Some(location) = checker.anonymous_names.get(&name) {
            checker
                .anonymous_functions
                .get_mut(location)
                .expect("checked anonymous function")
                .mutates = true;
        } else if env.contains_key(name.as_str()) {
            for signature in checker
                .mutate_environment(&mut env, &name, Span::default())?
                .signatures_mut()
            {
                signature.mutates = true;
            }
        }
    }
    checker.refresh_callable_effects(&mut env)?;
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
            return Err(checker.error("S113","a reflected default must not mutate captured cells, change scene configuration, or register visual assertions; compute its default with pure value-building functions",span));
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
            return Err(checker.error("S115","imported modules cannot mutate external state or register visual assertions during initialization; export a helper and call it from the entry script instead",span));
        }
    }
    for (location, signature) in &checker.anonymous_functions {
        checker
            .result_types
            .insert(*location, Type::Function(signature.clone()));
    }
    let dynamic_exports = checker
        .external_reads
        .iter()
        .cloned()
        .chain(
            checker.binding_scopes[0]
                .iter()
                .filter(|(name, binding)| {
                    binding.kind != BindingKind::Let
                        || env
                            .get(name.as_str())
                            .is_some_and(|ty| ty.contains_signal())
                })
                .map(|(name, _)| name.clone()),
        )
        .collect();
    Ok(Checked {
        output_type: interface
            .map(|interface| interface.result.clone())
            .or_else(|| (entry_returns || checker.collects_output).then_some(Type::Animation)),
        dynamic_exports,
        function_clauses,
        anonymous_functions: checker.anonymous_functions,
        exports: env
            .into_iter()
            .map(|(name, ty)| (name.to_string(), std::sync::Arc::unwrap_or_clone(ty)))
            .collect(),
        schemas: checker.schemas,
        result_types: checker.result_types,
        template_expressions: checker.template_expressions,
        native_methods: checker.native_methods,
        call_types: checker.call_types,
        reflected: checker.reflected,
        binding_types: checker.binding_types,
    })
}

#[derive(Clone)]
struct BindingInfo {
    kind: BindingKind,
    function: Option<String>,
    callable: BTreeSet<String>,
}
struct Checker<'a> {
    source: &'a str,
    interface: Option<&'a crate::expression::Interface>,
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
    anonymous_functions: BTreeMap<(usize, usize), Signature>,
    anonymous_names: BTreeMap<String, (usize, usize)>,
    callable_origins: BTreeMap<(usize, usize), BTreeSet<String>>,
    callable_bindings: BTreeMap<usize, BTreeSet<String>>,
    callable_substitutions: BTreeMap<(usize, usize), BTreeMap<String, BTreeSet<String>>>,
    binding_scopes: Vec<BTreeMap<String, BindingInfo>>,
    mutations: BTreeSet<String>,
    function: Option<String>,
    function_capabilities: Option<sanim_core::FnCapabilities>,
    external_reads: BTreeSet<String>,
    calls: BTreeMap<String, BTreeSet<String>>,
    root: bool,
    collects_output: bool,
}
impl Checker<'_> {
    fn error(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, message, self.source, span)
    }
    fn builtin(&self, name: &str) -> Option<&prelude::Builtin> {
        match self.interface {
            Some(interface) => interface.builtin(name),
            None => prelude::builtin(name),
        }
    }
    fn check_expression_type(&self, ty: &Type, span: Span) -> Result<(), Diagnostic> {
        if self.interface.is_some() && !crate::expression::allows_type(ty) {
            return Err(self.error("S141", format!("{ty} is unavailable in expressions"), span));
        }
        Ok(())
    }
    fn call_mutates(&self, expr: &Expr, env: &Environment) -> bool {
        let ExprKind::Call { callee, args, .. } = &expr.kind else {
            return false;
        };
        matches!(&callee.kind, ExprKind::Name(name) if self.mutations.contains(name))
            || crate::timing::builtin_call(callee, args, &self.native_methods).is_some_and(|call| {
                crate::list_functions::callback_type(&call, &(&*env, &self.result_types))
                    .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
            })
            || crate::analysis::lookup(callee, &(env, &self.result_types))
                .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
    }
    fn call_state_effect(&self, expr: &Expr, env: &Environment) -> bool {
        self.call_mutates(expr, env)
            || matches!(&expr.kind, ExprKind::Call { callee, args, .. } if crate::timing::builtin_call(callee,args,&self.native_methods).is_some_and(|call| call.capabilities(&(env,&self.result_types)).contains(sanim_core::FnCapability::Mut)))
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
                    callable: BTreeSet::new(),
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
            self.require_capability(
                sanim_core::FnCapability::Mut,
                span,
                &format!("assigning outer mutable binding '{name}' changes external state"),
            )?;
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
        if prelude::protected(name)
            || self
                .interface
                .is_some_and(|interface| interface.functions.contains_key(name))
        {
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
            TypeExpr::Function(_,parameters,result) => {
                for (_,_,ty) in parameters { self.validate_template(ty,variables,depth+1,span)?; }
                self.validate_template(result,variables,depth+1,span)?
            }
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
        self.check_expression_type(&ty, span)?;
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
            TypeExpr::Function(capabilities, parameters, result) => {
                self.resolve_callable_type(*capabilities, parameters, result, variables, depth, span)
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
            self.insert_environment(env, name.into(), narrowed, span)?;
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
                    if self.interface.is_some() && *kind == BindingKind::Sig {
                        return Err(self.error(
                            "S141",
                            "signals are unavailable in expressions",
                            statement.span,
                        ));
                    }
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
                        && crate::analysis::lookup(callee, &(&*env, &self.result_types))
                            .is_some_and(|ty| {
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
                        } else if *kind == BindingKind::Mut {
                            ty.editable_schema()
                        } else if *kind == BindingKind::Sig
                            && !matches!(value.kind, ExprKind::Call { .. })
                        {
                            // Checked call results already guarantee their tags and dimensions.
                            ty.widen_literals()
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
                    if ty.contains_function()
                        && (*kind != BindingKind::Let || ty.signatures().is_empty())
                    {
                        return Err(self.error("S111","function values require an immutable let binding; callable containers and mutable function cells are not supported",statement.span));
                    }
                    self.binding_types
                        .insert(statement.span.start, self.copy_type(&ty, statement.span)?);
                    self.bind_kind(name, *kind);
                    if let Some(origin) = self
                        .callable_origins
                        .get(&(value.span.start, value.span.end))
                        .cloned()
                    {
                        self.charge_callable_origins(&origin, statement.span)?;
                        self.callable_bindings
                            .insert(statement.span.start, origin.clone());
                        self.binding_scopes
                            .last_mut()
                            .expect("binding scope")
                            .get_mut(name)
                            .expect("declared binding")
                            .callable = origin;
                    }
                    self.insert_environment(env, name.clone(), ty, statement.span)?;
                }
                StatementKind::Assign { name, value } => {
                    let expected = self.writable(name, env, statement.span)?;
                    let actual = self.expression(value, env, Some(&expected), variables)?;
                    self.expect(&expected, &actual, value.span)?;
                    if !self.root && self.function.is_none() {
                        return Err(self.error("S115", "imported modules may initialize mutable cells but cannot reassign them while importing; perform the edit in a helper called from the root script", statement.span));
                    }
                }
                StatementKind::Expr(expr) => {
                    let ty = self.expression(expr, env, None, variables)?;
                    if ty == Type::Animation {
                        if self.root && self.function.is_none() {
                            self.collects_output = true;
                        }
                    } else if ty != Type::Unit {
                        return Err(self.error(
                            "S114",
                            format!(
                                "unused {ty} value; bind it with let or return it from a function"
                            ),
                            expr.span,
                        ));
                    }
                }
                StatementKind::Return(expr) => {
                    let entry_type = Type::Animation;
                    let result = result
                        .or_else(|| (self.root && self.function.is_none()).then_some(&entry_type));
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
                    if let Some(expr) = expr {
                        let ty = self.expression(expr, env, Some(expected), variables)?;
                        self.expect(expected, &ty, expr.span)?;
                    } else {
                        self.expect(expected, &Type::Unit, statement.span)?;
                    }
                    returned = true;
                }
                StatementKind::If { condition, yes, no } => {
                    let ty = self.expression(condition, env, Some(&Type::Bool), variables)?;
                    self.expect(&Type::Bool, &ty, condition.span)?;
                    let mut yes_scope = self.fork_environment(env, statement.span)?;
                    let mut no_scope = self.fork_environment(env, statement.span)?;
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
                    let mut scope = self.fork_environment(env, statement.span)?;
                    self.insert_environment(&mut scope, name.clone(), inner, statement.span)?;
                    self.scoped_block(body, &mut scope, result, variables)?;
                }
                StatementKind::Repeat { count, body } => {
                    let ty = self.expression(count, env, None, variables)?;
                    self.expect(&Type::Numeric(Unit::Number), &ty, count.span)?;
                    self.scoped_block(
                        body,
                        &mut self.fork_environment(env, statement.span)?,
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
                        let mut scope = self.fork_environment(env, statement.span)?;
                        for (name, ty) in case.bindings.iter().zip(fields) {
                            self.fresh(name, &scope, case.span)?;
                            self.insert_environment(
                                &mut scope,
                                name.clone(),
                                ty.clone(),
                                statement.span,
                            )?;
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
        self.check_expression_type(&actual, expr.span)?;
        self.check_call_capabilities(expr, env)?;
        if !actual.signatures().is_empty() || actual.contains_animation() {
            let ty = self.copy_type(&actual, expr.span)?;
            self.result_types
                .insert((expr.span.start, expr.span.end), ty);
        }
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
            ExprKind::Function(declaration) => {
                let StatementKind::Function { name, .. } = &declaration.kind else {
                    unreachable!("anonymous function AST")
                };
                let mut scope = self.fork_environment(env, expr.span)?;
                self.declare_clauses(
                    &Module {
                        statements: vec![*declaration.clone()],
                        comments: vec![],
                    },
                    &mut scope,
                    variables,
                )?;
                let mut signature = scope[name.as_str()].signatures()[0].clone();
                self.check_function(declaration, env, &signature, false, variables)?;
                let mutates = 'search: {
                    let mut pending = vec![name.as_str()];
                    let mut seen = BTreeSet::new();
                    while let Some(current) = pending.pop() {
                        if self.mutations.contains(current) {
                            break 'search true;
                        }
                        if seen.insert(current)
                            && let Some(calls) = self.calls.get(current)
                        {
                            pending.extend(calls.iter().map(String::as_str));
                        }
                    }
                    false
                };
                signature.mutates = mutates;
                let origin = BTreeSet::from([name.clone()]);
                self.charge_callable_origins(&origin, expr.span)?;
                self.callable_origins
                    .insert((expr.span.start, expr.span.end), origin);
                self.anonymous_names
                    .insert(name.clone(), (expr.span.start, expr.span.end));
                self.anonymous_functions
                    .insert((expr.span.start, expr.span.end), signature.clone());
                Ok(Type::Function(signature))
            }
            ExprKind::Reflect { value, options } => {
                if self.interface.is_some() {
                    return Err(self.error(
                        "S141",
                        "reflection is unavailable in expressions",
                        expr.span,
                    ));
                }
                let actual = self.expression(value, env, expected, variables)?;
                let ty = expected
                    .filter(|ty| {
                        crate::reflection::concrete_schema(ty)
                            && ty.reflectable()
                            && ty.accepts(&actual)
                    })
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
                if expected.is_some_and(|ty| !ty.string_literal_choices().is_empty()) {
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
                self.check_name_capture(name, env, expr.span)?;
                // References can escape through aliases, parameters and returned callables.
                // Include their lexical origin in the same bounded call graph.
                if env
                    .get(name.as_str())
                    .is_some_and(|ty| !ty.signatures().is_empty())
                {
                    let origin = self
                        .binding_scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name))
                        .map(|binding| binding.callable.clone())
                        .filter(|origin| !origin.is_empty())
                        .unwrap_or_else(|| BTreeSet::from([name.clone()]));
                    self.charge_callable_origins(&origin, expr.span)?;
                    self.callable_origins
                        .insert((expr.span.start, expr.span.end), origin.clone());
                    if let Some(function) = &self.function {
                        self.calls
                            .entry(function.clone())
                            .or_default()
                            .extend(origin);
                    }
                }
                if let Some(value) = prelude::values().get(name.as_str()) {
                    Ok(value.ty())
                } else {
                    env.get(name.as_str()).ok_or_else(|| {
                        let hint=expected.map(Type::string_literal_choices).filter(|values|values.contains(&name.as_str())).map(|_|format!(" Did you mean {name:?}? This argument expects a quoted string value ({expected}).",expected=expected.expect("literal context"))).unwrap_or_default();
                        self.error("S120",format!("unknown name '{name}'.{hint} Standard constructors such as Box and Color need no import; user names are case-sensitive."),expr.span)
                    }).and_then(|ty|self.copy_type(if let Type::Signal(inner)=ty.as_ref() {inner.as_ref()}else{ty.as_ref()},expr.span))
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
                let expected_right =
                    if op == "*" && sanim_core::math::linear::matrix::shape(&left).is_some() {
                        None
                    } else {
                        Some(context.as_ref().unwrap_or(&left))
                    };
                let right = self.expression(b, env, expected_right, variables)?;
                sanim_core::math::operations::binary_type(op, &left, &right)
                    .map_err(|e| self.error("S106", e, expr.span))
            }
            ExprKind::Member(base, name) => {
                if expr
                    .qualified_name()
                    .is_some_and(|name| self.external_reads.contains(&name))
                {
                    self.require_capability(sanim_core::FnCapability::Ref, expr.span, "reading a mutable or signal export from an imported module is an external read")?;
                }
                let ty = self.expression(base, env, None, variables)?;
                if ty == Type::Element {
                    self.require_capability(sanim_core::FnCapability::Ref, expr.span, "element getters sample the external animation timeline; pass an explicit snapshot to a pure function")?;
                }
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
                if let Some(index) = crate::list_functions::constant_count(Some(index), None)
                    && let Some(inner) = ty
                        .fixed_index(index)
                        .map_err(|message| self.error("S122", message, expr.span))?
                {
                    // Preserve the checked cardinality proof for runtime indexing.
                    let checked = self.copy_type(&ty, base.span)?;
                    self.result_types
                        .insert((base.span.start, base.span.end), checked);
                    return Ok(inner);
                }
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
            if crate::animations::definition(name).is_some()
                && !matches!(&receiver, Type::Record(fields) if fields.contains_key(name))
            {
                return Err(self.error(
                    "S123",
                    format!("{name} is an animation method for Element or List<Element>; this receiver has type {receiver}"),
                    base.span,
                ));
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
        if !direct_callee(callee)
            && !matches!(callee.kind, ExprKind::Function(_) | ExprKind::Call { .. })
        {
            return Err(self.error("S111","indirect function calls are forbidden; call a declared function or imported module function directly so the compiler can verify the call graph",callee.span));
        }
        if let ExprKind::Name(name) = &callee.kind
            && self.builtin(name).is_some()
        {
            return self.native(expr, name, args, env, expected, variables);
        }
        let ty = self.expression(callee, env, None, variables)?;
        let Some(first_signature) = ty.signatures().first() else {
            return Err(self.error("S123", format!("cannot call {ty:?}"), callee.span));
        };
        let mut signature = first_signature.clone();
        let overloaded = ty.signatures().len() > 1;
        let pretyped = if overloaded {
            let (selected, actual) =
                self.clause_signature(ty.signatures(), args, env, variables, expr.span)?;
            signature = selected;
            Some(actual)
        } else {
            None
        };
        if let Some(function) = &self.function
            && signature.mutates
        {
            self.mutations.insert(function.clone());
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
            if let Type::Variable(generic) | Type::BoundedVariable(generic, _) = parameter
                && let Some(origins) = self
                    .callable_origins
                    .get(&(value.span.start, value.span.end))
            {
                self.charge_callable_origins(origins, expr.span)?;
                self.callable_substitutions
                    .entry((expr.span.start, expr.span.end))
                    .or_default()
                    .entry(generic.clone())
                    .or_default()
                    .extend(origins.iter().cloned());
            }
            infer(parameter, &actual, &mut substitutions, self.type_budget)
                .map_err(|e| self.error("S106", e, value.span))?;
            let expected = self.substitute(parameter, &substitutions, value.span)?;
            if !(overloaded && expected.overlaps(&actual)) {
                self.expect(&expected, &actual, value.span)?;
            }
        }
        if let Type::Variable(generic) | Type::BoundedVariable(generic, _) =
            signature.result.as_ref()
            && let Some(origins) = self
                .callable_substitutions
                .get(&(expr.span.start, expr.span.end))
                .and_then(|values| values.get(generic))
                .cloned()
        {
            self.charge_callable_origins(&origins, expr.span)?;
            self.callable_origins
                .insert((expr.span.start, expr.span.end), origins);
        }
        self.complete_generics(&signature, &mut substitutions, expr.span)?;
        let stored = self.copy_substitutions(&substitutions, expr.span)?;
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

fn direct_callee(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(_) => true,
        ExprKind::Member(base, _) => direct_callee(base),
        _ => false,
    }
}
