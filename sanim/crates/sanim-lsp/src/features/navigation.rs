//! Definition lookup shares parser locations, lexical symbols and checked method dispatch.
use super::*;
use crate::stdlib;

pub(crate) fn definition(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
) -> Result<Value, String> {
    let opened = workspace.documents.get(uri).ok_or("document is not open")?;
    let document = &opened.document;
    let offset = document.offset(position)?;
    let Some((analysis, module)) = workspace.module(uri) else {
        return Ok(Value::Null);
    };
    let syntax = opened.syntax();
    let reference = syntax
        .type_references
        .iter()
        .find(|reference| reference.span.start <= offset && offset < reference.span.end);
    let local = |name: &str| {
        definition_symbols(
            module,
            document.text.len(),
            name,
            offset,
            &syntax.declarations,
            reference.is_some(),
        )
        .into_iter()
        .map(|symbol| json!({"uri":uri,"range":document.range(symbol.selection)}))
        .collect::<Vec<_>>()
    };
    let named = |name: &str| {
        let targets = local(name);
        if !targets.is_empty() {
            return targets;
        }
        if let Some((alias, member)) = name.split_once('.') {
            let targets = imported(workspace, analysis, module, alias, Some(member));
            if !targets.is_empty() {
                return targets;
            }
        }
        stdlib::location(name).into_iter().collect()
    };
    let expression = reference
        .is_none()
        .then(|| expression_at(&module.ast, offset))
        .flatten();
    let argument = if reference.is_none() && expression.is_none() {
        call_position(analysis, module, offset)
    } else {
        None
    }
    .and_then(|call| {
        let name = call.named.as_ref()?;
        if call.value.is_none_or(|span| offset >= span.start)
            || !syntax.highlights.iter().any(|highlight| {
                highlight.start <= offset
                    && offset < highlight.end
                    && &document.text[highlight.start..highlight.end] == name
            })
        {
            return None;
        }
        let builtin = match &call.callee.kind {
            ExprKind::Name(name) => prelude::builtin(name),
            _ => native_method(module, document.text.len(), &call.callee, offset),
        }?;
        stdlib::location(builtin.name)
    });
    let mut targets = if let Some(target) = argument {
        vec![target]
    } else if let Some(reference) = reference {
        named(&reference.name)
    } else if let Some(expression) = expression {
        if let Some(builtin) = native_method(module, document.text.len(), &expression, offset) {
            stdlib::location(builtin.name).into_iter().collect()
        } else {
            match expression.kind {
                ExprKind::Name(name) => named(&name),
                ExprKind::Member(base, name) => {
                    if let ExprKind::Name(alias) = base.kind {
                        let qualified = format!("{alias}.{name}");
                        let targets = local(&qualified);
                        if !targets.is_empty() {
                            targets
                        } else {
                            let targets =
                                imported(workspace, analysis, module, &alias, Some(&name));
                            if targets.is_empty() {
                                stdlib::location(&qualified).into_iter().collect()
                            } else {
                                targets
                            }
                        }
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            }
        }
    } else if let Some(highlight) = syntax.highlights.iter().find(|highlight| {
        highlight.start <= offset
            && offset < highlight.end
            && matches!(
                highlight.kind.as_ref(),
                "tag" | "type" | "variable" | "function"
            )
    }) {
        named(&document.text[highlight.start..highlight.end])
    } else {
        Vec::new()
    };
    // Import strings open the source file, including imports not already open in the editor.
    if targets.is_empty()
        && syntax.highlights.iter().any(|highlight| {
            highlight.start <= offset
                && offset < highlight.end
                && highlight.kind.as_ref() == "string"
        })
    {
        for statement in &module.ast.statements {
            if statement.span.start <= offset
                && offset < statement.span.end
                && let StatementKind::Import { alias, .. } = &statement.kind
            {
                targets = imported(workspace, analysis, module, alias, None);
                break;
            }
        }
    }
    Ok(match targets.len() {
        0 => Value::Null,
        1 => targets.pop().expect("one definition"),
        _ => json!(targets),
    })
}

fn imported(
    workspace: &Workspace,
    analysis: &crate::workspace::Analysis,
    module: &AnalyzedModule,
    alias: &str,
    member: Option<&str>,
) -> Vec<Value> {
    module
        .ast
        .statements
        .iter()
        .find_map(|statement| {
            let StatementKind::Import {
                path,
                alias: import_alias,
            } = &statement.kind
            else {
                return None;
            };
            if import_alias != alias {
                return None;
            }
            let source = sanim_lang::source::resolve(&analysis.entry, path).ok()?;
            let imported = analysis.modules.get(&source)?;
            let uri = workspace.uri_for_path(&analysis.root.join(&source)).ok()?;
            let document = Document::new(workspace.document_text(&uri)?, 0);
            Some(if let Some(name) = member {
                definition_symbols(
                    imported,
                    document.text.len(),
                    name,
                    document.text.len(),
                    &[],
                    false,
                )
                .into_iter()
                .filter(|symbol| symbol.root)
                .map(|symbol| json!({"uri":uri,"range":document.range(symbol.selection)}))
                .collect()
            } else {
                vec![json!({"uri":uri,"range":document.range(Span::default())})]
            })
        })
        .unwrap_or_default()
}

fn definition_symbols(
    module: &AnalyzedModule,
    length: usize,
    name: &str,
    offset: usize,
    declarations: &[sanim_lang::parser::Declaration],
    type_context: bool,
) -> Vec<Symbol> {
    let qualified = name.rsplit_once('.');
    let candidates = symbols(module, length).into_iter().chain(
        declarations
            .iter()
            .filter(|declaration| match &declaration.kind {
                sanim_lang::parser::DeclarationKind::TypeParameter => declaration.name == name,
                sanim_lang::parser::DeclarationKind::EnumVariant(qualifier) => qualified
                    .is_some_and(|(parent, member)| {
                        declaration.name == member && qualifier.as_ref() == parent
                    }),
            })
            .map(|declaration| Symbol {
                name: name.into(),
                ty: None,
                span: declaration.span,
                selection: declaration.span,
                scope: declaration.scope,
                kind: match &declaration.kind {
                    sanim_lang::parser::DeclarationKind::TypeParameter => 26,
                    sanim_lang::parser::DeclarationKind::EnumVariant(_) => 22,
                },
                forward: true,
                root: matches!(
                    &declaration.kind,
                    sanim_lang::parser::DeclarationKind::EnumVariant(_)
                ),
            }),
    );
    let Some(symbol) = select_symbol(
        candidates.filter(|symbol| !type_context || matches!(symbol.kind, 2 | 10 | 26)),
        name,
        offset,
    )
    .filter(|symbol| symbol.selection.end > symbol.selection.start) else {
        return Vec::new();
    };
    if symbol
        .ty
        .as_ref()
        .is_some_and(|ty| ty.signatures().len() > 1)
    {
        symbols(module, length)
            .into_iter()
            .filter(|candidate| {
                candidate.kind == 12 && candidate.name == name && candidate.scope == symbol.scope
            })
            .collect()
    } else {
        vec![symbol]
    }
}
