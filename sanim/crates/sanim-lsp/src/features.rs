//! Navigation and completion consume the compiler AST and checked type metadata.
use crate::{text::Document, workspace::Workspace};
use sanim_lang::{
    AnalyzedModule,
    ast::{BindingKind, Expr, ExprKind, Module, Statement, StatementKind},
    core::{Span, TimelineEffect, Type},
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
fn symbols(module: &AnalyzedModule, length: usize) -> Vec<Symbol> {
    fn block(
        body: &[Statement],
        scope: Span,
        module: &AnalyzedModule,
        root: bool,
        out: &mut Vec<Symbol>,
    ) {
        for statement in body {
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
                    let signature = match module.checked.exports.get(name) {
                        Some(Type::Function(signature)) => Some(signature),
                        _ => None,
                    };
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
                StatementKind::Repeat { body, .. } | StatementKind::Background { body } => {
                    branch(body, statement.span, module, out)
                }
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
    symbols(module, length)
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
        statement.visit_expressions(visitor);
        statement.visit(&mut |statement| {
            if let StatementKind::Function {
                parameters, body, ..
            } = &statement.kind
            {
                for parameter in parameters {
                    let default = &parameter.default;
                    if let Some(default) = default {
                        default.visit(visitor);
                    }
                }
                for statement in body {
                    statement.visit_expressions(visitor);
                }
            }
        });
    }
}
fn expression_at(module: &Module, offset: usize) -> Option<Expr> {
    let mut best: Option<Expr> = None;
    expressions(module, &mut |expression| {
        if expression.span.end > expression.span.start
            && expression.span.start <= offset
            && offset <= expression.span.end
            && matches!(expression.kind, ExprKind::Name(_) | ExprKind::Member(_, _))
            && best.as_ref().is_none_or(|best| {
                expression.span.end - expression.span.start < best.span.end - best.span.start
            })
        {
            best = Some(expression.clone());
        }
    });
    best
}
fn native_signature(builtin: &prelude::Builtin, receiver: bool) -> (String, Vec<(String, String)>) {
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
        format!(" -> {}", builtin.result)
    };
    let label = format!(
        "{name}({}){result}",
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

/// The canonical parser owns opening tags, including incomplete attributes.
/// The replacement covers the entire utility token, rather than the identifier
/// suffix following its last hyphen.
fn markup_position(
    syntax: &crate::workspace::Syntax,
    offset: usize,
) -> Option<(&sanim_lang::parser::MarkupOpening, Span)> {
    if syntax
        .highlights
        .iter()
        .any(|span| span.kind.as_ref() == "comment" && span.start <= offset && offset < span.end)
    {
        return None;
    }
    let opening = syntax
        .markup
        .iter()
        .rev()
        .find(|opening| opening.span.start <= offset && offset <= opening.span.end)?;
    if opening.attributes.iter().any(|attribute| {
        attribute
            .value
            .is_some_and(|value| value.start <= offset && offset <= value.end)
    }) {
        return None;
    }
    let span = opening
        .attributes
        .iter()
        .find(|attribute| attribute.span.start <= offset && offset <= attribute.span.end)
        .map_or(
            Span {
                start: offset,
                end: offset,
                ..Default::default()
            },
            |attribute| attribute.span,
        );
    Some((opening, span))
}

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
                let signature = match &call.callee.kind {
                    ExprKind::Name(name) => ty(module, document.text.len(), name, offset)?,
                    _ => return None,
                };
                let Type::Function(signature) = signature else {
                    return None;
                };
                match call.named {
                    Some(name) => signature
                        .parameters
                        .iter()
                        .find(|parameter| parameter.name == name),
                    None => signature.parameters.get(call.index),
                }?
                .ty
                .clone()
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
    let values = expected.string_literals()?;
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
) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let offset = document.offset(position)?;
    let syntax = workspace.documents[uri].syntax();
    if let Some(items) = literal_completion(workspace, uri, offset, &syntax) {
        return Ok(items);
    }
    if let Some((opening, span)) = markup_position(&syntax, offset) {
        let prefix = &document.text[span.start..offset];
        let mut items = sanim_lang::utilities::completions(&opening.name, prefix)
            .into_iter()
            .map(|utility| {
                json!({
                    "label":utility.token,"kind":20,"detail":utility.detail(),
                    "documentation":{"kind":"markdown","value":utility.markdown()},
                    "textEdit":{"range":document.range(span),"newText":utility.token},
                })
            })
            .collect::<Vec<_>>();
        if let Some(builtin) = prelude::builtin(&opening.name) {
            items.extend(
                builtin
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.name.starts_with(prefix))
                    .map(|parameter| {
                        json!({
                            "label":parameter.name,"kind":10,"detail":parameter.ty.to_string(),
                            "textEdit":{"range":document.range(span),"newText":parameter.name},
                        })
                    }),
            );
        }
        return Ok(json!({"isIncomplete":false,"items":items}));
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
                if let Type::Record(fields) = receiver {
                    return Ok(
                        json!({"isIncomplete":false,"items":fields.into_iter().map(|(name,ty)|json!({"label":name,"kind":5,"detail":ty.to_string()})).collect::<Vec<_>>()}),
                    );
                }
                if matches!(receiver, Type::DataFrame { .. } | Type::Series(_)) {
                    let mut members = sanim_lang::prelude::methods_for(&receiver)
                        .map(|builtin| json!({"label":builtin.name.rsplit('.').next().unwrap_or(builtin.name),"kind":2,"detail":native_signature(builtin,true).0}))
                        .collect::<Vec<_>>();
                    if matches!(receiver, Type::DataFrame { .. }) {
                        members.push(json!({"label":"columns","kind":5,"detail":"List<String>"}));
                    }
                    return Ok(json!({"isIncomplete":false,"items":members}));
                }
            }
        }
    }
    for builtin in prelude::builtins() {
        items.entry(builtin.name.into()).or_insert_with(||json!({"label":builtin.name,"kind":3,"detail":builtin_signature(builtin.name).map(|(label,_)|label),"documentation":"Native Sanim operation. Optional duration= and ease= controls use the language's ordered timeline semantics."}));
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
    let (label, mut effect_span, range, function_effect) = if let Some(symbol) = declaration {
        let effect = match &symbol.ty {
            Some(Type::Function(signature)) => Some(signature.duration),
            _ => None,
        };
        (
            Some(
                symbol
                    .ty
                    .map(|ty| format!("{}: {ty}", symbol.name))
                    .unwrap_or(symbol.name),
            ),
            symbol.span,
            symbol.selection,
            effect,
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
        (label, expression.span, expression.span, None)
    } else {
        return Ok(Value::Null);
    };
    expressions(&module.ast, &mut |node| {
        if let ExprKind::Call { callee, .. } = &node.kind
            && callee.span.start <= offset
            && offset <= callee.span.end
            && callee.span.end > callee.span.start
        {
            effect_span = node.span;
        }
    });
    let effect = function_effect
        .or_else(|| {
            module
                .checked
                .durations
                .iter()
                .find(|node| {
                    node.span.start == effect_span.start && node.span.end == effect_span.end
                })
                .map(|node| node.effect)
        })
        .unwrap_or(TimelineEffect::Dependent);
    let duration = duration_label(effect);
    Ok(label.map_or(Value::Null,|label|json!({"contents":{"kind":"markdown","value":format!("```sanim\n{label}\n```\nTimeline duration: **{duration}**")},"range":document.range(range)})))
}

pub(crate) fn definition(
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
    let Some(expression) = expression_at(&module.ast, offset) else {
        return Ok(Value::Null);
    };
    let target = match expression.kind {
        ExprKind::Name(name) => find_symbol(module, document.text.len(), &name, offset)
            .filter(|symbol| symbol.selection.end > symbol.selection.start)
            .map(|symbol| (uri.to_owned(), document.range(symbol.selection))),
        ExprKind::Member(base, name) => {
            if let ExprKind::Name(alias) = base.kind {
                module.ast.statements.iter().find_map(|statement| {
                    let StatementKind::Import {
                        path,
                        alias: import_alias,
                    } = &statement.kind
                    else {
                        return None;
                    };
                    if import_alias != &alias {
                        return None;
                    };
                    let source = sanim_lang::source::resolve(&analysis.entry, path).ok()?;
                    let imported = analysis.modules.get(&source)?;
                    let imported_uri = workspace.uri_for_path(&analysis.root.join(&source)).ok()?;
                    let text = workspace.document_text(&imported_uri)?;
                    let document = Document::new(text, 0);
                    symbols(imported, document.text.len())
                        .into_iter()
                        .find(|symbol| symbol.root && symbol.name == name)
                        .map(|symbol| (imported_uri, document.range(symbol.selection)))
                })
            } else {
                None
            }
        }
        _ => None,
    };
    Ok(target.map_or(Value::Null, |(uri, range)| json!({"uri":uri,"range":range})))
}

pub(crate) fn document_symbols(workspace: &Workspace, uri: &str) -> Result<Value, String> {
    let document = &workspace
        .documents
        .get(uri)
        .ok_or("document is not open")?
        .document;
    let Some((_, module)) = workspace.module(uri) else {
        return Ok(json!([]));
    };
    Ok(json!(symbols(module,document.text.len()).into_iter().filter(|symbol|symbol.root).map(|symbol|json!({"name":symbol.name,"kind":symbol.kind,"detail":symbol.ty.map(|ty|ty.to_string()).unwrap_or_default(),"range":document.range(symbol.span),"selectionRange":document.range(symbol.selection)})).collect::<Vec<_>>()))
}

struct CallPosition {
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
        })
    } else {
        analysis
            .pending_calls
            .iter()
            .filter(|pending| pending.span.start <= offset && offset <= pending.span.end)
            .min_by_key(|pending| pending.span.end - pending.span.start)
            .map(|pending| CallPosition {
                callee: pending.callee.clone(),
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
    let Some(CallPosition {
        callee,
        index,
        named,
        ..
    }) = call_position(analysis, module, offset)
    else {
        return Ok(Value::Null);
    };
    let native = if let ExprKind::Name(name) = &callee.kind {
        builtin_signature(name)
    } else {
        native_method(module, document.text.len(), &callee, offset)
            .map(|builtin| native_signature(builtin, true))
    };
    let signature = native.or_else(|| {
        let (name, ty) = match &callee.kind {
            ExprKind::Name(name) => (name.clone(), ty(module, document.text.len(), name, offset)?),
            ExprKind::Member(base, name) => {
                if let ExprKind::Name(base) = &base.kind {
                    let Type::Record(mut fields) = ty(module, document.text.len(), base, offset)?
                    else {
                        return None;
                    };
                    (format!("{base}.{name}"), fields.remove(name)?)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let Type::Function(signature) = ty else {
            return None;
        };
        Some((
            format!("{name}: {}", Type::Function(signature.clone())),
            signature
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
                .collect(),
        ))
    });
    let Some((label, parameters)) = signature else {
        return Ok(Value::Null);
    };
    let active = named
        .as_ref()
        .and_then(|name| {
            parameters
                .iter()
                .position(|(parameter, _)| parameter == name)
        })
        .unwrap_or(index)
        .min(parameters.len().saturating_sub(1));
    Ok(
        json!({"signatures":[{"label":label,"parameters":parameters.iter().map(|(_,label)|json!({"label":label})).collect::<Vec<_>>()}],"activeSignature":0,"activeParameter":active}),
    )
}

fn duration_label(effect: TimelineEffect) -> String {
    match effect {
        TimelineEffect::Fixed(duration) => format!("{duration}s"),
        TimelineEffect::Dependent => {
            "dependent on arguments/data; resolved during preparation".into()
        }
    }
}

pub(crate) fn inlay_hints(
    workspace: &Workspace,
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
    let durations = module
        .checked
        .durations
        .iter()
        .map(|node| ((node.span.start, node.span.end), node.effect))
        .collect::<BTreeMap<_, _>>();
    let mut hints = BTreeMap::new();
    for statement in &module.ast.statements {
        statement.visit(&mut |statement| {
            if let StatementKind::Bind {annotation:None,..}=&statement.kind
                && let Some(span)=statement.name_span
                && let Some(ty)=module.checked.binding_types.get(&statement.span.start)
                && span.end>=start&&span.end<=end {
                hints.insert((span.end,0),json!({"position":document.position(span.end),"label":format!(": {}",crate::type_display::compact(module,ty)),"kind":1,"paddingRight":false,
                    "tooltip":{"kind":"markdown","value":crate::type_display::tooltip(module,ty)}}));
            }
            let effect=if let StatementKind::Function {name,..}=&statement.kind {
                match module.checked.exports.get(name) {Some(Type::Function(signature))=>Some(signature.duration),_=>None}
            } else {durations.get(&(statement.span.start,statement.span.end)).copied()};
            if let Some(effect)=effect.filter(|effect|!effect.is_zero()) {
                let offset=document.text[..statement.span.end.min(document.text.len())].trim_end().len();
                if offset>=start&&offset<=end {
                    hints.insert((offset,1),json!({"position":document.position(offset),"label":format!("duration: {}",duration_label(effect)),"paddingLeft":true}));
                }
            }
        });
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
