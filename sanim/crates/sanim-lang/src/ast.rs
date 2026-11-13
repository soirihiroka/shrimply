use sanim_core::{GenericParameter, Span};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Module {
    pub statements: Vec<Statement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind {
    #[default]
    Let,
    Mut,
    Sig,
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
    Animate {
        name: String,
        value: Expr,
        duration: Expr,
        easing: Option<Expr>,
    },
    Background {
        body: Vec<Statement>,
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
        name: String,
        generics: Vec<GenericParameter<TypeExpr>>,
        parameters: Vec<(String, TypeExpr, Option<Expr>)>,
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
        period: Option<Expr>,
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
    Named(String, Vec<TypeExpr>),
    Record(Vec<(String, TypeExpr)>),
    Intersection(Vec<TypeExpr>),
    Union(Vec<TypeExpr>),
    Difference(Box<TypeExpr>, Box<TypeExpr>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExprKind {
    Number(String),
    String(String),
    Bool(bool),
    None,
    Color(String),
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

impl Expr {
    pub fn visit(&self, visitor: &mut impl FnMut(&Expr)) {
        self.visit_while(&mut |expr| {
            visitor(expr);
            true
        });
    }
    pub fn visit_while(&self, visitor: &mut impl FnMut(&Expr) -> bool) {
        if !visitor(self) {
            return;
        }
        match &self.kind {
            ExprKind::Unary(_, value)
            | ExprKind::Member(value, _)
            | ExprKind::Reflect { value, .. } => value.visit_while(visitor),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                a.visit_while(visitor);
                b.visit_while(visitor);
            }
            ExprKind::Call { callee, args, .. } => {
                callee.visit_while(visitor);
                for (_, value) in args {
                    value.visit_while(visitor);
                }
            }
            ExprKind::List(values) => {
                for value in values {
                    value.visit_while(visitor)
                }
            }
            ExprKind::Record(values) => {
                for (_, value) in values {
                    value.visit_while(visitor)
                }
            }
            ExprKind::Dict(values) => {
                for (key, value) in values {
                    key.visit_while(visitor);
                    value.visit_while(visitor);
                }
            }
            _ => {}
        }
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
        if let ExprKind::Name(name) = &node.kind {
            names.insert(name.clone());
        }
    });
}
impl Statement {
    /// Visit every declaration and executable statement, including function bodies.
    pub fn visit(&self, visitor: &mut impl FnMut(&Statement)) {
        visitor(self);
        match &self.kind {
            StatementKind::Background { body }
            | StatementKind::Function { body, .. }
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
    pub fn visit_expressions(&self, visitor: &mut impl FnMut(&Expr)) {
        match &self.kind {
            StatementKind::Bind { value, .. }
            | StatementKind::Assign { value, .. }
            | StatementKind::Expr(value)
            | StatementKind::Return(value) => value.visit(visitor),
            StatementKind::Animate {
                value,
                duration,
                easing,
                ..
            } => {
                value.visit(visitor);
                duration.visit(visitor);
                if let Some(easing) = easing {
                    easing.visit(visitor);
                }
            }
            StatementKind::Background { body } => {
                for statement in body {
                    statement.visit_expressions(visitor);
                }
            }
            StatementKind::If { condition, yes, no } => {
                condition.visit(visitor);
                for statement in yes.iter().chain(no) {
                    statement.visit_expressions(visitor);
                }
            }
            StatementKind::For { iterable, body, .. } => {
                iterable.visit(visitor);
                for statement in body {
                    statement.visit_expressions(visitor);
                }
            }
            StatementKind::Repeat {
                count,
                period,
                body,
            } => {
                count.visit(visitor);
                if let Some(period) = period {
                    period.visit(visitor);
                }
                for statement in body {
                    statement.visit_expressions(visitor);
                }
            }
            StatementKind::Match { value, cases } => {
                value.visit(visitor);
                for case in cases {
                    for statement in &case.body {
                        statement.visit_expressions(visitor);
                    }
                }
            }
            _ => {}
        }
    }
}
