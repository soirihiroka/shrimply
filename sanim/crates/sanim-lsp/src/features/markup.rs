//! Native-markup completion uses the grammar's indexed source ranges and constructor metadata.
use crate::{text::Document, workspace::Syntax};
use sanim_lang::{
    core::{Color, Span, Type},
    parser::{MarkupAttribute, MarkupOpening},
    prelude,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

fn opening_at(syntax: &Syntax, offset: usize) -> Option<&MarkupOpening> {
    let mut index = syntax
        .markup
        .partition_point(|opening| opening.name_span.start <= offset)
        .checked_sub(1)?;
    loop {
        let opening = &syntax.markup[index];
        if opening.element_span.start <= offset && offset <= opening.element_span.end {
            if opening.name_span.start <= offset && offset <= opening.name_span.end
                || opening
                    .closing
                    .is_some_and(|span| span.start <= offset && offset <= span.end)
                || opening.span.start <= offset && offset <= opening.span.end
            {
                return Some(opening);
            }
            // A literal CodeView body is foreign source, not native markup or Sanim expressions.
            if matches!(
                prelude::builtin(&opening.name).map(|builtin| builtin.content),
                Some(prelude::MarkupContent::Literal { .. })
            ) {
                return Some(opening);
            }
        }
        index = opening.parent?;
    }
}

pub(super) fn non_code(syntax: &Syntax, offset: usize) -> bool {
    syntax
        .highlights
        .partition_point(|span| span.start <= offset)
        .checked_sub(1)
        .and_then(|index| syntax.highlights.get(index))
        .is_some_and(|span| {
            offset <= span.end && matches!(span.kind.as_ref(), "string" | "comment")
        })
}

pub(super) fn attribute_position(syntax: &Syntax, offset: usize) -> Option<(&MarkupOpening, Span)> {
    let opening = opening_at(syntax, offset)?;
    if offset < opening.span.start || offset > opening.span.end || non_code(syntax, offset) {
        return None;
    }
    if opening.attributes.iter().any(|attribute| {
        attribute
            .value
            .is_some_and(|span| span.start <= offset && offset <= span.end)
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

fn tags() -> &'static BTreeMap<&'static str, &'static prelude::Builtin> {
    static TAGS: OnceLock<BTreeMap<&'static str, &'static prelude::Builtin>> = OnceLock::new();
    TAGS.get_or_init(|| {
        prelude::all_builtins()
            .filter(|builtin| {
                matches!(
                    builtin.result,
                    Type::Element | Type::Animation | Type::VoxelLayout
                ) && builtin.name.chars().next().is_some_and(char::is_uppercase)
                    && !builtin.name.contains('.')
            })
            .map(|builtin| (builtin.name, builtin))
            .collect()
    })
}

fn value_completion(
    document: &Document,
    opening: &MarkupOpening,
    attribute: &MarkupAttribute,
    offset: usize,
) -> Option<Value> {
    let parameter = prelude::builtin(&opening.name)?
        .parameters
        .iter()
        .find(|parameter| parameter.name == attribute.name)?;
    let span = attribute.value?;
    if !(span.start <= offset && offset <= span.end) {
        return None;
    }
    let raw = &document.text[span.start..span.end];
    let quoted = raw.starts_with(['\'', '"']);
    let colors =
        quoted && parameter.ty.accepts(&Type::Color) && !parameter.ty.accepts(&Type::String);
    if let Type::Enum { name, variants } = &parameter.ty {
        let braced = document.text[..span.start].trim_end().ends_with('{');
        return Some(
            json!({"isIncomplete":false,"items":variants.iter().filter(|(_, fields)| fields.is_empty()).map(|(variant,_)| {
            let value = format!("{name}.{variant}");
            json!({"label":value,"kind":20,"detail":parameter.ty.to_string(),"textEdit":{"range":document.range(span),"newText":if braced { value.clone() } else { format!("{{{value}}}") }}})
        }).collect::<Vec<_>>()}),
        );
    }
    let values: Vec<String> = if colors {
        Color::names().map(str::to_ascii_lowercase).collect()
    } else if !parameter.ty.string_literal_choices().is_empty() {
        parameter
            .ty
            .string_literal_choices()
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else if parameter.ty == Type::Bool && !quoted {
        let braced = document.text[..span.start].trim_end().ends_with('{');
        return Some(
            json!({"isIncomplete":false,"items":(["true","false"].into_iter().map(|value| json!({"label":value,"kind":12,"textEdit":{"range":document.range(span),"newText":if braced { value.to_owned() } else { format!("{{{value}}}") }}})).collect::<Vec<_>>())}),
        );
    } else {
        return None;
    };
    Some(
        json!({"isIncomplete":false,"items":values.into_iter().map(|value| {
        let literal = sanim_lang::core::template::quote_string(&value);
        json!({"label":literal,"kind":20,"detail":parameter.ty.to_string(),"textEdit":{"range":document.range(span),"newText":literal}})
    }).collect::<Vec<_>>()}),
    )
}

pub(super) fn completion(
    document: &Document,
    syntax: &Syntax,
    offset: usize,
    snippets: bool,
) -> Option<Value> {
    let opening = opening_at(syntax, offset)?;
    if opening.name_span.start <= offset && offset <= opening.name_span.end {
        let prefix = &document.text[opening.name_span.start..offset];
        let insert_body = snippets
            && opening.name_span.end == document.text.len()
            && opening.attributes.is_empty()
            && !opening.self_closing;
        let items: Vec<_> = tags().iter().filter(|(name, _)| name.starts_with(prefix)).map(|(name, builtin)| {
            let body = matches!(builtin.content, prelude::MarkupContent::Literal { .. })
                || builtin.parameters.iter().any(|parameter| matches!(parameter.name, "children" | "text"));
            let text = if insert_body {
                if body { format!("{name} $1>$0</{name}>") } else { format!("{name} $0/>") }
            } else { (*name).to_owned() };
            let mut item = json!({"label":name,"kind":7,"detail":super::native_signature(builtin,false).0,"textEdit":{"range":document.range(opening.name_span),"newText":text}});
            if insert_body { item["insertTextFormat"] = json!(2); }
            item
        }).collect();
        return Some(json!({"isIncomplete":false,"items":items}));
    }
    if let Some(span) = opening
        .closing
        .filter(|span| span.start <= offset && offset <= span.end)
    {
        let prefix = &document.text[span.start..offset];
        let items = if opening.name.starts_with(prefix) {
            vec![
                json!({"label":opening.name,"kind":7,"detail":"Close the innermost native element","textEdit":{"range":document.range(span),"newText":format!("{}{}",opening.name,if document.text[span.end..].starts_with('>') { "" } else { ">" })}}),
            ]
        } else {
            vec![]
        };
        return Some(json!({"isIncomplete":false,"items":items}));
    }
    if offset < opening.span.start || offset > opening.span.end {
        return matches!(
            prelude::builtin(&opening.name).map(|builtin| builtin.content),
            Some(prelude::MarkupContent::Literal { .. })
        )
        .then(|| json!({"isIncomplete":false,"items":[]}));
    }
    for attribute in &opening.attributes {
        if attribute
            .value
            .is_some_and(|span| span.start <= offset && offset <= span.end)
        {
            return value_completion(document, opening, attribute, offset);
        }
    }
    let (_, span) = attribute_position(syntax, offset)?;
    let prefix = &document.text[span.start..offset];
    let used: Vec<_> = opening
        .attributes
        .iter()
        .filter(|attribute| attribute.span != span)
        .collect();
    let explicit: BTreeSet<_> = used
        .iter()
        .filter(|attribute| !attribute.utility)
        .map(|attribute| attribute.name.as_str())
        .collect();
    let mut items: Vec<_> = sanim_lang::utilities::completions(&opening.name, prefix).into_iter()
        .filter(|utility| !utility.property_names().any(|property| explicit.contains(property)))
        .filter(|utility| !used.iter().any(|attribute| attribute.name == utility.token || attribute.utility && sanim_lang::utilities::conflicts(&attribute.name,&utility.token)))
        .map(|utility| json!({"label":utility.token,"kind":20,"detail":utility.detail(),"documentation":{"kind":"markdown","value":utility.markdown()},"textEdit":{"range":document.range(span),"newText":utility.token}})).collect();
    if let Some(builtin) = prelude::builtin(&opening.name) {
        let assigned = document.text[span.end..].trim_start().starts_with('=');
        items.extend(builtin.parameters.iter().filter(|parameter| parameter.name.starts_with(prefix) && !explicit.contains(parameter.name) && !used.iter().any(|attribute|attribute.utility && sanim_lang::utilities::conflicts(&attribute.name,parameter.name))).map(|parameter| {
            let text = if snippets && !assigned {
                let quoted = !parameter.ty.string_literal_choices().is_empty() || parameter.ty == Type::String || parameter.ty.accepts(&Type::Color) && !parameter.ty.accepts(&Type::String);
                if quoted { format!("{}=\"$1\"$0",parameter.name) } else { format!("{}={{$1}}$0",parameter.name) }
            } else { parameter.name.to_owned() };
            let mut item = json!({"label":parameter.name,"kind":10,"detail":parameter.ty.to_string(),"textEdit":{"range":document.range(span),"newText":text}});
            if snippets && !assigned { item["insertTextFormat"] = json!(2); }
            item
        }));
    }
    Some(json!({"isIncomplete":false,"items":items}))
}
