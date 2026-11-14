//! Scroll operations use the same tracks, native layout and viewport bounds as rendering.
use crate::{
    native::Context,
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::*;
use std::sync::OnceLock;

fn registry() -> &'static [Builtin] {
    static METHODS: OnceLock<Vec<Builtin>> = OnceLock::new();
    METHODS.get_or_init(|| {
        let dimension = Type::union([Type::Numeric(Unit::Length), Type::Numeric(Unit::Percent)])
            .expect("scroll units");
        let mut method = Builtin {
            name: "Element.scroll_to",
            parameters: vec![
                Parameter {
                    name: "view",
                    ty: Type::Element,
                    required: true,
                },
                Parameter {
                    name: "to",
                    ty: Type::union([
                        Type::Vector(2, Some(Unit::Length)),
                        Type::Vector(2, Some(Unit::Percent)),
                    ])
                    .expect("scroll vector units"),
                    required: false,
                },
                Parameter {
                    name: "x",
                    ty: dimension.clone(),
                    required: false,
                },
                Parameter {
                    name: "y",
                    ty: dimension,
                    required: false,
                },
                Parameter {
                    name: "line",
                    ty: Type::Numeric(Unit::Number),
                    required: false,
                },
                Parameter {
                    name: "range",
                    ty: code::values::range_type(),
                    required: false,
                },
                Parameter {
                    name: "align",
                    ty: Type::String,
                    required: false,
                },
            ],
            result: Type::Unit,
            schedule: true,
            content: Default::default(),
            timing: crate::timing::Timing {
                default: Fraction::new(3u64, 10u64),
                ..Default::default()
            },
            execute: Execution::Native(execute),
            check: crate::native_types::fixed,
        };
        crate::prelude::finalize(&mut method);
        vec![method]
    })
}
pub(crate) fn builtin(name: &str) -> Option<&'static Builtin> {
    registry().iter().find(|builtin| builtin.name == name)
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    let supported = Type::Element.accepts(receiver);
    registry().iter().filter(move |_| supported)
}

fn layout_work(elements: &[Element]) -> usize {
    let mut pending: Vec<_> = elements.iter().collect();
    let mut work = 0usize;
    while let Some(element) = pending.pop() {
        work = work.saturating_add(1);
        if let Geometry::Code { document, .. } = element.geometry.as_ref() {
            work = work.saturating_add(document.text.len());
        }
        if let Some(children) = element.geometry.children() {
            pending.extend(children);
        }
    }
    work
}

pub(crate) fn execute(
    _name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let Some(Value::Element(view)) = args.get("view") else {
        return Err("scroll_to requires a scrollable element receiver".into());
    };
    if !view.layout.clips() {
        return Err(format!(
            "{}.scroll_to requires a container with overflow=\"auto\", \"scroll\" or \"hidden\" and explicit width and height",
            view.id
        ));
    }
    if context
        .animation
        .reactive
        .binding(&view.id, Property::Scroll)
        .is_some()
    {
        return Err(format!(
            "{}.scroll is signal-owned; animate the mutable offset instead of calling scroll_to",
            view.id
        ));
    }
    let targets = usize::from(args.contains_key("to"))
        + usize::from(args.contains_key("x") || args.contains_key("y"))
        + usize::from(args.contains_key("line"))
        + usize::from(args.contains_key("range"));
    if targets != 1 {
        return Err("scroll_to requires one target: to=Vec2(...), x=/y=, line= or range=".into());
    }
    if args.contains_key("align") && !args.contains_key("line") && !args.contains_key("range") {
        return Err("scroll alignment applies to line= or range= targets".into());
    }
    let start = *context.cursor;
    if context.animation.find(&view.id).is_none() {
        context.animation.add(view.as_ref().clone(), start)?;
    }
    context.charge_work(context.animation.element_count())?;
    let (width, height) = context.animation.logical_size();
    let sampled = context
        .animation
        .layout_with(start, width, height, |element, time| {
            sanim_data::code::view::resolve_pending(element, time)?;
            sanim_data::font::resolve_pending(element)
        })?;
    context.charge_work(layout_work(&sampled))?;
    let rendered = sampled
        .iter()
        .find_map(|element| element.find(&view.id))
        .ok_or("scroll target is not visible at this timeline time")?;
    let maximum = rendered.layout.scroll_range;
    let from = rendered
        .layout
        .scroll
        .expect("scroll viewport retained by native layout");
    let mut to = from;
    if let Some(value) = args.get("to") {
        to = value.vec2()?;
    } else if args.contains_key("x") || args.contains_key("y") {
        to.unit = args
            .get("x")
            .or_else(|| args.get("y"))
            .expect("axis target")
            .number()?
            .unit;
        if to.unit == Unit::Percent {
            for (axis, maximum) in maximum.into_iter().enumerate() {
                to.values[axis] = if maximum == 0. {
                    0.
                } else {
                    from.values[axis] / maximum
                };
            }
        }
        for (axis, name) in ["x", "y"].into_iter().enumerate() {
            if let Some(value) = args.get(name) {
                let value = value.number()?;
                if value.unit != to.unit {
                    return Err("scroll_to x= and y= must use the same unit; use points for both or percentages for both".into());
                }
                to.values[axis] = math::to_f64(value.value)?;
            }
        }
    } else {
        let Geometry::Code { document, .. } = rendered.geometry.as_ref() else {
            return Err("line= and range= scrolling require CodeView".into());
        };
        context.charge_work(document.text.len() + document.scopes.len())?;
        let selected = if let Some(value) = args.get("range") {
            code::values::range(value)?
        } else {
            let line = args
                .get("line")
                .expect("source line target")
                .number()?
                .usize()?;
            code::view::Range {
                start_line: line,
                start_column: 1,
                end_line: line.checked_add(1).ok_or("source line overflow")?,
                end_column: 1,
                origin: None,
            }
        };
        let [top, bottom] = sanim_data::code::view::range_bounds(rendered, &selected)?;
        let viewport = sanim_core::scrolling::viewport(rendered)?;
        let viewport_height = viewport[3] - viewport[1];
        to.values[1] = match args
            .get("align")
            .map(Value::string)
            .transpose()?
            .unwrap_or("nearest")
        {
            "nearest" => {
                if top < from.values[1] {
                    top
                } else if bottom > from.values[1] + viewport_height {
                    if bottom - top > viewport_height {
                        top
                    } else {
                        bottom - viewport_height
                    }
                } else {
                    from.values[1]
                }
            }
            "start" => top,
            "center" => (top + bottom - viewport_height) / 2.,
            "end" => bottom - viewport_height,
            other => {
                return Err(format!(
                    "unknown scroll alignment {other:?}; use nearest, start, center or end"
                ));
            }
        };
    }
    sanim_core::scrolling::validate(rendered, to)?;
    for (axis, maximum) in maximum.into_iter().enumerate() {
        to.values[axis] = to.values[axis].clamp(
            0.,
            if to.unit == Unit::Percent {
                1.
            } else {
                maximum
            },
        );
    }
    let from = if to.unit == Unit::Percent {
        Vec2 {
            values: [
                if maximum[0] == 0. {
                    0.
                } else {
                    from.values[0] / maximum[0]
                },
                if maximum[1] == 0. {
                    0.
                } else {
                    from.values[1] / maximum[1]
                },
            ],
            unit: Unit::Percent,
        }
    } else {
        from
    };
    context.animation.schedule(
        &[Motion {
            element: view.as_ref().clone(),
            property: Property::Scroll,
            from: Some(Value::Vec2(from)),
            to: Value::Vec2(to),
            reveal: None,
            horizontal: None,
            easing: None,
        }],
        start,
        context.duration,
        context.easing,
    )?;
    Ok(Value::Unit)
}
