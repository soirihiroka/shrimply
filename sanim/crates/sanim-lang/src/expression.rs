//! Typed, single-value evaluation for hosts with no scene or resource access.
use std::{cell::Cell, collections::BTreeMap, sync::Arc};

use sanim_core::{Diagnostic, Span, Type, Value};

use crate::{Limits, Loaded, ast::StatementKind, prelude};

pub type Bindings = imbl::OrdMap<String, Value>;
pub type HostCall<'a> = dyn Fn(&str, &Bindings) -> Result<Value, String> + 'a;

/// Static input and result types for a host's expression slot.
#[derive(Clone)]
pub struct Interface {
    pub(crate) inputs: BTreeMap<String, Type>,
    pub(crate) functions: BTreeMap<&'static str, prelude::Builtin>,
    pub(crate) result: Type,
}

impl Interface {
    pub fn new(result: Type) -> Self {
        Self {
            inputs: BTreeMap::new(),
            functions: BTreeMap::new(),
            result,
        }
    }

    pub fn input(mut self, name: &str, ty: Type) -> Self {
        self.inputs.insert(name.into(), ty);
        self
    }

    pub fn function(
        mut self,
        name: &'static str,
        result: Type,
        parameters: impl IntoIterator<Item = prelude::Parameter>,
    ) -> Self {
        self.functions.insert(
            name,
            prelude::Builtin {
                name,
                parameters: parameters.into_iter().collect(),
                result,
                state_effect: false,
                timing: Default::default(),
                content: Default::default(),
                execute: prelude::Execution::Host,
                check: crate::native_types::fixed,
            },
        );
        self
    }

    pub(crate) fn builtin(&self, name: &str) -> Option<&prelude::Builtin> {
        self.functions.get(name).or_else(|| prelude::builtin(name))
    }
}

pub(crate) struct Host<'a> {
    pub interface: &'a Interface,
    pub call: &'a HostCall<'a>,
}

pub struct CheckedExpression {
    pub(crate) module: Loaded,
    pub(crate) interface: Interface,
    pub(crate) limits: Limits,
    pub(crate) bindings: crate::eval::ModuleBindings,
    retained_bytes: usize,
}

impl CheckedExpression {
    pub fn check(
        name: &str,
        source: &str,
        mut interface: Interface,
        mut limits: Limits,
    ) -> Result<Self, Diagnostic> {
        let mut ast = parse(name, source, &limits)?;
        let Some(last) = ast.statements.last_mut() else {
            return Err(Diagnostic::new(
                "S110",
                "expression must return a value",
                name,
                Span::default(),
            ));
        };
        last.kind = match std::mem::replace(&mut last.kind, StatementKind::Return(None)) {
            StatementKind::Expr(value) => StatementKind::Return(Some(value)),
            statement => statement,
        };
        for name_in_scope in interface
            .inputs
            .keys()
            .map(String::as_str)
            .chain(interface.functions.keys().copied())
        {
            if prelude::protected(name_in_scope)
                || interface.inputs.contains_key(name_in_scope)
                    && interface.functions.contains_key(name_in_scope)
            {
                return Err(Diagnostic::new(
                    "S101",
                    format!("conflicting host name '{name_in_scope}'"),
                    name,
                    Span::default(),
                ));
            }
        }
        let type_budget = Cell::new(limits.max_type_bytes);
        let checked =
            crate::checker::check_expression(name, &ast, &type_budget, &limits, &interface)?;
        let mut referenced = std::collections::BTreeSet::new();
        for statement in &ast.statements {
            statement.visit_all_expressions(&mut |expr| {
                if let crate::ast::ExprKind::Name(name) = &expr.kind {
                    referenced.insert(name.as_str());
                }
            });
        }
        interface
            .inputs
            .retain(|name, _| referenced.contains(name.as_str()));
        let module = Loaded {
            id: name.into(),
            ast: Arc::new(ast),
            imports: BTreeMap::new(),
            checked: Arc::new(checked),
            interface_key: String::new(),
        };
        // Each evaluation observes a new host context and can call stateful host functions.
        limits.max_cache_bytes = 0;
        limits.max_cache_entries = 0;
        let bindings = crate::eval::ModuleBindings::new(&module, false);
        let retained_bytes = (limits.max_type_bytes - type_budget.get())
            .saturating_add(source.len().saturating_mul(2))
            .saturating_add(
                crate::cache::parsed::retained_bytes(&module.ast, source.len(), 0, usize::MAX)
                    .unwrap_or(usize::MAX),
            );
        Ok(Self {
            module,
            interface,
            limits,
            bindings,
            retained_bytes,
        })
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Only referenced host values need to be prepared at evaluation time.
    pub fn inputs(&self) -> impl Iterator<Item = &str> {
        self.interface.inputs.keys().map(String::as_str)
    }

    pub fn evaluate(&self, inputs: Bindings, call: &HostCall<'_>) -> Result<Value, Diagnostic> {
        crate::eval::evaluate_expression(self, inputs, call)
    }
}

pub fn parse(name: &str, source: &str, limits: &Limits) -> Result<crate::ast::Module, Diagnostic> {
    if source.len() > limits.max_source_bytes {
        return Err(Diagnostic::new(
            "S201",
            "source byte limit exceeded",
            name,
            Span::default(),
        ));
    }
    let module = crate::parser::parse(name, source, limits.max_parse_steps)?;
    if let Some(statement) = module
        .statements
        .iter()
        .find(|statement| matches!(statement.kind, StatementKind::Import { .. }))
    {
        return Err(Diagnostic::new(
            "S141",
            "imports are unavailable in expressions",
            name,
            statement.span,
        ));
    }
    Ok(module)
}

/// One registry policy for checking, evaluation, and editor suggestions.
pub fn allows_builtin(builtin: &prelude::Builtin) -> bool {
    use prelude::Execution;
    let name = builtin.name;
    matches!(builtin.execute, Execution::Host | Execution::List)
        || sanim_core::math::functions::function(name).is_some()
        || sanim_core::math::linear::LIST_FUNCTIONS
            .iter()
            .any(|(function, _)| *function == name)
        || sanim_core::math::color::Op::from_name(name).is_some()
        || matches!(
            name,
            "Color"
                | "Fraction"
                | "str"
                | "len"
                | "range"
                | "get"
                | "Some"
                | "unwrap"
                | "append"
                | "insert"
                | "pop"
                | "popleft"
                | "replace_at"
                | "move_item"
                | "keys"
                | "Data"
                | "assert"
                | "lerp"
                | "frame_index"
                | "timecode"
                | "timecode_parts"
        )
}

pub(crate) fn allows_type(ty: &Type) -> bool {
    !ty.contains(&|ty| {
        matches!(
            ty,
            Type::Animation
                | Type::Element
                | Type::Signal(_)
                | Type::Forever
                | Type::Font
                | Type::Texture
                | Type::Curve(_)
                | Type::Predicate
                | Type::Code(_)
                | Type::Sql
                | Type::SqlIdent
                | Type::VoxelGrid
                | Type::VoxelMaterial
                | Type::VoxelLayout
                | Type::Density
                | Type::Random
                | Type::DataFrame { .. }
                | Type::Series(_)
        )
    })
}
