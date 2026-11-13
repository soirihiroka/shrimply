//! Source/type checking and inexpensive advice without evaluating data or scenes.
use crate::{
    Engine, Loaded, Sources, Statistics,
    ast::*,
    prelude, progress,
    source::{self, Snapshot},
};
use sanim_core::{Diagnostic, NodeDuration, Number, Span, Unit};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct LintDiagnostic {
    pub severity: LintSeverity,
    pub code: String,
    pub message: String,
    pub source: String,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct LintReport {
    pub source: String,
    pub diagnostics: Vec<LintDiagnostic>,
    pub dependencies: BTreeMap<String, String>,
    pub statistics: Statistics,
    pub durations: BTreeMap<String, Vec<NodeDuration>>,
    /// Shared compiler structures for editor tooling; no duplicated AST/type maps
    /// and no large internal structures in command-line JSON reports.
    #[serde(skip)]
    pub modules: BTreeMap<String, AnalyzedModule>,
}

#[derive(Clone, Debug)]
pub struct AnalyzedModule {
    pub ast: std::sync::Arc<Module>,
    pub checked: std::sync::Arc<crate::CheckedModule>,
}
impl LintReport {
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == LintSeverity::Warning)
            .count()
    }
}

impl Engine {
    /// Parse imports and check every type/effect without opening runtime resources.
    /// Use full preparation (`check` in the CLI) to also evaluate data and contracts.
    pub fn lint(&mut self, entry: &str, sources: &dyn Sources) -> Result<LintReport, Diagnostic> {
        self.with_progress(entry, |engine, reporter| {
            let entry = source::resolve("", entry)
                .map_err(|message| Diagnostic::new("S201", message, entry, Span::default()))?;
            let sources = progress::ObservedSources {
                provider: sources,
                reporter,
            };
            let mut snapshot = Snapshot {
                provider: &sources,
                reads: BTreeMap::new(),
                hashes: Default::default(),
            };
            let mut modules = Vec::new();
            let mut statistics = Statistics::default();
            engine.load(
                &entry,
                &mut snapshot,
                &mut modules,
                &mut Vec::new(),
                (
                    &mut statistics,
                    &std::cell::Cell::new(engine.limits.max_type_bytes),
                    reporter,
                ),
            )?;
            let mut diagnostics = Vec::new();
            for module in &modules {
                advice(
                    module,
                    module.id == entry,
                    engine.limits.warn_operations,
                    &mut diagnostics,
                );
            }
            diagnostics.sort_by(|a, b| {
                (&a.source, a.span.start, &a.code).cmp(&(&b.source, b.span.start, &b.code))
            });
            statistics.prepare_micros = reporter
                .started
                .elapsed()
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX);
            Ok(LintReport {
                source: entry,
                diagnostics,
                dependencies: snapshot.fingerprints(),
                statistics,
                modules: modules
                    .iter()
                    .map(|module| {
                        (
                            module.id.clone(),
                            AnalyzedModule {
                                ast: module.ast.clone(),
                                checked: module.checked.clone(),
                            },
                        )
                    })
                    .collect(),
                durations: modules
                    .into_iter()
                    .map(|module| (module.id, module.checked.durations.clone()))
                    .collect(),
            })
        })
    }
}

fn expensive(expression: &Expr, checked: &crate::CheckedModule) -> Option<&'static str> {
    let ExprKind::Call { callee, .. } = &expression.kind else {
        return None;
    };
    let builtin = match &callee.kind {
        ExprKind::Name(name) => prelude::builtin(name),
        ExprKind::Member(_, _) => checked
            .native_methods
            .get(&(callee.span.start, callee.span.end))
            .and_then(|method| prelude::builtin(&method.name)),
        _ => None,
    }?;
    if checked
        .native_methods
        .get(&(callee.span.start, callee.span.end))
        .is_some_and(|method| method.materializes)
    {
        return Some("SQLite DataFrame/Series materialization");
    }

    match builtin.execute {
        prelude::Execution::Read => Some("resource read/decoding"),
        prelude::Execution::Query => Some("SQLite query"),
        prelude::Execution::Decode => Some("schema decoding"),
        _ if builtin.uses_font_environment() && builtin.result != sanim_core::Type::Font => {
            Some("font shaping/layout")
        }
        _ => crate::charts::expensive(builtin.name),
    }
}

fn advice(module: &Loaded, root: bool, budget: u64, diagnostics: &mut Vec<LintDiagnostic>) {
    let mut warn = |code: &str, message: String, span| {
        diagnostics.push(LintDiagnostic {
            severity: LintSeverity::Warning,
            code: code.into(),
            message,
            source: module.id.clone(),
            span,
        })
    };
    let mut loops = Vec::new();
    for statement in &module.ast.statements {
        statement.visit(&mut |statement| {
            match &statement.kind {
                StatementKind::For { .. } | StatementKind::Repeat { .. } => loops.push(statement.span),
                _ => {},
            }
            if let StatementKind::Repeat { count: Expr { kind: ExprKind::Number(text), .. }, .. } = &statement.kind
                && let Ok(count) = Number::parse(text)
                && count.unit == Unit::Number && count.value >= sanim_core::Fraction::from(budget)
            {
                warn("P102", format!("this repeat has {text} iterations, at least the {budget}-operation warning budget before its body is counted. Profile preparation and prefer shared data or native chart/list operations where appropriate"), statement.span);
            }

        });
    }
    loops.sort_by_key(|span| span.start);
    let mut furthest = 0;
    let loop_ends: Vec<_> = loops
        .iter()
        .map(|span| {
            furthest = furthest.max(span.end);
            furthest
        })
        .collect();
    let mut referenced = BTreeSet::new();
    let mut inspect = |expression: &Expr| {
        if let ExprKind::Name(name) = &expression.kind {
            referenced.insert(name.clone());
        }
        if let Some(work) = expensive(expression, &module.checked) {
            let preceding = loops.partition_point(|span| span.start <= expression.span.start);
            if preceding > 0 && loop_ends[preceding - 1] >= expression.span.end {
                warn(
                    "P101",
                    format!(
                        "{work} appears inside a bounded loop. Its cost can repeat per iteration; hoist invariant inputs out of the loop and use profile to measure actual cache reuse"
                    ),
                    expression.span,
                );
            }
        }
    };
    for statement in &module.ast.statements {
        statement.visit_expressions(&mut inspect);
        // Function declarations are skipped by visit_expressions. Visit their
        // bodies separately, so every expression is inspected once, including
        // nested functions, without another AST or repeated subtree traversal.
        statement.visit(&mut |statement| {
            if let StatementKind::Function {
                body, parameters, ..
            } = &statement.kind
            {
                for parameter in parameters {
                    let default = &parameter.default;
                    if let Some(default) = default {
                        default.visit(&mut inspect);
                    }
                }
                for statement in body {
                    statement.visit_expressions(&mut inspect);
                }
            }
        });
    }

    if root {
        for statement in &module.ast.statements {
            if let StatementKind::Bind {
                kind, name, value, ..
            } = &statement.kind
                && !name.starts_with('_')
                && !referenced.contains(name.as_str())
            {
                let mut costly = false;
                let mut reflected = false;
                value.visit(&mut |expression| {
                    costly |= expensive(expression, &module.checked).is_some();
                    reflected |= matches!(expression.kind, ExprKind::Reflect { .. });
                });
                if !reflected && (costly || *kind == BindingKind::Sig) {
                    warn(
                        "L001",
                        format!(
                            "'{name}' is never referenced in this script. Demand preparation can leave eligible pure work unevaluated; full check/inspection still evaluates it. Remove unused work or prefix an intentional inspection binding with '_'"
                        ),
                        statement.span,
                    );
                }
            }
        }
    }
}
