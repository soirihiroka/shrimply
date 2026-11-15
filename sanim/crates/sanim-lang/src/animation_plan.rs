//! Pure temporal composition. Only the returned root is lowered into the shared scene IR.
mod cells;
use crate::{
    Limits,
    native::{self, Context},
    prelude::{Builtin, Execution, Parameter},
};
pub(crate) use cells::write_cell;
use sanim_core::{
    plan::{self, Flow, Node, Style, TimeDimension},
    *,
};
use std::sync::Arc;

pub(crate) fn duration_type() -> Type {
    Type::union([
        Type::Numeric(Unit::Duration),
        Type::StringLiteral("auto".into()),
        Type::StringLiteral("match_parent".into()),
    ])
    .expect("animation duration sizing")
}
pub(crate) fn dimension(value: Option<&Value>) -> Result<TimeDimension, String> {
    match value {
        None => Ok(TimeDimension::Auto),
        Some(Value::String(s)) if s.as_ref() == "auto" => Ok(TimeDimension::Auto),
        Some(Value::String(s)) if s.as_ref() == "match_parent" => Ok(TimeDimension::MatchParent),
        Some(value) => {
            let number = value.number()?;
            if number.unit != Unit::Duration {
                return Err("Animation duration needs time units, auto or match_parent".into());
            }
            Ok(TimeDimension::Fixed(number.value))
        }
    }
}
pub(crate) fn sized_effect(
    operation: &str,
    arguments: imbl::OrdMap<String, Value>,
    default: Fraction,
    easing: Easing,
    children: imbl::Vector<Arc<plan::Animation>>,
) -> Result<Value, String> {
    let requested = dimension(arguments.get("duration"))?;
    let duration = if let TimeDimension::Fixed(duration) = requested {
        duration
    } else {
        default
    };
    math::temporal::nonnegative("duration", duration)?;
    let mut animation = plan::Animation::new(Node::Effect {
        effect: plan::Effect {
            operation: operation.into(),
            arguments,
            duration,
            easing,
        },
        children,
    });
    animation.style.duration = requested;
    Ok(Value::Animation(Arc::new(animation)))
}
pub(crate) fn children_type() -> Type {
    Type::List(
        Box::new(Type::union([Type::Animation, Type::Element]).expect("animation children")),
        ListLength::Unknown,
    )
}
fn builtin(name: &'static str, parameters: Vec<Parameter>) -> Builtin {
    Builtin {
        name,
        parameters,
        result: Type::Animation,
        state_effect: false,
        timing: Default::default(),
        content: Default::default(),
        execute: Execution::Native(construct),
        check: crate::native_types::fixed,
    }
}
pub(crate) fn register(out: &mut Vec<Builtin>) {
    let p = |name, ty, required| Parameter { name, ty, required };
    let duration = Type::Numeric(Unit::Duration);
    let choice = |names: &[&str]| {
        Type::union(names.iter().map(|name| Type::StringLiteral((*name).into())))
            .expect("temporal choices")
    };
    out.push(builtin(
        "Animation",
        vec![
            p("children", children_type(), false),
            p(
                "target",
                Type::union([
                    Type::Element,
                    Type::List(Box::new(Type::Element), ListLength::Unknown),
                ])
                .expect("animation targets"),
                false,
            ),
            p(
                "duration",
                Type::union([duration.clone(), choice(&["auto", "match_parent"])])
                    .expect("time dimension"),
                false,
            ),
            p("flow", choice(&["parallel", "sequence"]), false),
            p(
                "justify",
                Type::union([
                    Justify::ty(),
                    choice(&["start", "end", "center", "between", "around", "evenly"]),
                ])
                .expect("temporal alignment"),
                false,
            ),
            p("gap", duration.clone(), false),
            p("padding", duration.clone(), false),
            p("padding_start", duration.clone(), false),
            p("padding_end", duration.clone(), false),
            p("margin", duration.clone(), false),
            p("margin_start", duration.clone(), false),
            p("margin_end", duration.clone(), false),
            p("offset", duration.clone(), false),
            p("position_mode", PositionMode::ty(), false),
            p("overflow", Overflow::ty(), false),
        ],
    ));
    out.push(builtin(
        "wait",
        vec![p(
            "duration",
            Type::union([duration.clone(), Type::Forever]).expect("hold duration"),
            true,
        )],
    ));
    out.push(builtin(
        "animate",
        vec![
            p("target", Type::Unknown, true),
            p("to", Type::Unknown, true),
            p("duration", duration, true),
            p("ease", Type::String, false),
        ],
    ));
    out.last_mut().expect("animate registration").execute = Execution::CellAnimation;
    out.extend(crate::animations::wrappers());
}
pub(crate) fn methods_for(receiver: &Type) -> impl Iterator<Item = &'static Builtin> {
    (*receiver == Type::Animation)
        .then(|| repeat_builtin())
        .into_iter()
}
fn repeat_builtin() -> &'static Builtin {
    static REPEAT: std::sync::OnceLock<Builtin> = std::sync::OnceLock::new();
    REPEAT.get_or_init(|| {
        builtin(
            "Animation.repeat",
            vec![
                Parameter {
                    name: "animation",
                    ty: Type::Animation,
                    required: true,
                },
                Parameter {
                    name: "count",
                    ty: Type::union([Type::Numeric(Unit::Number), Type::Forever])
                        .expect("repeat count"),
                    required: true,
                },
            ],
        )
    })
}
pub(crate) fn methods() -> impl Iterator<Item = &'static Builtin> {
    std::iter::once(repeat_builtin())
}
pub(crate) fn cell(cell: u64, to: Value, duration: Fraction, easing: Easing) -> Value {
    Value::Animation(Arc::new(plan::Animation::new(Node::Cell {
        cell,
        to,
        duration,
        easing,
    })))
}

fn children(
    args: &imbl::OrdMap<String, Value>,
) -> Result<imbl::Vector<Arc<plan::Animation>>, String> {
    let Some(value) = args.get("children") else {
        return Ok(Default::default());
    };
    let Value::List(values) = value else {
        return Err("Animation children must be a List of Animation or Element values".into());
    };
    values
        .iter()
        .map(|value| match value {
            Value::Animation(plan) => Ok(plan.clone()),
            Value::Element(element) => Ok(Arc::new(plan::Animation::new(Node::Element(
                element.clone(),
            )))),
            _ => Err(format!(
                "Animation child has type {}; expected Animation or Element",
                value.ty()
            )),
        })
        .collect()
}
pub(crate) fn wrapper(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let operation =
        crate::animations::wrapper_operation(name).ok_or("unknown animation wrapper")?;
    let definition = crate::animations::definition(operation).expect("canonical wrapper");
    let easing = args
        .get("ease")
        .map(Value::string)
        .transpose()?
        .map(Easing::parse)
        .transpose()?
        .unwrap_or(definition.builtin.timing.easing);
    let mut arguments = args.clone();
    arguments.remove("children");
    sized_effect(
        operation,
        arguments,
        definition.builtin.timing.default,
        easing,
        children(args)?,
    )
}

fn construct(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    _context: Context<'_>,
) -> Result<Value, String> {
    let node = match name {
        "wait" => Node::Hold(if matches!(args.get("duration"), Some(Value::Forever)) {
            None
        } else {
            Some(crate::timing::Timing::default().value(args.get("duration"))?)
        }),
        "Animation.repeat" => {
            let Some(Value::Animation(animation)) = args.get("animation") else {
                return Err("repeat requires an Animation".into());
            };
            let count = if matches!(args.get("count"), Some(Value::Forever)) {
                None
            } else {
                Some(
                    args.get("count")
                        .ok_or("repeat requires count")?
                        .number()?
                        .usize()?,
                )
            };
            if count == Some(0) {
                return Ok(Value::Animation(Arc::new(plan::Animation::new(
                    Node::Hold(Some(0.into())),
                ))));
            }
            if count == Some(1) {
                return Ok(Value::Animation(animation.clone()));
            }
            if animation.style == Style::default()
                && let Node::Repeat {
                    animation: base,
                    count: Some(inner),
                } = animation.node.as_ref()
            {
                let count = count
                    .map(|count| count.checked_mul(*inner).ok_or("repeat count overflow"))
                    .transpose()?;
                Node::Repeat {
                    animation: base.clone(),
                    count,
                }
            } else {
                Node::Repeat {
                    animation: animation.clone(),
                    count,
                }
            }
        }
        "animate" => return Err("animate target must name a mutable binding".into()),
        _ => {
            if let Some(target) = args.get("target") {
                let elements = match target {
                    Value::Element(element) => std::iter::once(element.clone()).collect(),
                    Value::List(items) => items
                        .iter()
                        .map(|item| match item {
                            Value::Element(element) => Ok(element.clone()),
                            _ => Err("Animation target list must contain only Elements"),
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err("Animation target must be an Element or List<Element>".into()),
                };
                Node::Scope {
                    elements,
                    children: children(args)?,
                }
            } else {
                Node::Container(children(args)?)
            }
        }
    };
    let mut animation = plan::Animation::new(node);
    if name == "Animation" {
        let time = |key, default| native::number(args, key, default);
        let text = |key, default: &str| {
            args.get(key)
                .map(Value::string)
                .transpose()
                .map(|v| v.unwrap_or(default).to_owned())
        };
        let padding = time("padding", 0.into())?;
        let margin = time("margin", 0.into())?;
        animation.style = Style {
            duration: dimension(args.get("duration"))?,
            flow: match text("flow", "parallel")?.as_str() {
                "parallel" => Flow::Parallel,
                "sequence" => Flow::Sequence,
                _ => return Err("Animation flow must be parallel or sequence".into()),
            },
            justify: match args.get("justify") {
                Some(Value::Variant { tag, .. }) => Justify::from_name(tag)?,
                Some(value) => match value.string()? {
                    "start" => Justify::Start,
                    "end" => Justify::End,
                    "center" => Justify::Center,
                    "between" => Justify::SpaceBetween,
                    "around" => Justify::SpaceAround,
                    "evenly" => Justify::SpaceEvenly,
                    _ => {
                        return Err(
                            "justify must be start, end, center, between, around or evenly".into(),
                        );
                    }
                },
                None => Justify::Start,
            },
            gap: time("gap", 0.into())?,
            padding_start: time("padding_start", padding)?,
            padding_end: time("padding_end", padding)?,
            margin_start: time("margin_start", margin)?,
            margin_end: time("margin_end", margin)?,
            offset: time("offset", 0.into())?,
            position_mode: PositionMode::parse(&text("position_mode", "relative")?)?,
            overflow: Overflow::parse(&text("overflow", "hidden")?)?,
        };
    }
    Ok(Value::Animation(Arc::new(animation)))
}

pub(crate) struct Lowered {
    pub animation: sanim_core::Animation,
    pub layout: Arc<[plan::Receipt]>,
}
pub(crate) fn measure(
    plan: &plan::Animation,
    available: Option<Fraction>,
    limits: &Limits,
) -> Result<(Fraction, bool, Arc<[plan::Receipt]>), plan::LayoutError> {
    let layout = plan.layout(
        available,
        limits.max_operations.try_into().unwrap_or(usize::MAX),
    )?;
    let indefinite = layout.duration.is_none();
    let finite = layout.finite_prefix;
    Ok((finite, indefinite, layout.receipts))
}
pub(crate) fn lower(
    plan: &plan::Animation,
    base: &sanim_core::Animation,
    available: Option<Fraction>,
    limits: &Limits,
) -> Result<Lowered, plan::LayoutError> {
    let layout = plan.layout(
        available,
        limits.max_operations.try_into().unwrap_or(usize::MAX),
    )?;
    let mut animation = base.clone();
    let mut budget = limits.data.max_bytes;
    let mut work = 0;
    let mut pending = Pending {
        remaining: limits.max_operations.try_into().unwrap_or(usize::MAX),
        ..Default::default()
    };
    collect(&layout.placements, 0, &Arc::from([]), &mut pending)?;
    for (id, mount) in pending.fallbacks {
        if !pending.referenced.contains(&id) {
            pending.mounts.entry(id).or_insert(mount);
        }
    }
    let mut descendants = std::collections::BTreeSet::new();
    for mount in pending.mounts.values() {
        if let Some(children) = mount.element.geometry.children() {
            for child in children {
                let mut ids = Vec::new();
                child.ids(&mut ids);
                descendants.extend(ids);
            }
        }
    }
    let mut roots: Vec<_> = pending
        .mounts
        .values()
        .filter(|mount| !descendants.contains(&mount.element.id))
        .cloned()
        .collect();
    for mount in &mut roots {
        let mut ids = Vec::new();
        mount.element.ids(&mut ids);
        for id in ids {
            if let Some(child) = pending.mounts.get(&id) {
                mount.birth = mount.birth.min(child.birth);
                if mount.cycle != child.cycle {
                    mount.cycle = None;
                }
            }
        }
    }
    roots.sort_by_key(|mount| mount.order);
    if roots.len() > limits.max_elements || pending.mounts.len() > limits.max_elements {
        return Err("Animation exceeds the element budget before mounting".into());
    }
    for root in roots {
        mount(&mut animation, &root.element, root.birth)?;
        animation
            .layers
            .back_mut()
            .expect("mounted visual root")
            .cycle = root.cycle;
    }
    pending.actions.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .expect("validated finite action starts")
    });
    for action in pending.actions {
        lower_action(&action, &mut animation, limits, &mut budget, &mut work)?;
    }

    animation.indefinite = layout.duration.is_none();
    animation.duration = layout.finite_prefix;
    if animation.element_count() > limits.max_elements {
        return Err("Animation exceeds the element budget".into());
    }
    Ok(Lowered {
        animation,
        layout: layout.receipts,
    })
}
fn mount(
    animation: &mut sanim_core::Animation,
    element: &Element,
    start: Fraction,
) -> Result<(), String> {
    if let Some(existing) = animation.find(&element.id) {
        if existing != element {
            return Err(format!(
                "element identity '{}' refers to different element definitions",
                element.id
            ));
        }
        Ok(())
    } else {
        animation.add(element.clone(), start)
    }
}
#[derive(Clone)]
struct Mount {
    element: Arc<Element>,
    birth: Fraction,
    cycle: Option<math::cycle::Cycle>,
    order: usize,
}
struct Action<'a> {
    placement: &'a plan::Placement,
    targets: Arc<[Arc<Element>]>,
    cycle: Option<math::cycle::Cycle>,
    start: Fraction,
    duration: Fraction,
    easing: Easing,
}
#[derive(Default)]
struct Pending<'a> {
    actions: Vec<Action<'a>>,
    mounts: std::collections::BTreeMap<String, Mount>,
    fallbacks: std::collections::BTreeMap<String, Mount>,
    referenced: std::collections::BTreeSet<String>,
    remaining: usize,
}
impl Pending<'_> {
    fn reference(
        &mut self,
        element: &Arc<Element>,
        placement: &plan::Placement,
        cycle: Option<math::cycle::Cycle>,
        order: usize,
        active: bool,
    ) -> Result<(), String> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or("Animation visual references exceed the lowering work budget")?;
        self.referenced.insert(element.id.clone());
        if !active {
            return Ok(());
        }
        let value = Mount {
            element: element.clone(),
            birth: placement
                .clip
                .map_or(placement.start, |(start, _)| placement.start.max(start)),
            cycle,
            order,
        };
        if let Some(old) = self.mounts.get_mut(&element.id) {
            if !Arc::ptr_eq(&old.element, element) && old.element != *element {
                return Err(format!(
                    "element identity '{}' refers to different definitions",
                    element.id
                ));
            }
            old.birth = old.birth.min(value.birth);
            old.order = old.order.min(order);
            if old.cycle != cycle {
                old.cycle = None;
            }
        } else {
            self.mounts.insert(element.id.clone(), value);
        }
        Ok(())
    }
}
fn compile_action<'a>(
    placement: &'a plan::Placement,
    targets: Arc<[Arc<Element>]>,
    cycle: Option<math::cycle::Cycle>,
) -> Result<Option<Action<'a>>, String> {
    let (duration, easing) = match placement.animation.node.as_ref() {
        Node::Effect { effect, .. } => (
            if matches!(
                placement.animation.style.duration,
                TimeDimension::MatchParent | TimeDimension::Fixed(_)
            ) {
                placement
                    .duration
                    .ok_or("effect duration must resolve to a finite temporal extent")?
            } else {
                effect.duration
            },
            effect.easing,
        ),
        Node::Cell {
            duration, easing, ..
        } => (*duration, *easing),
        _ => return Err("only animation actions can be lowered".into()),
    };
    let Some(interval) = (math::temporal::Window {
        cycle,
        clip: placement.clip,
    })
    .interval(placement.start, duration)?
    else {
        return Ok(None);
    };
    math::temporal::writer_end(placement.start, duration)?;
    Ok(Some(Action {
        placement,
        targets,
        cycle,
        start: interval.start,
        duration,
        easing,
    }))
}
fn collect<'a>(
    nodes: &'a [plan::Placement],
    index: usize,
    inherited: &Arc<[Arc<Element>]>,
    pending: &mut Pending<'a>,
) -> Result<(), String> {
    let placement = &nodes[index];
    let active = !placement
        .clip
        .is_some_and(|(a, b)| b < a || placement.start > b || a == b && placement.start != a);
    pending.remaining = pending
        .remaining
        .checked_sub(1)
        .ok_or("Animation actions exceed the lowering work budget")?;
    let mut scope = inherited.clone();
    let cycle = placement.clock.as_deref().copied();
    match placement.animation.node.as_ref() {
        Node::Scope { .. } => {
            scope = placement.targets.clone();
            if active {
                pending.remaining = pending
                    .remaining
                    .checked_sub(scope.len())
                    .ok_or("Animation scope targets exceed the lowering work budget")?;
                for element in scope.iter() {
                    pending
                        .fallbacks
                        .entry(element.id.clone())
                        .or_insert(Mount {
                            element: element.clone(),
                            birth: placement
                                .clip
                                .map_or(placement.start, |(start, _)| placement.start.max(start)),
                            cycle,
                            order: index,
                        });
                }
            }
        }
        Node::Element(element) => pending.reference(element, placement, cycle, index, active)?,
        Node::Effect { effect, .. } => {
            let unbound = crate::animations::definition(&effect.operation).is_some()
                && !effect.arguments.contains_key("element");
            if unbound {
                if !placement.targets.is_empty() {
                    scope = placement.targets.clone();
                }
                if scope.is_empty() {
                    return Err(format!(
                        "{} has no visual target; set Animation target={{element}} or nest an Element inside the effect",
                        effect.operation
                    ));
                }
            }
            let action = if active {
                compile_action(
                    placement,
                    if unbound {
                        scope.clone()
                    } else {
                        Arc::from([])
                    },
                    cycle,
                )?
            } else {
                None
            };
            let action_active = action.is_some();
            if unbound {
                for element in scope.iter() {
                    pending.reference(element, placement, cycle, index, action_active)?;
                }
            }
            for value in effect.arguments.values() {
                match value {
                    Value::Element(element) => {
                        pending.reference(element, placement, cycle, index, action_active)?
                    }
                    Value::List(items) => {
                        for item in items.iter() {
                            if let Value::Element(element) = item {
                                pending.reference(
                                    element,
                                    placement,
                                    cycle,
                                    index,
                                    action_active,
                                )?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(action) = action {
                pending.actions.push(action);
            }
        }
        Node::Cell { .. } if active => {
            if let Some(action) = compile_action(placement, Arc::from([]), cycle)? {
                pending.actions.push(action);
            }
        }

        _ => {}
    }
    let mut child = index + 1;
    while child < placement.end {
        collect(nodes, child, &scope, pending)?;
        child = nodes[child].end;
    }
    Ok(())
}
fn lower_action(
    action: &Action<'_>,
    animation: &mut sanim_core::Animation,
    limits: &Limits,
    budget: &mut usize,
    work: &mut u64,
) -> Result<(), String> {
    let placement = action.placement;
    let plan = &placement.animation;
    let duration = action.duration;
    let easing = action.easing;
    let mut cursor = placement.start;
    let mut list_edit = None;
    let mut created = Vec::new();
    let origin = plan.origin.as_ref();
    let context = Context {
        module: origin.map_or("<animation>", |origin| origin.location.source.as_ref()),
        identity: &[],
        span: origin.map_or(Span::default(), |origin| origin.location.span),
        animation,
        cursor: &mut cursor,
        limits,
        retained_bytes: budget,
        duration,
        easing,
        cycle: action.cycle,
        clip: placement.clip,
        list_edit: &mut list_edit,
        created: &mut created,
        native_work: work,
        remaining_work: limits.max_operations,
    };
    match plan.node.as_ref() {
        Node::Effect { effect, .. } => {
            let mut arguments = effect.arguments.clone();
            if !action.targets.is_empty() {
                arguments.insert(
                    "element".into(),
                    if action.targets.len() == 1 {
                        Value::Element(action.targets[0].clone())
                    } else {
                        Value::List(action.targets.iter().cloned().map(Value::Element).collect())
                    },
                );
            }
            if crate::animations::definition(&effect.operation).is_some() {
                crate::animations::execute_effect(&effect.operation, &arguments, context)?;
            } else if effect.operation.as_ref() == "Element.scroll_to" {
                crate::scrolling::execute_effect(&effect.operation, &arguments, context)?;
            } else {
                crate::code_view::execute_effect(&effect.operation, &arguments, context)?;
            }
        }
        Node::Cell { cell, to, .. } => write_cell(*cell, to.clone(), context)?,
        _ => unreachable!("validated action"),
    }
    Ok(())
}
