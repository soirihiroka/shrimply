//! Element-owned animation methods share one typed registry and timeline scheduler.
use crate::{
    native::{Context, number},
    prelude::{Builtin, Execution, Parameter},
};
use sanim_core::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    MoveTo,
    Shift,
    RotateTo,
    ScaleTo,
    FillTo,
    FadeIn,
    FadeOut,
    Create,
    Uncreate,
    Write,
    Unwrite,
    GrowFromCenter,
    ShrinkToCenter,
    ExpandX,
    StretchXTo,
}
pub(crate) struct Definition {
    pub builtin: Builtin,
    wrapper: &'static str,
    kind: Kind,
}
pub(crate) fn definitions() -> &'static [Definition] {
    static DEFINITIONS: std::sync::OnceLock<Vec<Definition>> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let target = Type::union([
            Type::Element,
            Type::List(Box::new(Type::Element), ListLength::Unknown),
        ])
        .expect("element target union");
        let position = Type::union([
            Type::List(
                Box::new(Type::Numeric(Unit::Length)),
                sanim_core::ListLength::Exact(2),
            ),
            Type::List(
                Box::new(Type::Numeric(Unit::Percent)),
                sanim_core::ListLength::Exact(2),
            ),
        ])
        .expect("position unit union");
        let p = |name, ty, required| Parameter { name, ty, required };
        [
            (Kind::MoveTo, "Element.move_to"),
            (Kind::Shift, "Element.shift"),
            (Kind::RotateTo, "Element.rotate_to"),
            (Kind::ScaleTo, "Element.scale_to"),
            (Kind::FillTo, "Element.fill_to"),
            (Kind::FadeIn, "Element.fade_in"),
            (Kind::FadeOut, "Element.fade_out"),
            (Kind::Create, "Element.create"),
            (Kind::Uncreate, "Element.uncreate"),
            (Kind::Write, "Element.write"),
            (Kind::Unwrite, "Element.unwrite"),
            (Kind::GrowFromCenter, "Element.grow_from_center"),
            (Kind::ShrinkToCenter, "Element.shrink_to_center"),
            (Kind::ExpandX, "Element.expand_x"),
            (Kind::StretchXTo, "Element.stretch_x_to"),
        ]
        .into_iter()
        .map(|(kind, name)| {
            let mut parameters = vec![p("element", target.clone(), true)];
            let to = match kind {
                Kind::MoveTo | Kind::Shift => Some(position.clone()),
                Kind::RotateTo => Some(Type::Numeric(Unit::Angle)),
                Kind::ScaleTo | Kind::StretchXTo => Some(Type::Numeric(Unit::Number)),
                Kind::FillTo => Some(Type::Color),
                _ => None,
            };
            if let Some(to) = to {
                parameters.push(p("to", to, true));
            }
            parameters.extend([
                p("duration", crate::animation_plan::duration_type(), false),
                p("ease", Type::String, false),
            ]);
            if matches!(kind, Kind::ExpandX | Kind::StretchXTo) {
                parameters.extend([
                    p("origin", Type::Numeric(Unit::Percent), false),
                    p("offset", Type::Numeric(Unit::Length), false),
                ]);
            }
            let easing = match kind {
                Kind::FadeIn
                | Kind::FadeOut
                | Kind::FillTo
                | Kind::Create
                | Kind::Uncreate
                | Kind::Write
                | Kind::Unwrite => Easing::Smootherstep,
                _ => Easing::Spring,
            };
            let mut builtin = Builtin {
                name,
                parameters,
                result: Type::Animation,
                state_effect: false,
                content: Default::default(),
                timing: crate::timing::Timing {
                    default: Fraction::new(3u64, 10u64),
                    easing,
                },
                execute: Execution::Native(execute),
                check: crate::native_types::fixed,
            };
            crate::prelude::finalize(&mut builtin);
            Definition {
                builtin,
                wrapper: Box::leak(format!("{kind:?}").into_boxed_str()),
                kind,
            }
        })
        .collect()
    })
}
pub(crate) fn definition(name: &str) -> Option<&'static Definition> {
    static INDEX: std::sync::OnceLock<
        std::collections::BTreeMap<&'static str, &'static Definition>,
    > = std::sync::OnceLock::new();
    INDEX
        .get_or_init(|| {
            definitions()
                .iter()
                .flat_map(|definition| {
                    [
                        (definition.builtin.name, definition),
                        (
                            definition
                                .builtin
                                .name
                                .rsplit('.')
                                .next()
                                .expect("animation method name"),
                            definition,
                        ),
                    ]
                })
                .collect()
        })
        .get(name)
        .copied()
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    let eligible = definitions()[0].builtin.parameters[0].ty.accepts(receiver);
    definitions()
        .iter()
        .filter(move |_| eligible)
        .map(|definition| &definition.builtin)
}

fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let definition = definition(name).expect("registered animation");
    let easing = args
        .get("ease")
        .map(Value::string)
        .transpose()?
        .map(Easing::parse)
        .transpose()?
        .unwrap_or(definition.builtin.timing.easing);
    crate::animation_plan::sized_effect(
        name,
        args.clone(),
        definition.builtin.timing.default,
        easing,
        Default::default(),
    )
}

pub(crate) fn wrappers() -> impl Iterator<Item = Builtin> {
    definitions().iter().map(|definition| {
        let mut builtin = definition.builtin.clone();
        builtin.name = definition.wrapper;
        builtin.parameters.remove(0);
        builtin.parameters.push(Parameter {
            name: "children",
            ty: crate::animation_plan::children_type(),
            required: false,
        });
        builtin.execute = Execution::Native(crate::animation_plan::wrapper);
        builtin
    })
}
pub(crate) fn wrapper_operation(name: &str) -> Option<&'static str> {
    definitions()
        .iter()
        .find(|definition| definition.wrapper == name)
        .map(|definition| definition.builtin.name)
}
pub(crate) fn execute_effect(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let targets = args
        .get("element")
        .ok_or("animation requires an element or a list of elements")?;
    let elements: Vec<_> = match targets {
        Value::Element(element) => vec![element.as_ref()],
        Value::List(items) => items
            .iter()
            .map(|item| match item {
                Value::Element(element) => Ok(element.as_ref()),
                _ => Err("animation targets must all be Element values"),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("animation requires Element or List<Element> targets".into()),
    };
    context.charge_work(elements.len())?;
    if elements.is_empty() {
        return Err(format!(
            "{name}: target list is empty; provide at least one Element"
        ));
    }
    let definition =
        definition(name).ok_or_else(|| format!("unknown animation operation {name}"))?;
    let motions = elements
        .into_iter()
        .map(|element| {
            motion(
                definition,
                args,
                element,
                context.animation,
                *context.cursor,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    context.animation.schedule_in_window(
        &motions,
        *context.cursor,
        context.duration,
        context.easing,
        math::temporal::Window {
            cycle: context.cycle,
            clip: context.clip,
        },
    )?;
    *context.cursor = math::temporal::writer_end(*context.cursor, context.duration)?;
    Ok(Value::Unit)
}

fn motion(
    definition: &Definition,
    args: &imbl::OrdMap<String, Value>,
    element: &Element,
    animation: &Animation,
    cursor: Fraction,
) -> Result<Motion, String> {
    let name = definition.builtin.name;
    let kind = definition.kind;
    let required = |name: &str| {
        args.get(name)
            .ok_or_else(|| format!("missing argument '{name}'"))
    };
    let (property, to, from) = match kind {
        Kind::MoveTo => (Property::Position, required("to")?.clone(), None),
        Kind::Shift => {
            let offset = required("to")?.vec2()?;
            let mut current = animation
                .sample_property(element, Property::Position, cursor)?
                .vec2()?;
            if current.values.iter().all(|v| *v == 0.0) {
                current.unit = offset.unit;
            }
            if offset.unit != current.unit {
                return Err(
                    "shift and current position must both use pt or both use percentages".into(),
                );
            }
            (
                Property::Position,
                Value::from_vector(
                    vec![
                        math::float_arithmetic("+", current.values[0], offset.values[0])?,
                        math::float_arithmetic("+", current.values[1], offset.values[1])?,
                    ],
                    current.unit,
                )?,
                None,
            )
        }
        Kind::FadeIn => (
            Property::Opacity,
            Value::Number(Number {
                value: element.opacity,
                unit: Unit::Number,
            }),
            Some(Value::Number(Number::scalar(0))),
        ),
        Kind::FadeOut => (Property::Opacity, Value::Number(Number::scalar(0)), None),
        Kind::Create | Kind::Write => (
            Property::Reveal,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        Kind::Uncreate | Kind::Unwrite => {
            (Property::Reveal, Value::Number(Number::scalar(0)), None)
        }
        Kind::GrowFromCenter | Kind::ShrinkToCenter => {
            if element.layout.x_align != Align::Center || element.layout.y_align != Align::Center {
                return Err(format!(
                    "{name} requires x_align: Align.Center and y_align: Align.Center; use scale_to for an edge-anchored element"
                ));
            }
            if kind == Kind::GrowFromCenter {
                (
                    Property::Scale,
                    Value::Number(Number {
                        value: element.scale,
                        unit: Unit::Number,
                    }),
                    Some(Value::Number(Number::scalar(0))),
                )
            } else {
                (Property::Scale, Value::Number(Number::scalar(0)), None)
            }
        }
        Kind::RotateTo => (Property::Rotation, required("to")?.clone(), None),
        Kind::ScaleTo => {
            if required("to")?.number()?.value <= Fraction::from(0) {
                return Err("scale must be positive".into());
            }
            (Property::Scale, required("to")?.clone(), None)
        }
        Kind::ExpandX => (
            Property::StretchX,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        Kind::StretchXTo => {
            let to = required("to")?.number()?;
            if !math::finite(&to.value) || to.value < Fraction::from(0) {
                return Err("horizontal scale must be finite and nonnegative".into());
            }
            (Property::StretchX, required("to")?.clone(), None)
        }
        Kind::FillTo => {
            if !matches!(
                animation.sample_property(element, Property::Fill, cursor)?,
                Value::Color(_)
            ) {
                return Err("fill_to currently interpolates solid Color fills; a LinearGradient can be set as a static fill, but its stops cannot be interpolated. Animate opacity or transform the gradient-filled element instead.".into());
            }
            (Property::Fill, required("to")?.clone(), None)
        }
    };
    Ok(Motion {
        element: element.clone(),
        property,
        to,
        from,
        reveal: match kind {
            Kind::Create | Kind::Uncreate => Some(RevealKind::Create),
            Kind::Write | Kind::Unwrite => Some(RevealKind::Write),
            _ => None,
        },
        horizontal: if kind == Kind::ExpandX
            || kind == Kind::StretchXTo
                && (args.contains_key("origin") || args.contains_key("offset"))
        {
            Some(HorizontalTransform {
                scale: Fraction::from(1),
                origin: Number {
                    value: number(
                        args,
                        "origin",
                        if matches!(element.geometry.as_ref(), Geometry::Box { .. }) {
                            if element.horizontal == HorizontalTransform::default() {
                                Fraction::new(1u64, 2u64)
                            } else {
                                element.horizontal.origin.value
                            }
                        } else {
                            Fraction::from(0)
                        },
                    )?,
                    unit: Unit::Percent,
                },
                offset: number(args, "offset", element.horizontal.offset)?,
            })
        } else {
            None
        },
        easing: None,
    })
}
