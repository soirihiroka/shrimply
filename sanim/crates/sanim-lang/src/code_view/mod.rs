//! Native source-code elements and timeline presentation methods.
mod edits;
mod patch_edit;
mod selectors;
mod syntax;
use crate::{
    native::Context,
    native_types,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::{
    code::{
        Language,
        view::{Options, Range},
    },
    *,
};
use std::sync::Arc;

use sanim_core::code::values::*;

pub(crate) fn register(output: &mut Vec<Builtin>, common: Vec<Parameter>) {
    let p = |name, ty, required| Parameter { name, ty, required };
    for operation in Native::CODE_VALUES {
        let (parameters, result) = operation.signature();
        output.push(Builtin {
            name: operation.name(),
            parameters: parameters
                .into_iter()
                .map(|(name, ty, required)| p(name, ty, required))
                .collect(),
            result,
            schedule: false,
            timing: Default::default(),
            content: Default::default(),
            execute: Execution::Native(execute),
            check: native_types::fixed,
        });
    }
    let mut parameters = vec![
        p("src", Type::String, false),
        p("code", Type::String, false),
        p("lang", Type::String, false),
        p("font", Type::Font, false),
        p("font_size", Type::Numeric(Unit::Length), false),
        p("line_height", Type::Numeric(Unit::Length), false),
        p("tab_size", Type::Numeric(Unit::Number), false),
        p("line_numbers", Type::Bool, false),
        p("scope_indicators", Type::Bool, false),
        p("theme", theme_type(), false),
        p(
            "spans",
            Type::List(Box::new(span_type()), sanim_core::ListLength::Unknown),
            false,
        ),
    ];
    parameters.extend(common);
    output.push(Builtin {
        name: "CodeView",
        parameters,
        result: Type::Element,
        schedule: false,
        timing: Default::default(),
        content: crate::prelude::MarkupContent::Literal {
            parameter: "code",
            dedent: true,
        },
        execute: Execution::Source(execute),
        check: check_source,
    });
}
fn check_source(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let src = actuals.first().is_some_and(|ty| *ty != Type::Unit);
    let code = actuals.get(1).is_some_and(|ty| *ty != Type::Unit);
    if src == code {
        return Err("provide exactly one CodeView source: src=\"file.py\", code=String, or a literal markup body".into());
    }
    Ok(builtin.result.clone())
}

fn check_query(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    if builtin.name == "CodeView.range"
        && !builtin
            .parameters
            .iter()
            .zip(actuals)
            .any(|(parameter, ty)| {
                selectors::QUERY_KINDS.contains(&parameter.name) && *ty != Type::Unit
            })
    {
        return Err("CodeView.range requires name=, level=, line=, character=, or regex=; use view.ranges() to list all syntax ranges".into());
    }
    Ok(builtin.result.clone())
}
fn check_edit(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let present = |name| {
        builtin
            .parameters
            .iter()
            .zip(actuals)
            .any(|(parameter, ty)| parameter.name == name && *ty != Type::Unit)
    };
    if present("code") == present("src") {
        return Err(
            "range.edit requires exactly one replacement: code=String or src=\"file.py\"".into(),
        );
    }
    Ok(builtin.result.clone())
}

fn methods() -> &'static [Builtin] {
    static METHODS: std::sync::OnceLock<Vec<Builtin>> = std::sync::OnceLock::new();
    METHODS.get_or_init(|| {
        let p = |name, ty, required| Parameter { name, ty, required };
        let mut methods = Vec::new();
        for (name, arguments, result, schedule) in [
            (
                "CodeView.patch",
                vec![
                    p("src", Type::String, true),
                    p("animation", Type::String, false),
                    p("file", Type::String, false),
                    p("range", range_type(), false),
                ],
                Type::Unit,
                true,
            ),
            (
                "CodeView.select",
                vec![p(
                    "ranges",
                    Type::List(Box::new(range_type()), sanim_core::ListLength::Unknown),
                    false,
                )],
                Type::Unit,
                true,
            ),
            (
                "CodeView.style",
                vec![
                    p(
                        "spans",
                        Type::List(Box::new(span_type()), sanim_core::ListLength::Unknown),
                        false,
                    ),
                    p("theme", theme_type(), false),
                    p("line_numbers", Type::Bool, false),
                    p("scope_indicators", Type::Bool, false),
                ],
                Type::Unit,
                true,
            ),
            (
                "CodeView.fold",
                vec![p("range", range_type(), true)],
                Type::Unit,
                true,
            ),
            (
                "CodeView.unfold",
                vec![p("range", range_type(), true)],
                Type::Unit,
                true,
            ),
            (
                "CodeView.range",
                selectors::parameters(false),
                range_type(),
                false,
            ),
            (
                "CodeView.ranges",
                selectors::parameters(true),
                Type::List(Box::new(range_type()), sanim_core::ListLength::Unknown),
                false,
            ),
            ("CodeRange.fold", vec![], Type::Unit, true),
            ("CodeRange.unfold", vec![], Type::Unit, true),
            ("CodeRange.select", vec![], Type::Unit, true),
            (
                "CodeRange.scroll_to",
                vec![p("align", Type::String, false)],
                Type::Unit,
                true,
            ),
            (
                "CodeRange.style",
                vec![p("style", style_type(), true)],
                Type::Unit,
                true,
            ),
            (
                "CodeRange.edit",
                vec![
                    p("code", Type::String, false),
                    p("src", Type::String, false),
                    p("animation", Type::String, false),
                    p("file", Type::String, false),
                ],
                Type::Unit,
                true,
            ),
        ] {
            let mut parameters = vec![if name.starts_with("CodeRange.") {
                p("range", range_type(), true)
            } else {
                p("view", Type::Element, true)
            }];
            parameters.extend(arguments);
            let mut builtin = Builtin {
                name,
                parameters,
                result,
                schedule,
                timing: crate::timing::Timing {
                    default: if schedule {
                        Fraction::new(3u64, 10u64)
                    } else {
                        0.into()
                    },
                    ..Default::default()
                },
                content: Default::default(),
                execute: if matches!(name, "CodeView.patch" | "CodeRange.edit") {
                    Execution::Source(execute)
                } else if schedule {
                    Execution::Native(execute)
                } else {
                    Execution::Access(execute)
                },
                check: if matches!(name, "CodeView.range" | "CodeView.ranges") {
                    check_query
                } else if name == "CodeRange.edit" {
                    check_edit
                } else {
                    native_types::fixed
                },
            };
            crate::prelude::finalize(&mut builtin);
            methods.push(builtin);
        }
        methods
    })
}
pub(crate) fn builtin(name: &str) -> Option<&'static Builtin> {
    methods().iter().find(|method| method.name == name)
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    methods().iter().filter(move |method| {
        if method.name.starts_with("CodeRange.") {
            range_type().accepts(receiver)
        } else {
            *receiver == Type::Element
        }
    })
}
fn current_document(
    view: &Value,
    context: &Context<'_>,
) -> Result<Arc<sanim_core::code::Document>, String> {
    let Value::Element(view) = view else {
        return Err("code method requires CodeView".into());
    };
    let Geometry::Code { document, .. } = view.geometry.as_ref() else {
        return Err(format!("'{}' is not a CodeView", view.id));
    };
    let document = context
        .animation
        .code_views
        .get(&view.id)
        .map(|state| state.presentation(*context.cursor))
        .transpose()?
        .flatten()
        .map(|presentation| {
            if presentation.appearance >= 1.into() {
                presentation.to.document.clone()
            } else {
                presentation.from.document.clone()
            }
        })
        .unwrap_or_else(|| document.clone());
    sample_document(view, document, context)
}

/// Apply retained code/language inputs to the caller's current source revision.
fn sample_document(
    view: &Element,
    document: Arc<sanim_core::code::Document>,
    context: &Context<'_>,
) -> Result<Arc<sanim_core::code::Document>, String> {
    let bound = |property| {
        context
            .animation
            .reactive
            .binding(&view.id, property)
            .is_some()
    };
    if !bound(Property::Code) && !bound(Property::CodeLanguage) {
        return Ok(document);
    }
    let text = if bound(Property::Code) {
        match context
            .animation
            .sample_property(view, Property::Code, *context.cursor)?
        {
            Value::String(text) => text,
            _ => return Err("CodeView code source must be String".into()),
        }
    } else {
        document.text.clone()
    };
    let language = if bound(Property::CodeLanguage) {
        Language::parse(
            context
                .animation
                .sample_property(view, Property::CodeLanguage, *context.cursor)?
                .string()?,
        )?
    } else {
        document.language
    };
    let language = if language == Language::Auto {
        Language::detect(&document.path)
    } else {
        language
    };
    if text == document.text && language == document.language {
        return Ok(document);
    }
    sanim_core::code::Document::source_shared(&document.path, text, language)
        .and_then(syntax::parse)
        .map(Arc::new)
}

fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    syntax::install();
    if name.starts_with("CodeRange.") {
        return selectors::action(name, args, context);
    }
    if let Some(operation) = Native::from_name(name) {
        return operation.execute(args);
    }

    match name {
        "CodeView" => {
            let document = if let Some(value) = args.get("document") {
                document(value)?
            } else {
                if args.contains_key("src") {
                    return Err("CodeView src was not loaded by the source runtime".into());
                }
                let text = args.get("code").ok_or(
                    r#"CodeView requires src="file.py", code=String, or a literal markup body"#,
                )?;
                let Value::String(text) = text else {
                    return Err("CodeView code must be String".into());
                };
                let language = Language::parse(
                    args.get("lang")
                        .map(Value::string)
                        .transpose()?
                        .unwrap_or("plain"),
                )?;
                Arc::new(sanim_core::code::Document::source_shared(
                    "",
                    text.clone(),
                    language,
                )?)
            };
            document.source()?;
            context.charge_work(document.text.len())?;
            let document = Arc::new(syntax::parse(document.as_ref().clone())?);
            let font = match args.get("font") {
                Some(Value::Font(font)) => font.clone(),
                None => Font::family("monospace", "monospace")?,
                _ => return Err("CodeView font requires Font".into()),
            };
            let size = crate::native::number(args, "font_size", 24.into())?;
            let line_height =
                crate::native::number(args, "line_height", size * Fraction::new(3u64, 2u64))?;
            if size <= 0.into() || line_height <= 0.into() {
                return Err("CodeView font_size and line_height must be positive".into());
            }
            let tab_size = args
                .get("tab_size")
                .map(|value| integer(value, "tab_size"))
                .transpose()?
                .unwrap_or(4);
            if !(1..=32).contains(&tab_size) {
                return Err("CodeView tab_size must be an integer from 1 to 32".into());
            }
            let options = Arc::new(Options {
                font,
                font_size: size,
                line_height,
                auto_line_height: !args.contains_key("line_height"),
                tab_size,
                line_numbers: args
                    .get("line_numbers")
                    .map(Value::boolean)
                    .transpose()?
                    .unwrap_or(false),
                scope_indicators: args
                    .get("scope_indicators")
                    .map(Value::boolean)
                    .transpose()?
                    .unwrap_or(false),
                theme: Arc::new(
                    args.get("theme")
                        .map(theme)
                        .transpose()?
                        .unwrap_or_default(),
                ),
                spans: args
                    .get("spans")
                    .map(spans)
                    .transpose()?
                    .unwrap_or_default(),
            });
            let mut group_args = args.clone();
            if !group_args.contains_key("overflow")
                && args.contains_key("width")
                && args.contains_key("height")
            {
                group_args.insert("overflow".into(), Value::String("auto".into()));
            }
            group_args.remove("font");
            group_args.insert(
                "children".into(),
                Value::List(List::typed(Type::Element, vec![])),
            );
            let Value::Element(mut root) = crate::native::composition(
                &group_args,
                context.module,
                context.identity,
                context.span,
            )?
            else {
                unreachable!()
            };
            sanim_core::code::view::validate_ranges(
                &root.id,
                &document,
                &options
                    .spans
                    .iter()
                    .map(|span| span.range.clone())
                    .collect::<Vec<_>>(),
            )?;
            let element = Arc::make_mut(&mut root);
            element.geometry = Arc::new(Geometry::Code {
                document,
                options,
                pending: true,
                source_changed: false,
                presentation: None,
                children: Default::default(),
            });
            element.layout.x_align = match args.get("x_align") {
                Some(Value::Variant { tag, .. }) => Align::from_name(tag)?,
                _ => Align::Center,
            };
            element.layout.y_align = match args.get("y_align") {
                Some(Value::Variant { tag, .. }) => Align::from_name(tag)?,
                _ => Align::Center,
            };
            Ok(Value::Element(root))
        }
        "CodeView.range" | "CodeView.ranges" => {
            selectors::query(args, context, name == "CodeView.ranges")
        }
        _ => edits::execute(name, args, context),
    }
}
