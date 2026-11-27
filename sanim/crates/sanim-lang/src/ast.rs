use sanim_core::{FnCapabilities, GenericParameter, Span};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Module {
    pub statements: Vec<Statement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Span>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
    /// Exact identifier location for editor navigation and inferred-type hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_span: Option<Span>,
}

pub use sanim_core::BindingKind;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub kind: BindingKind,
    pub name: String,
    pub name_span: Span,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StatementKind {
    Bind {
        kind: BindingKind,
        name: String,
        annotation: Option<TypeExpr>,
        value: Expr,
    },
    Assign {
        name: String,
        value: Expr,
    },
    Alias {
        name: String,
        generics: Vec<GenericParameter<TypeExpr>>,
        ty: TypeExpr,
    },
    Enum {
        name: String,
        variants: Vec<(String, Vec<TypeExpr>)>,
    },
    Function {
        capabilities: FnCapabilities,
        name: String,
        generics: Vec<GenericParameter<TypeExpr>>,
        parameters: Vec<Parameter>,
        result: TypeExpr,
        body: Vec<Statement>,
    },
    Import {
        path: String,
        alias: String,
    },
    Expr(Expr),
    Return(Expr),
    If {
        condition: Expr,
        yes: Vec<Statement>,
        no: Vec<Statement>,
    },
    For {
        name: String,
        iterable: Expr,
        body: Vec<Statement>,
    },
    Repeat {
        count: Expr,
        body: Vec<Statement>,
    },
    Match {
        value: Expr,
        cases: Vec<Case>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub qualifier: Option<String>,
    pub variant: String,
    pub bindings: Vec<String>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeExpr {
    Function(
        FnCapabilities,
        Vec<(BindingKind, String, TypeExpr)>,
        Box<TypeExpr>,
    ),
    ConstLength(usize),
    Refined(Box<TypeExpr>, String, sanim_core::Number),
    StringLiteral(String),
    Named(String, Vec<TypeExpr>),
    Signal(Box<TypeExpr>),
    Record(Vec<(String, TypeExpr)>),
    Intersection(Vec<TypeExpr>),
    Union(Vec<TypeExpr>),
    Difference(Box<TypeExpr>, Box<TypeExpr>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Expr {
    #[serde(skip, default = "next_expr_identity")]
    pub(crate) cache_id: u64,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExprKind {
    /// Reuses the canonical function declaration; the generated name is not source-visible.
    Function(Box<Statement>),
    Number(String),
    String(String),
    Template(Vec<sanim_core::TemplatePart<Expr>>),
    Bool(bool),
    None,
    Name(String),
    Reflect {
        value: Box<Expr>,
        options: sanim_core::reflection::Options,
    },
    List(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Dict(Vec<(Expr, Expr)>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call {
        callee: Box<Expr>,
        types: Vec<TypeExpr>,
        args: Vec<(Option<String>, Expr)>,
    },
}

pub(crate) fn next_expr_identity() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("expression identity overflow")
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self {
            cache_id: next_expr_identity(),
            kind,
            span,
        }
    }
    pub fn visit<'a>(&'a self, visitor: &mut impl FnMut(&'a Expr)) {
        self.visit_while(&mut |expr| {
            visitor(expr);
            true
        });
    }
    pub fn visit_while<'a>(&'a self, visitor: &mut impl FnMut(&'a Expr) -> bool) {
        self.visit_while_mode(visitor, true);
    }
    fn visit_while_mode<'a>(&'a self, visitor: &mut impl FnMut(&'a Expr) -> bool, _jobs: bool) {
        if !visitor(self) {
            return;
        }
        match &self.kind {
            ExprKind::Unary(_, value)
            | ExprKind::Member(value, _)
            | ExprKind::Reflect { value, .. } => value.visit_while_mode(visitor, _jobs),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                a.visit_while_mode(visitor, _jobs);
                b.visit_while_mode(visitor, _jobs);
            }
            ExprKind::Call { callee, args, .. } => {
                callee.visit_while_mode(visitor, _jobs);
                for (_, value) in args {
                    value.visit_while_mode(visitor, _jobs);
                }
            }
            ExprKind::Template(parts) => {
                for part in parts {
                    if let sanim_core::TemplatePart::Hole { value, format, .. } = part {
                        value.visit_while_mode(visitor, _jobs);
                        if let Some(format) = format {
                            format.visit_while_mode(visitor, _jobs)
                        }
                    }
                }
            }
            ExprKind::List(values) => {
                for value in values {
                    value.visit_while_mode(visitor, _jobs)
                }
            }
            ExprKind::Record(values) => {
                for (_, value) in values {
                    value.visit_while_mode(visitor, _jobs)
                }
            }
            ExprKind::Dict(values) => {
                for (key, value) in values {
                    key.visit_while_mode(visitor, _jobs);
                    value.visit_while_mode(visitor, _jobs);
                }
            }
            _ => {}
        }
    }
    /// Visit syntax inside function values as well as executable expressions.
    pub fn visit_all<'a>(&'a self, visitor: &mut dyn FnMut(&'a Expr)) {
        self.visit_mode(visitor, true, true);
    }
    fn visit_mode<'a>(
        &'a self,
        visitor: &mut dyn FnMut(&'a Expr),
        definitions: bool,
        nested: bool,
    ) {
        self.visit_while_mode(
            &mut |expr| {
                visitor(expr);
                if definitions && let ExprKind::Function(declaration) = &expr.kind {
                    declaration.walk_expressions(visitor, true, true);
                }
                true
            },
            nested,
        );
    }
    pub fn qualified_name(&self) -> Option<String> {
        match &self.kind {
            ExprKind::Name(name) => Some(name.clone()),
            ExprKind::Member(base, name) => Some(format!("{}.{name}", base.qualified_name()?)),
            _ => None,
        }
    }
}
/// Value dependencies preserve member paths across imported module boundaries.
pub fn value_references(expr: &Expr, names: &mut std::collections::BTreeSet<String>) {
    expr.visit_while(&mut |node| {
        if let ExprKind::Function(declaration) = &node.kind {
            function_references(declaration, names);
            return false;
        }
        if matches!(node.kind, ExprKind::Name(_) | ExprKind::Member(..))
            && let Some(name) = node.qualified_name()
        {
            names.insert(name);
            return false;
        }
        true
    });
}
pub fn referenced_names(expr: &Expr, names: &mut std::collections::BTreeSet<String>) {
    expr.visit(&mut |node| {
        if let ExprKind::Function(declaration) = &node.kind {
            function_references(declaration, names);
        } else if let ExprKind::Name(name) = &node.kind {
            names.insert(name.clone());
        }
    });
}
impl Statement {
    /// Visit every declaration and executable statement, including function bodies.
    pub fn visit<'a>(&'a self, visitor: &mut impl FnMut(&'a Statement)) {
        visitor(self);
        match &self.kind {
            StatementKind::Function { body, .. }
            | StatementKind::For { body, .. }
            | StatementKind::Repeat { body, .. } => {
                for statement in body {
                    statement.visit(visitor);
                }
            }
            StatementKind::If { yes, no, .. } => {
                for statement in yes.iter().chain(no) {
                    statement.visit(visitor);
                }
            }
            StatementKind::Match { cases, .. } => {
                for statement in cases.iter().flat_map(|case| &case.body) {
                    statement.visit(visitor);
                }
            }
            _ => {}
        }
    }
    /// Visit executable expressions, excluding nested function declarations.
    pub fn visit_expressions<'a>(&'a self, visitor: &mut impl FnMut(&'a Expr)) {
        self.walk_expressions(visitor, false, true);
    }
    /// Visit all expression syntax, including named and anonymous function bodies.
    pub fn visit_all_expressions<'a>(&'a self, visitor: &mut dyn FnMut(&'a Expr)) {
        self.walk_expressions(visitor, true, true);
    }
    /// Visit expressions owned by this statement, excluding child statement bodies.
    pub fn visit_own_expressions<'a>(&'a self, visitor: &mut dyn FnMut(&'a Expr)) {
        self.walk_expressions(visitor, false, false);
    }
    fn walk_expressions<'a>(
        &'a self,
        visitor: &mut dyn FnMut(&'a Expr),
        definitions: bool,
        nested: bool,
    ) {
        match &self.kind {
            StatementKind::Bind { value, .. }
            | StatementKind::Assign { value, .. }
            | StatementKind::Expr(value)
            | StatementKind::Return(value) => value.visit_mode(visitor, definitions, nested),
            StatementKind::If { condition, yes, no } => {
                condition.visit_mode(visitor, definitions, nested);
                for statement in yes.iter().chain(no) {
                    if nested {
                        statement.walk_expressions(visitor, definitions, true);
                    }
                }
            }
            StatementKind::For { iterable, body, .. } => {
                iterable.visit_mode(visitor, definitions, nested);
                for statement in body {
                    if nested {
                        statement.walk_expressions(visitor, definitions, true);
                    }
                }
            }
            StatementKind::Repeat { count, body } => {
                count.visit_mode(visitor, definitions, nested);
                for statement in body {
                    if nested {
                        statement.walk_expressions(visitor, definitions, true);
                    }
                }
            }
            StatementKind::Match { value, cases } => {
                value.visit_mode(visitor, definitions, nested);
                for case in cases {
                    for statement in &case.body {
                        if nested {
                            statement.walk_expressions(visitor, definitions, true);
                        }
                    }
                }
            }
            StatementKind::Function {
                parameters, body, ..
            } if definitions => {
                for parameter in parameters {
                    if let Some(default) = &parameter.default {
                        default.visit_mode(visitor, true, nested);
                    }
                }
                for statement in body {
                    if nested {
                        statement.walk_expressions(visitor, true, true);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Lexical dependencies of a function value, including nested closure factories.
pub fn function_references(
    declaration: &Statement,
    names: &mut std::collections::BTreeSet<String>,
) {
    let StatementKind::Function {
        parameters, body, ..
    } = &declaration.kind
    else {
        return;
    };
    for statement in body {
        statement.visit(&mut |statement| {
            if let StatementKind::Assign { name, .. } = &statement.kind {
                names.insert(name.clone());
            }
        });
        statement.visit_expressions(&mut |expr| {
            if let ExprKind::Function(declaration) = &expr.kind {
                function_references(declaration, names);
            } else if let ExprKind::Name(name) = &expr.kind {
                names.insert(name.clone());
            }
        });
    }
    for parameter in parameters {
        names.remove(&parameter.name);
        if let Some(default) = &parameter.default {
            referenced_names(default, names);
        }
    }
}
