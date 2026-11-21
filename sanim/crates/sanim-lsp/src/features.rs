//! Navigation and completion consume the compiler AST and checked type metadata.
use crate::{text::Document, workspace::Workspace};
use sanim_lang::{
    AnalyzedModule,
    ast::{BindingKind, Expr, ExprKind, Module, Statement, StatementKind},
    core::{Signature, Span, Type},
    prelude,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) fn formatting(
    workspace: &Workspace,
    uri: &str,
    options: &Value,
) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let defaults = sanim_lang::formatter::FormatOptions::default();
    let tab_size = options["tabSize"]
        .as_u64()
        .unwrap_or(defaults.indent_width as u64);
    let options = sanim_lang::formatter::FormatOptions {
        indent_width: usize::try_from(tab_size).map_err(|_| "tabSize exceeds platform limits")?,
        use_tabs: options["insertSpaces"].as_bool() == Some(false),
        ..defaults
    };
    let formatted = sanim_lang::formatter::format_source(
        uri,
        &document.text,
        options,
        sanim_lang::Limits::default().max_parse_steps,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    if formatted == document.text {
        return Ok(json!([]));
    }
    Ok(
        json!([{"range":document.range(Span {start:0,end:document.text.len(),..Default::default()}),"newText":formatted}]),
    )
}

struct Symbol {
    name: String,
    ty: Option<Type>,
    span: Span,
    selection: Span,
    scope: Span,
    kind: u32,
    forward: bool,
    root: bool,
}
fn declaration_signature<'a>(
    module: &'a AnalyzedModule,
    name: &str,
    span: Span,
) -> Option<&'a Signature> {
    if let Some(signature) = module
        .checked
        .anonymous_functions
        .get(&(span.start, span.end))
    {
        return Some(signature);
    }
    module
        .checked
        .exports
        .get(name)?
        .signatures()
        .get(*module.checked.function_clauses.get(&span.start)?)
}
fn symbols(module: &AnalyzedModule, length: usize) -> Vec<Symbol> {
    fn block(
        body: &[Statement],
        scope: Span,
        module: &AnalyzedModule,
        root: bool,
        out: &mut Vec<Symbol>,
    ) {
        for statement in body {
            statement.visit_own_expressions(&mut |expr| {
                if let ExprKind::Function(declaration) = &expr.kind
                    && let StatementKind::Function {
                        name,
                        parameters,
                        body,
                        ..
                    } = &declaration.kind
                {
                    function(declaration, name, parameters, body, module, out);
                }
            });
            let (name, kind, forward) = match &statement.kind {
                StatementKind::Bind { name, kind, .. } => (
                    Some(name),
                    if *kind == BindingKind::Mut { 13 } else { 14 },
                    false,
                ),
                StatementKind::Function { name, .. } => (Some(name), 12, true),
                StatementKind::Alias { name, .. } => (Some(name), 26, true),
                StatementKind::Enum { name, .. } => (Some(name), 10, true),
                StatementKind::Import { alias, .. } => (Some(alias), 2, true),
                _ => (None, 0, false),
            };
            if let Some(name) = name {
                let ty = module
                    .checked
                    .binding_types
                    .get(&statement.span.start)
                    .or_else(|| {
                        (!matches!(statement.kind, StatementKind::Bind { .. }))
                            .then(|| module.checked.exports.get(name))
                            .flatten()
                    })
                    .or_else(|| module.checked.schemas.get(name))
                    .cloned();
                out.push(Symbol {
                    name: name.clone(),
                    ty,
                    span: statement.span,
                    selection: statement.name_span.unwrap_or(statement.span),
                    scope,
                    kind,
                    forward,
                    root,
                });
            }
            match &statement.kind {
                StatementKind::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    function(statement, name, parameters, body, module, out);
                }
                StatementKind::If { yes, no, .. } => {
                    branch(yes, statement.span, module, out);
                    branch(no, statement.span, module, out);
                }
                StatementKind::For { name, body, .. } => {
                    let scope = body_span(body, statement.span);
                    out.push(Symbol {
                        name: name.clone(),
                        ty: None,
                        span: statement.span,
                        selection: statement.name_span.unwrap_or(statement.span),
                        scope,
                        kind: 13,
                        forward: true,
                        root: false,
                    });
                    block(body, scope, module, false, out);
                }
                StatementKind::Repeat { body, .. } => branch(body, statement.span, module, out),
                StatementKind::Match { cases, .. } => {
                    for case in cases {
                        let scope = body_span(&case.body, statement.span);
                        for name in &case.bindings {
                            out.push(Symbol {
                                name: name.clone(),
                                ty: None,
                                span: statement.span,
                                selection: Span::default(),
                                scope,
                                kind: 13,
                                forward: true,
                                root: false,
                            });
                        }
                        block(&case.body, scope, module, false, out);
                    }
                }
                _ => {}
            }
        }
    }
    fn function(
        statement: &Statement,
        name: &str,
        parameters: &[sanim_lang::ast::Parameter],
        body: &[Statement],
        module: &AnalyzedModule,
        out: &mut Vec<Symbol>,
    ) {
        let signature = declaration_signature(module, name, statement.span);
        for (index, parameter) in parameters.iter().enumerate() {
            let name = &parameter.name;
            out.push(Symbol {
                name: name.clone(),
                ty: signature
                    .and_then(|signature| signature.parameters.get(index))
                    .map(|parameter| parameter.ty.clone()),
                span: statement.span,
                selection: parameter.name_span,
                scope: statement.span,
                kind: 13,
                forward: true,
                root: false,
            });
        }
        block(body, statement.span, module, false, out);
    }
    fn body_span(body: &[Statement], fallback: Span) -> Span {
        Span {
            start: body
                .first()
                .map_or(fallback.start, |statement| statement.span.start),
            end: body
                .last()
                .map_or(fallback.end, |statement| statement.span.end),
            ..fallback
        }
    }
    fn branch(body: &[Statement], fallback: Span, module: &AnalyzedModule, out: &mut Vec<Symbol>) {
        block(body, body_span(body, fallback), module, false, out);
    }
    let mut out = Vec::new();
    block(
        &module.ast.statements,
        Span {
            end: length,
            ..Default::default()
        },
        module,
        true,
        &mut out,
    );
    out
}
fn visible(symbol: &Symbol, offset: usize) -> bool {
    symbol.scope.start <= offset
        && offset <= symbol.scope.end
        && (symbol.forward || symbol.span.start <= offset)
}
fn find_symbol(
    module: &AnalyzedModule,
    length: usize,
    name: &str,
    offset: usize,
) -> Option<Symbol> {
    select_symbol(symbols(module, length), name, offset)
}
fn select_symbol(
    symbols: impl IntoIterator<Item = Symbol>,
    name: &str,
    offset: usize,
) -> Option<Symbol> {
    symbols
        .into_iter()
        .filter(|symbol| symbol.name == name && visible(symbol, offset))
        .min_by_key(|symbol| {
            (
                symbol.scope.end - symbol.scope.start,
                usize::MAX - symbol.span.start,
            )
        })
}
fn expressions(module: &Module, visitor: &mut impl FnMut(&Expr)) {
    for statement in &module.statements {
        statement.visit_all_expressions(visitor);
    }
}

fn expression_at(module: &Module, offset: usize) -> Option<Expr> {
    let mut best: Option<Expr> = None;
    expressions(module, &mut |expression| {
        if expression.span.end > expression.span.start
            && expression.span.start <= offset
            && offset <= expression.span.end
            && matches!(expression.kind, ExprKind::Name(_) | ExprKind::Member(_, _))
            && !matches!(&expression.kind, ExprKind::Member(base, _) if offset < base.span.end)
            && best.as_ref().is_none_or(|best| {
                expression.span.end - expression.span.start < best.span.end - best.span.start
            })
        {
            best = Some(expression.clone());
        }
    });
    best
}
pub(crate) fn native_signature(
    builtin: &prelude::Builtin,
    receiver: bool,
) -> (String, Vec<(String, String)>) {
    let parameters = builtin
        .parameters
        .iter()
        .skip(usize::from(receiver))
        .map(|parameter| {
            let annotation = if parameter.ty == Type::Unknown {
                String::new()
            } else {
                format!(": {}", parameter.ty)
            };
            (
                parameter.name.to_owned(),
                format!(
                    "{}{}{annotation}",
                    parameter.name,
                    if parameter.required { "" } else { "?" },
                ),
            )
        })
        .collect::<Vec<_>>();
    let name = if receiver {
        builtin.name.rsplit('.').next().unwrap_or(builtin.name)
    } else {
        builtin.name
    };
    let result = if builtin.result == Type::Unknown {
        String::new()
    } else {
        format!(": {}", builtin.result)
    };
    let capabilities = builtin.capabilities();
    let prefix = if capabilities.is_pure() {
        "fn".to_owned()
    } else {
        format!("fn<{capabilities}>")
    };
    let label = format!(
        "{prefix} {name}({}){result}",
        parameters
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );
    (label, parameters)
}
fn builtin_signature(name: &str) -> Option<(String, Vec<(String, String)>)> {
    prelude::builtin(name).map(|builtin| native_signature(builtin, false))
}
fn native_method(
    module: &AnalyzedModule,
    length: usize,
    callee: &Expr,
    offset: usize,
) -> Option<&'static prelude::Builtin> {
    if let Some(canonical) = module
        .checked
        .native_methods
        .get(&(callee.span.start, callee.span.end))
    {
        return prelude::builtin(&canonical.name);
    }
    // Incomplete calls have not been checked yet; their receiver is still known
    // from the completed declarations retained by the canonical parser.
    if let ExprKind::Member(base, name) = &callee.kind
        && let ExprKind::Name(base) = &base.kind
        && let Some(receiver) = ty(module, length, base, offset)
    {
        return sanim_lang::prelude::methods_for(&receiver)
            .find(|builtin| builtin.name.rsplit('.').next() == Some(name.as_str()));
    }
    None
}

fn ty(module: &AnalyzedModule, length: usize, name: &str, offset: usize) -> Option<Type> {
    find_symbol(module, length, name, offset)
        .and_then(|symbol| symbol.ty)
        .or_else(|| prelude::values().get(name).map(|value| value.ty()))
}

fn callable_type(
    module: &AnalyzedModule,
    length: usize,
    callee: &Expr,
    offset: usize,
) -> Option<(String, Type)> {
    match &callee.kind {
        ExprKind::Name(name) => Some((name.clone(), ty(module, length, name, offset)?)),
        ExprKind::Member(base, name) => {
            let ExprKind::Name(base) = &base.kind else {
                return None;
            };
            let Type::Record(mut fields) = ty(module, length, base, offset)? else {
                return None;
            };
            Some((format!("{base}.{name}"), fields.remove(name)?))
        }
        _ => None,
    }
}
fn argument_value(expr: &Expr) -> Option<sanim_lang::core::Value> {
    use sanim_lang::core::{Number, TemplatePart, Value};
    match &expr.kind {
        ExprKind::String(value) => Some(Value::String(value.clone().into())),
        ExprKind::Number(value) => Some(Value::Number(Number::parse(value).ok()?)),
        ExprKind::Bool(value) => Some(Value::Bool(*value)),
        ExprKind::Template(parts) => {
            let mut text = String::new();
            for part in parts {
                if let TemplatePart::Text(value) = part {
                    text.push_str(value);
                } else {
                    return None;
                }
            }
            Some(Value::String(text.into()))
        }
        _ => None,
    }
}
fn argument_type(
    module: &AnalyzedModule,
    length: usize,
    expr: &Expr,
    offset: usize,
) -> Option<Type> {
    module
        .checked
        .result_types
        .get(&(expr.span.start, expr.span.end))
        .cloned()
        .or_else(|| callable_type(module, length, expr, offset).map(|(_, ty)| ty))
        .or_else(|| argument_value(expr).map(|value| value.ty()))
}
fn applicable_signature(
    module: &AnalyzedModule,
    length: usize,
    call: &CallPosition,
    signature: &Signature,
    offset: usize,
    skip_active: bool,
) -> bool {
    let mut positional = 0;
    call.arguments
        .iter()
        .enumerate()
        .all(|(index, (label, value))| {
            let parameter = if let Some(label) = label {
                signature
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *label)
            } else {
                let p = signature.parameters.get(positional);
                positional += 1;
                p
            };
            if skip_active && index == call.index {
                return true;
            }
            let Some(parameter) = parameter else {
                return false;
            };
            if parameter.ty.unresolved() {
                return true;
            }
            if let Some(value) = argument_value(value) {
                sanim_lang::core::validate_value(&parameter.ty, &value).is_ok()
            } else {
                argument_type(module, length, value, offset)
                    .is_none_or(|actual| parameter.ty.overlaps(&actual))
            }
        })
}

mod markup;
use markup::attribute_position as markup_position;

fn literal_completion(
    workspace: &Workspace,
    uri: &str,
    offset: usize,
    syntax: &crate::workspace::Syntax,
) -> Option<Value> {
    if syntax
        .highlights
        .iter()
        .any(|span| span.kind.as_ref() == "comment" && span.start <= offset && offset < span.end)
    {
        return None;
    }
    let document = &workspace.documents.get(uri)?.document;
    let expected = syntax
        .markup
        .iter()
        .rev()
        .find_map(|opening| {
            let attribute = opening.attributes.iter().find(|attribute| {
                attribute
                    .value
                    .is_some_and(|span| span.start <= offset && offset <= span.end)
            })?;
            let parameter = prelude::builtin(&opening.name)?
                .parameters
                .iter()
                .find(|parameter| parameter.name == attribute.name)?;
            Some((parameter.ty.clone(), attribute.value?))
        })
        .or_else(|| {
            let (analysis, module) = workspace.module(uri)?;
            let call = call_position(analysis, module, offset)?;
            let native = match &call.callee.kind {
                ExprKind::Name(name) => prelude::builtin(name).map(|native| (native, false)),
                _ => native_method(module, document.text.len(), &call.callee, offset)
                    .map(|native| (native, true)),
            };
            let expected = if let Some((native, receiver)) = native {
                let mut parameters = native.parameters.iter().skip(usize::from(receiver));
                match call.named {
                    Some(name) => parameters.find(|parameter| parameter.name == name),
                    None => parameters.nth(call.index),
                }?
                .ty
                .clone()
            } else {
                let (_, callable) =
                    callable_type(module, document.text.len(), &call.callee, offset)?;
                Type::union(
                    callable
                        .signatures()
                        .iter()
                        .filter(|signature| {
                            applicable_signature(
                                module,
                                document.text.len(),
                                &call,
                                signature,
                                offset,
                                true,
                            )
                        })
                        .filter_map(|signature| {
                            match &call.named {
                                Some(name) => signature
                                    .parameters
                                    .iter()
                                    .find(|parameter| parameter.name == *name),
                                None => signature.parameters.get(call.index),
                            }
                            .map(|parameter| parameter.ty.clone())
                        }),
                )
                .ok()?
            };
            Some((
                expected,
                call.value.unwrap_or(Span {
                    start: offset,
                    end: offset,
                    ..Default::default()
                }),
            ))
        })?;
    let (expected, span) = expected;
    if offset < span.start || offset > span.end {
        return None;
    }
    let values = expected.string_literal_choices();
    if values.is_empty() {
        return None;
    }
    Some(
        json!({"isIncomplete":false,"items":values.into_iter().map(|value| {
        let quoted=sanim_lang::core::template::quote_string(value);
        json!({"label":quoted,"kind":20,"detail":expected.to_string(),"textEdit":{"range":document.range(span),"newText":quoted}})
    }).collect::<Vec<_>>()}),
    )
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
    trigger: Option<&str>,
) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let offset = document.offset(position)?;
    let syntax = workspace.documents[uri].syntax();
    if let Some(items) = markup::completion(document, &syntax, offset, workspace.snippet_support) {
        return Ok(items);
    }
    if let Some(items) = literal_completion(workspace, uri, offset, &syntax) {
        return Ok(items);
    }
    if markup::non_code(&syntax, offset) {
        return Ok(json!({"isIncomplete":false,"items":[]}));
    }
    if trigger.is_some_and(|character| character != ".") {
        return Ok(json!({"isIncomplete":false,"items":[]}));
    }
    let mut items = BTreeMap::new();
    if let Some((_, module)) = workspace.module(uri) {
        for symbol in symbols(module, document.text.len())
            .into_iter()
            .filter(|symbol| visible(symbol, offset))
        {
            items.insert(symbol.name.clone(),json!({"label":symbol.name,"kind":if symbol.kind==12 {3} else {6},"detail":symbol.ty.map(|ty|ty.to_string()).unwrap_or_else(||"declared in this scope".into())}));
        }
        // Identifier-prefix inspection chooses a namespace; all fields and types
        // still come from compiler metadata rather than a second source parser.
        let prefix = &document.text[..offset];
        let start = prefix
            .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .map_or(0, |index| index + 1);
        if prefix[..start].ends_with('.') {
            let end = start - 1;
            let begin = prefix[..end]
                .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .map_or(0, |index| index + 1);
            if let Some(receiver) = ty(module, document.text.len(), &prefix[begin..end], offset) {
                let mut members = prelude::methods_for(&receiver)
                    .map(|builtin| json!({"label":builtin.name.rsplit('.').next().unwrap_or(builtin.name),"kind":2,"detail":native_signature(builtin,true).0}))
                    .collect::<Vec<_>>();
                if let Type::Record(fields) = &receiver {
                    members.extend(
                        fields.iter().map(
                            |(name, ty)| json!({"label":name,"kind":5,"detail":ty.to_string()}),
                        ),
                    );
                }
                if matches!(receiver, Type::DataFrame { .. }) {
                    members.push(json!({"label":"columns","kind":5,"detail":"List<String>"}));
                }
                return Ok(json!({"isIncomplete":false,"items":members}));
            }
        }
    }
    for builtin in prelude::builtins() {
        items.entry(builtin.name.into()).or_insert_with(||json!({"label":builtin.name,"kind":3,"detail":builtin_signature(builtin.name).map(|(label,_)|label),"documentation":"Native Sanim operation."}));
    }
    for (name, definition) in prelude::types() {
        items.entry((*name).into()).or_insert_with(||json!({"label":name,"kind":7,"detail":definition.instantiate(vec![]).map(|ty|ty.to_string()).unwrap_or_else(|_|format!("type {name}"))}));
    }
    for (name, value) in prelude::values() {
        items
            .entry((*name).into())
            .or_insert_with(|| json!({"label":name,"kind":21,"detail":value.ty().to_string()}));
    }
    Ok(json!({"isIncomplete":false,"items":items.into_values().collect::<Vec<_>>()}))
}

pub(crate) fn reflected_at(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
) -> Option<std::sync::Arc<sanim_lang::core::reflection::Declaration>> {
    let document = &workspace.documents.get(uri)?.document;
    let offset = document.offset(position).ok()?;
    let (_, module) = workspace.module(uri)?;
    let symbol = symbols(module, document.text.len())
        .into_iter()
        .find(|symbol| symbol.selection.start <= offset && offset < symbol.selection.end)
        .or_else(|| {
            let expression = expression_at(&module.ast, offset)?;
            let ExprKind::Name(name) = expression.kind else {
                return None;
            };
            find_symbol(module, document.text.len(), &name, offset)
        });
    module
        .checked
        .reflected
        .values()
        .find(|input| {
            input.span.start <= offset && offset < input.span.end
                || symbol.as_ref().is_some_and(|symbol| {
                    symbol.name == input.name
                        && symbol.span.start <= input.span.start
                        && input.span.end <= symbol.span.end
                })
        })
        .cloned()
}

#[derive(Clone)]
pub(crate) struct AnimationLocation {
    pub span: Span,
    pub binding: Option<String>,
}
pub(crate) fn animation_at(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
) -> Option<AnimationLocation> {
    let document = &workspace.documents.get(uri)?.document;
    let offset = document.offset(position).ok()?;
    let (_, module) = workspace.module(uri)?;
    if let Some(symbol) = symbols(module, document.text.len())
        .into_iter()
        .find(|symbol| {
            symbol.selection.start <= offset
                && offset < symbol.selection.end
                && symbol.ty == Some(Type::Animation)
        })
    {
        return Some(AnimationLocation {
            span: symbol.span,
            binding: Some(symbol.name),
        });
    }
    let mut result: Option<AnimationLocation> = None;
    expressions(&module.ast, &mut |expression| {
        let selected = match &expression.kind {
            ExprKind::Call { callee, .. } => {
                callee.span.start <= offset && offset < callee.span.end
            }
            ExprKind::Name(_) => expression.span.start <= offset && offset < expression.span.end,
            _ => false,
        };
        if selected
            && module
                .checked
                .result_types
                .get(&(expression.span.start, expression.span.end))
                == Some(&Type::Animation)
            && result.as_ref().is_none_or(|found| {
                expression.span.end - expression.span.start < found.span.end - found.span.start
            })
        {
            if let ExprKind::Name(name) = &expression.kind
                && let Some(symbol) = find_symbol(module, document.text.len(), name, offset)
            {
                result = Some(AnimationLocation {
                    span: symbol.span,
                    binding: Some(name.clone()),
                });
            } else {
                result = Some(AnimationLocation {
                    span: expression.span,
                    binding: None,
                });
            }
        }
    });
    result
}

impl AnimationLocation {
    pub(crate) fn matches(&self, origin: &sanim_lang::core::plan::Origin) -> bool {
        origin.location.span == self.span
            || self.binding.as_ref().is_some_and(|name| {
                origin.binding.as_deref() == Some(name.as_str())
                    && self.span.start <= origin.location.span.start
                    && origin.location.span.end <= self.span.end
            })
    }
}
pub(crate) fn animation_sizes<'a>(
    receipts: impl Iterator<Item = &'a sanim_lang::core::plan::Receipt>,
) -> Vec<String> {
    const MAX_CONTEXTS: usize = 16;
    let mut sizes = BTreeMap::<String, usize>::new();
    for receipt in receipts {
        let mut summary = format!(
            "Duration: **{}**; sizing: `{}`",
            receipt.resolved, receipt.requested
        );
        if let Some(parent) = receipt.parent {
            summary.push_str(&format!(
                "; parent: **{}**",
                sanim_lang::core::plan::Extent::Finite(parent)
            ));
        }
        *sizes.entry(summary).or_default() += 1;
    }
    let count = sizes.len();
    let mut descriptions = sizes
        .into_iter()
        .take(MAX_CONTEXTS)
        .map(|(summary, placements)| {
            if placements > 1 {
                format!("{summary} ({placements} placements)")
            } else {
                summary
            }
        })
        .collect::<Vec<_>>();
    if count > MAX_CONTEXTS {
        descriptions.push(format!(
            "{} additional sizing contexts",
            count - MAX_CONTEXTS
        ));
    }
    descriptions
}
pub(crate) fn animation_hover(
    workspace: &mut Workspace,
    uri: &str,
    position: &Value,
    hover: &mut Value,
) {
    let Some(location) = animation_at(workspace, uri, position) else {
        return;
    };
    let source = workspace
        .module(uri)
        .map(|(analysis, _)| analysis.entry.clone());
    let Some(layout) = workspace.animation_layout(uri) else {
        return;
    };
    let descriptions = animation_sizes(layout.receipts.iter().filter(|receipt| {
        receipt.origin.as_ref().is_some_and(|origin| {
            source.as_deref() == Some(origin.location.source.as_ref()) && location.matches(origin)
        })
    }));
    let detail = if descriptions.is_empty() {
        layout.constraint.unwrap_or_default().to_owned()
    } else {
        descriptions.join("\n\n")
    };
    if detail.is_empty() {
        return;
    }
    let document = &workspace.documents[uri].document;
    if hover.is_null() {
        *hover = json!({"contents":{"kind":"markdown","value":detail},"range":document.range(location.span)});
    } else if let Some(value) = hover["contents"]["value"].as_str() {
        hover["contents"]["value"] = format!("{value}\n\n{detail}").into();
    }
}

pub(crate) fn hover(workspace: &Workspace, uri: &str, position: &Value) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let offset = document.offset(position)?;
    let syntax = workspace.documents[uri].syntax();
    if let Some((opening, span)) = markup_position(&syntax, offset)
        && let Some(attribute) = opening
            .attributes
            .iter()
            .find(|attribute| attribute.utility && attribute.span == span)
    {
        let value = sanim_lang::utilities::describe(&opening.name, &attribute.name)
            .map(|utility| utility.markdown())
            .unwrap_or_else(std::convert::identity);
        return Ok(
            json!({"contents":{"kind":"markdown","value":value},"range":document.range(span)}),
        );
    }
    let Some((_, module)) = workspace.module(uri) else {
        return Ok(Value::Null);
    };
    let declaration = symbols(module, document.text.len())
        .into_iter()
        .find(|symbol| {
            symbol.selection.end > symbol.selection.start
                && symbol.selection.start <= offset
                && offset < symbol.selection.end
                && document
                    .text
                    .get(symbol.selection.start..symbol.selection.end)
                    == Some(symbol.name.as_str())
        });
    let (label, range) = if let Some(symbol) = declaration {
        let clause = (symbol.kind == 12)
            .then(|| declaration_signature(module, &symbol.name, symbol.span))
            .flatten();
        let display_type = clause
            .map(|signature| Type::Function(signature.clone()))
            .or(symbol.ty);
        (
            Some(
                display_type
                    .map(|ty| format!("{}: {ty}", symbol.name))
                    .unwrap_or(symbol.name),
            ),
            symbol.selection,
        )
    } else if let Some(expression) = expression_at(&module.ast, offset) {
        let label = native_method(module, document.text.len(), &expression, offset)
            .map(|builtin| native_signature(builtin, true).0)
            .or_else(|| match &expression.kind {
                ExprKind::Name(name) => {
                    builtin_signature(name).map(|(label, _)| label).or_else(|| {
                        ty(module, document.text.len(), name, offset)
                            .map(|ty| format!("{name}: {ty}"))
                    })
                }
                ExprKind::Member(base, name) => {
                    if let ExprKind::Name(base) = &base.kind {
                        match ty(module, document.text.len(), base, offset) {
                            Some(Type::Record(fields)) => {
                                fields.get(name).map(|ty| format!("{base}.{name}: {ty}"))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            });
        (label, expression.span)
    } else {
        return Ok(Value::Null);
    };
    Ok(label.map_or(Value::Null,|label|json!({"contents":{"kind":"markdown","value":format!("```sanim\n{label}\n```")},"range":document.range(range)})))
}

mod navigation;
pub(crate) use navigation::definition;

pub(crate) fn document_symbols(workspace: &Workspace, uri: &str) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let Some((_, module)) = workspace.module(uri) else {
        return Ok(json!([]));
    };
    Ok(json!(symbols(module,document.text.len()).into_iter().filter(|symbol|symbol.root).map(|symbol| {
        let clause=(symbol.kind==12).then(||declaration_signature(module,&symbol.name,symbol.span)).flatten();
        let detail=clause.map(|signature|Type::Function(signature.clone()).to_string()).or_else(||symbol.ty.map(|ty|ty.to_string())).unwrap_or_default();
        json!({"name":symbol.name,"kind":symbol.kind,"detail":detail,"range":document.range(symbol.span),"selectionRange":document.range(symbol.selection)})
    }).collect::<Vec<_>>()))
}

struct CallPosition {
    span: Span,
    arguments: Vec<(Option<String>, Expr)>,
    callee: Expr,
    index: usize,
    named: Option<String>,
    value: Option<Span>,
}
fn call_position(
    analysis: &crate::workspace::Analysis,
    module: &AnalyzedModule,
    offset: usize,
) -> Option<CallPosition> {
    let mut call: Option<Expr> = None;
    expressions(&module.ast, &mut |expression| {
        if matches!(&expression.kind,ExprKind::Call {callee,..} if callee.span.end>callee.span.start)
            && expression.span.start <= offset
            && offset <= expression.span.end
            && call.as_ref().is_none_or(|best| {
                expression.span.end - expression.span.start < best.span.end - best.span.start
            })
        {
            call = Some(expression.clone());
        }
    });
    if let Some(Expr {
        kind: ExprKind::Call { callee, args, .. },
        span,
        ..
    }) = call
    {
        let index = args
            .iter()
            .position(|(_, value)| offset <= value.span.end)
            .unwrap_or(args.len());
        Some(CallPosition {
            callee: *callee,
            index,
            named: args.get(index).and_then(|(name, _)| name.clone()),
            value: args.get(index).map(|(_, value)| value.span),
            span,
            arguments: args,
        })
    } else {
        analysis
            .pending_calls
            .iter()
            .filter(|pending| pending.span.start <= offset && offset <= pending.span.end)
            .min_by_key(|pending| pending.span.end - pending.span.start)
            .map(|pending| CallPosition {
                callee: pending.callee.clone(),
                span: pending.span,
                arguments: Vec::new(),
                index: pending.argument,
                named: pending.argument_name.clone(),
                value: pending.argument_span,
            })
    }
}

pub(crate) fn signature(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let offset = document.offset(position)?;
    let Some((analysis, module)) = workspace.module(uri) else {
        return Ok(Value::Null);
    };
    let Some(call) = call_position(analysis, module, offset) else {
        return Ok(Value::Null);
    };
    let native = match &call.callee.kind {
        ExprKind::Name(name) => prelude::builtin(name).map(|builtin| (builtin, false)),
        _ => native_method(module, document.text.len(), &call.callee, offset)
            .map(|builtin| (builtin, true)),
    };
    let mut active_signature = 0;
    let entries = if let Some((builtin, receiver)) = native {
        let (_, mut parameters) = native_signature(builtin, receiver);
        let bindings = module
            .checked
            .call_types
            .get(&(call.span.start, call.span.end));
        for (index, (name, label)) in parameters.iter_mut().enumerate() {
            if builtin.parameters[index + usize::from(receiver)].ty != Type::Unknown {
                continue;
            }
            let argument = call
                .arguments
                .iter()
                .find(|(key, _)| key.as_ref() == Some(name))
                .or_else(|| {
                    call.arguments
                        .iter()
                        .filter(|(key, _)| key.is_none())
                        .nth(index)
                });
            if let Some((_, argument)) = argument
                && let Some(mut actual) =
                    argument_type(module, document.text.len(), argument, offset)
            {
                if let Some(bindings) = bindings {
                    for signature in actual.signatures_mut() {
                        for parameter in &mut signature.parameters {
                            parameter.ty = parameter.ty.substitute(bindings);
                        }
                        *signature.result = signature.result.substitute(bindings);
                        signature
                            .generics
                            .retain(|generic| !bindings.contains_key(&generic.name));
                    }
                }
                *label = format!("{name}: {actual}");
            }
        }
        let name = if receiver {
            builtin.name.rsplit('.').next().unwrap_or(builtin.name)
        } else {
            builtin.name
        };
        let result = module
            .checked
            .result_types
            .get(&(call.span.start, call.span.end))
            .or_else(|| (builtin.result != Type::Unknown).then_some(&builtin.result));
        let label = format!(
            "{name}({}){}",
            parameters
                .iter()
                .map(|(_, label)| label.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            result.map(|ty| format!(" -> {ty}")).unwrap_or_default()
        );
        vec![signature_information(
            label,
            &parameters,
            call.index,
            call.named.as_deref(),
        )]
    } else {
        let Some((name, callable)) =
            callable_type(module, document.text.len(), &call.callee, offset)
        else {
            return Ok(Value::Null);
        };
        active_signature = callable
            .signatures()
            .iter()
            .position(|signature| {
                applicable_signature(module, document.text.len(), &call, signature, offset, false)
            })
            .unwrap_or(0);
        callable
            .signatures()
            .iter()
            .map(|signature| {
                let parameters = signature
                    .parameters
                    .iter()
                    .map(|parameter| {
                        (
                            parameter.name.clone(),
                            format!(
                                "{}{}: {}",
                                match parameter.kind {
                                    BindingKind::Let => "",
                                    BindingKind::Mut => "mut ",
                                    BindingKind::Sig => "sig ",
                                },
                                parameter.name,
                                parameter.ty
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                signature_information(
                    format!("{name}: {}", Type::Function(signature.clone())),
                    &parameters,
                    call.index,
                    call.named.as_deref(),
                )
            })
            .collect::<Vec<_>>()
    };
    if entries.is_empty() {
        return Ok(Value::Null);
    }
    let active = entries[active_signature]["activeParameter"].clone();
    Ok(json!({"signatures":entries,"activeSignature":active_signature,"activeParameter":active}))
}
fn signature_information(
    label: String,
    parameters: &[(String, String)],
    index: usize,
    named: Option<&str>,
) -> Value {
    let active = named
        .and_then(|name| {
            parameters
                .iter()
                .position(|(parameter, _)| parameter == name)
        })
        .unwrap_or(index)
        .min(parameters.len().saturating_sub(1));
    json!({"label":label,"parameters":parameters.iter().map(|(_,label)|json!({"label":label})).collect::<Vec<_>>(),"activeParameter":active})
}

pub(crate) fn inlay_hints(
    workspace: &mut Workspace,
    uri: &str,
    range: &Value,
) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let start = document.offset(&range["start"])?;
    let end = document.offset(&range["end"])?;
    if end < start {
        return Err("inlay hint range ends before it starts".into());
    }
    let Some((_, module)) = workspace.module(uri) else {
        return Ok(json!([]));
    };
    let mut hints = BTreeMap::new();
    for statement in &module.ast.statements {
        statement.visit(&mut |statement| {
            if let StatementKind::Bind {annotation:None,..}=&statement.kind
                && let Some(span)=statement.name_span
                && let Some(ty)=module.checked.binding_types.get(&statement.span.start)
                && span.end>=start&&span.end<=end {
                let position = document.position(span.end);
                let mut hint = json!({"position":position,"label":format!(": {}",crate::type_display::compact(module,ty)),"kind":1,"paddingRight":false,
                    "tooltip":{"kind":"markdown","value":crate::type_display::tooltip(module,ty)}});
                if let Some(annotation) = crate::type_display::annotation(module,ty) {
                    hint["textEdits"] = json!([{"range":{"start":position,"end":position},"newText":format!(": {annotation}")}]);
                }
                hints.insert((span.end,0),hint);
            }
        });
    }
    let program_end = document.text.trim_end().len();
    let program_position = document.position(program_end);
    let entry = module.checked.output_type == Some(Type::Animation);
    if entry
        && start <= program_end
        && program_end <= end
        && let Some(layout) = workspace.animation_layout(uri)
    {
        let size = layout
            .duration
            .map(|duration| duration.to_string())
            .unwrap_or_else(|| {
                if layout
                    .constraint
                    .is_some_and(|constraint| constraint.contains("match_parent"))
                {
                    "match_parent".into()
                } else {
                    "unresolved".into()
                }
            });
        hints.insert((program_end,1),json!({"position":program_position,"label":format!("duration: {size}"),"paddingLeft":true,"tooltip":"Total duration of the script's unassigned top-level animations."}));
    }
    Ok(json!(hints.into_values().collect::<Vec<_>>()))
}

/// Standard LSP token names; Zed and VS Code map these to their native themes.
pub(crate) const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "function",
    "variable",
    "parameter",
    "property",
    "string",
    "number",
    "keyword",
    "comment",
    "operator",
];

pub(crate) fn semantic_tokens(workspace: &Workspace, uri: &str) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let Some((_, module)) = workspace.module(uri) else {
        return Ok(json!({"data":[]}));
    };
    let mut tokens = BTreeMap::<(usize, usize), (usize, u32)>::new();
    let opened = &workspace.documents[uri];
    let syntax = opened.syntax();
    for span in &syntax.highlights {
        let kind = match span.kind.as_ref() {
            "type" => 1,
            "function" | "tag" => 2,
            "property" => 5,
            "string" => 6,
            "number" => 7,
            "keyword" => 8,
            "comment" => 9,
            "operator" | "punctuation" => 10,
            _ => 3,
        };
        tokens.insert((span.start, span.end), (kind, 0));
    }
    let declarations = symbols(module, document.text.len());
    for symbol in &declarations {
        if symbol.selection.end > symbol.selection.start
            && symbol.selection.end - symbol.selection.start == symbol.name.len()
        {
            let kind = match symbol.kind {
                2 => 0,
                26 | 10 => 1,
                12 => 2,
                _ => 3,
            };
            tokens.insert((symbol.selection.start, symbol.selection.end), (kind, 1));
        }
    }
    let mut absolute = Vec::new();
    let mut covered_until = 0;
    for ((start, end), (kind, modifiers)) in tokens {
        if start < covered_until
            || end <= start
            || !document.text.is_char_boundary(start)
            || !document.text.is_char_boundary(end)
        {
            continue;
        }
        for (offset, text) in document.span_lines(start, end) {
            let position = document.position(offset);
            absolute.push((
                position["line"].as_u64().unwrap(),
                position["character"].as_u64().unwrap(),
                text.encode_utf16().count(),
                kind,
                modifiers,
            ));
        }
        covered_until = end;
    }
    let mut data = Vec::new();
    let (mut line, mut character) = (0, 0);
    for (next_line, next_character, length, kind, modifiers) in absolute {
        data.extend([
            next_line - line,
            if next_line == line {
                next_character - character
            } else {
                next_character
            },
            length as u64,
            kind as u64,
            u64::from(modifiers),
        ]);
        (line, character) = (next_line, next_character);
    }
    Ok(json!({"data":data}))
}
