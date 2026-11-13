//! Direct animation functions: build tracks once, then advance through shared call timing.
use crate::{native::{Context, number}, prelude::{Builtin, Execution, Parameter}};
use sanim_core::*;

pub(crate) fn register(out: &mut Vec<Builtin>) {
    let target = Type::union([Type::Element, Type::List(Box::new(Type::Element))]).expect("element target union");
    let p = |name, ty| Parameter { name, ty, required: true };
    let mut add = |name, mut parameters:Vec<Parameter>| {
        parameters.insert(0,p("element",target.clone()));
        out.push(Builtin {name,parameters,result:Type::Unit,schedule:true,
            timing:crate::timing::Timing {default:Fraction::new(3u64,10u64),..Default::default()},
            execute:Execution::Native(execute),check:crate::native_types::fixed});
    };
    for (name,ty) in [
        ("MoveTo",Type::Vector(2,None)),("Shift",Type::Vector(2,None)),
        ("RotateTo",Type::Numeric(Unit::Angle)),("ScaleTo",Type::Numeric(Unit::Number)),("FillTo",Type::Color),
    ] { add(name,vec![p("to",ty)]); }
    for name in ["FadeIn","FadeOut","Create","Uncreate","Write","Unwrite","GrowFromCenter","ShrinkToCenter"] {add(name,vec![]);}
    for name in ["ExpandX","StretchXTo"] {
        let mut parameters=vec![];
        if name=="StretchXTo" {parameters.push(p("to",Type::Numeric(Unit::Number)));}
        parameters.extend([
            Parameter{name:"origin",ty:Type::Numeric(Unit::Percent),required:false},
            Parameter{name:"offset",ty:Type::Numeric(Unit::Length),required:false},
        ]);
        add(name,parameters);
    }
}

fn execute(name:&str,args:&imbl::OrdMap<String,Value>,context:Context<'_>)->Result<Value,String> {
    let targets=args.get("element").ok_or("animation requires an element or a list of elements")?;
    let elements:Vec<_>=match targets {
        Value::Element(element)=>vec![element.as_ref()],
        Value::List(items)=>items.iter().map(|item|match item {
            Value::Element(element)=>Ok(element.as_ref()),
            _=>Err("animation targets must all be Element values"),
        }).collect::<Result<_,_>>()?,
        _=>return Err("animation requires Element or List<Element> targets".into()),
    };
    if elements.is_empty() {return Err(format!("{name}: target list is empty; provide at least one Element"));}
    let motions=elements.into_iter().map(|element|motion(name,args,element,context.animation,*context.cursor,context.easing)).collect::<Result<Vec<_>,_>>()?;
    context.animation.schedule(&motions,*context.cursor,context.duration,context.easing)?;
    *context.cursor=math::timeline_end(*context.cursor,context.duration)?;
    Ok(Value::Unit)
}

fn motion(name:&str,args:&imbl::OrdMap<String,Value>,element:&Element,animation:&Animation,cursor:Fraction,easing:Easing)->Result<Motion,String> {
    let required=|name:&str|args.get(name).ok_or_else(||format!("missing argument '{name}'"));
    let (property, to, from) = match name {
        "MoveTo" => (Property::Position, required("to")?.clone(), None),
        "Shift" => {
            let offset = required("to")?.vec2()?;
            let mut current = animation
                .sample_property(element, Property::Position, cursor)?
                .vec2()?;
            if current.values.iter().all(|v| *v == 0.0) {
                current.unit = offset.unit;
            }
            if offset.unit != current.unit {
                return Err(
                    "Shift and current position must both use pt or both use percentages".into(),
                );
            }
            (
                Property::Position,
                Value::Vec2(Vec2 {
                    values: [
                        math::float_arithmetic("+", current.values[0], offset.values[0])?,
                        math::float_arithmetic("+", current.values[1], offset.values[1])?,
                    ],
                    unit: current.unit,
                }),
                None,
            )
        }
        "FadeIn" => (
            Property::Opacity,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        "FadeOut" => (Property::Opacity, Value::Number(Number::scalar(0)), None),
        "Create" | "Write" => (
            Property::Reveal,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        "Uncreate" | "Unwrite" => (Property::Reveal, Value::Number(Number::scalar(0)), None),
        "GrowFromCenter" | "ShrinkToCenter" => {
            if element.layout.x_align != Align::Center || element.layout.y_align != Align::Center {
                return Err(format!(
                    "{name} requires x_align: Align.Center and y_align: Align.Center; use ScaleTo for an edge-anchored element"
                ));
            }
            if name == "GrowFromCenter" {
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
        "RotateTo" => (Property::Rotation, required("to")?.clone(), None),
        "ScaleTo" => {
            if required("to")?.number()?.value <= Fraction::from(0) {
                return Err("scale must be positive".into());
            }
            (Property::Scale, required("to")?.clone(), None)
        }
        "ExpandX" => (
            Property::StretchX,
            Value::Number(Number::scalar(1)),
            Some(Value::Number(Number::scalar(0))),
        ),
        "StretchXTo" => {
            let to = required("to")?.number()?;
            if !math::finite(&to.value) || to.value < Fraction::from(0) {
                return Err("horizontal scale must be finite and nonnegative".into());
            }
            (Property::StretchX, required("to")?.clone(), None)
        }
        _ => (Property::Fill, required("to")?.clone(), None),
    };
    Ok(Motion {
        element: element.clone(),
        property,
        to,
        from,
        reveal: match name {
            "Create" | "Uncreate" => Some(RevealKind::Create),
            "Write" | "Unwrite" => Some(RevealKind::Write),
            _ => None,
        },
        horizontal: if name == "ExpandX"
            || name == "StretchXTo" && (args.contains_key("origin") || args.contains_key("offset"))
        {
            Some(HorizontalTransform {
                scale: Fraction::from(1),
                origin: Number {
                    value: number(
                        args,
                        "origin",
                        if matches!(element.geometry.as_ref(), Geometry::Group { .. }) {
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
        easing: Some(easing),
    })
}
