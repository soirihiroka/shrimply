use super::*;
use crate::math::{self, temporal};

pub const MAX_PLAN_DEPTH: usize = 128;
pub const MAX_PLAN_NODES: usize = 100_000;

/// A single resolved occurrence; repeated references retain the original COW plan.
#[derive(Clone, Debug)]
pub struct Placement {
    pub animation: Arc<Animation>,
    /// Deduplicated visual references, structurally shared through single-child wrappers.
    pub targets: Arc<[Arc<Element>]>,
    pub start: Fraction,
    pub duration: Option<Fraction>,
    pub clip: Option<(Fraction, Fraction)>,
    /// One compiled period for a bounded or infinite repeat.
    pub cycle: Option<math::cycle::Cycle>,
    /// Inherited periodic context shared by compiled actions and source event indices.
    pub clock: Option<Arc<math::cycle::Cycle>>,
    /// Preorder subtree end, allowing one linear lowering traversal.
    pub end: usize,
}
#[derive(Clone, Debug)]
pub struct Layout {
    pub placements: Vec<Placement>,
    pub receipts: Arc<[Receipt]>,
    /// None is an explicitly infinite root, never an unknown finite duration.
    pub duration: Option<Fraction>,
    /// End of the bounded prepared prefix, shared by inspection and lowering.
    pub finite_prefix: Fraction,
}
#[derive(Clone, Copy)]
struct Size {
    main: Option<Fraction>,
    horizon: Option<Fraction>,
}
impl Size {
    fn finite(value: Fraction) -> Self {
        Self {
            main: Some(value),
            horizon: Some(value),
        }
    }
}
struct Measured {
    animation: Arc<Animation>,
    children: Vec<Measured>,
    intrinsic: Size,
    size: Size,
    extent: Option<Fraction>,
    targets: Arc<[Arc<Element>]>,
}
fn add(a: Option<Fraction>, b: Option<Fraction>) -> Result<Option<Fraction>, String> {
    a.zip(b).map(|(a, b)| math::timeline_end(a, b)).transpose()
}
fn maximum(a: Option<Fraction>, b: Option<Fraction>) -> Option<Fraction> {
    a.zip(b).map(|(a, b)| a.max(b))
}
impl Animation {
    pub fn new(node: Node) -> Self {
        let mut style = Style::default();
        if matches!(node, Node::Element(_)) {
            style.overflow = Overflow::Visible;
        }
        Self {
            node: Arc::new(node),
            style,
            origin: None,
        }
    }
    pub fn measure(&self) -> Result<Extent, LayoutError> {
        let mut budget = MAX_PLAN_NODES;
        Ok(measure(Arc::new(self.clone()), 0, &mut budget)?
            .extent
            .map_or(Extent::Infinite, Extent::Finite))
    }
    pub fn layout(
        &self,
        available: Option<Fraction>,
        max_nodes: usize,
    ) -> Result<Layout, LayoutError> {
        if let Some(available) = available {
            temporal::viewport(available)?;
        }
        let mut budget = max_nodes.min(MAX_PLAN_NODES);
        let measured = measure(Arc::new(self.clone()), 0, &mut budget)?;
        let duration = available.or(measured.extent);
        let clip = if extendable_source(&measured) {
            None
        } else {
            duration.map(|end| (Fraction::from(0), end))
        };
        let mut output = Output {
            placements: Vec::new(),
            receipts: Vec::new(),
            budget: max_nodes.min(MAX_PLAN_NODES),
            clock: None,
        };
        place(
            &measured,
            0.into(),
            available,
            true,
            available.is_some(),
            clip,
            &mut output,
        )?;
        let finite_prefix = if let Some(duration) = duration {
            duration
        } else {
            output
                .placements
                .iter()
                .try_fold(Fraction::from(0), |end, node| {
                    if node.clip.is_some_and(|(a, b)| b <= a || node.start >= b) {
                        return Ok(end);
                    }
                    let time = node.duration.map_or(Ok(node.start), |duration| {
                        temporal::end(node.start, duration)
                    })?;
                    let time = node
                        .cycle
                        .map_or(time, |clock| time.max(clock.until().unwrap_or(clock.end())));
                    let time = node.clip.map_or(time, |(_, clip_end)| time.min(clip_end));
                    Ok::<_, String>(end.max(time))
                })?
        };
        Ok(Layout {
            placements: output.placements,
            receipts: output.receipts.into(),
            duration,
            finite_prefix,
        })
    }
}
fn measure(
    animation: Arc<Animation>,
    depth: usize,
    budget: &mut usize,
) -> Result<Measured, LayoutError> {
    if depth > MAX_PLAN_DEPTH {
        return Err("Animation nesting exceeds the temporal layout depth budget".into());
    }
    *budget = budget
        .checked_sub(1)
        .ok_or("Animation exceeds its temporal layout node budget")?;
    let style = &animation.style;
    for (name, value) in [
        ("gap", style.gap),
        ("padding_start", style.padding_start),
        ("padding_end", style.padding_end),
        ("margin_start", style.margin_start),
        ("margin_end", style.margin_end),
    ] {
        temporal::nonnegative(name, value)?;
    }
    if !math::finite(&style.offset) {
        return Err("Animation offset must be finite".into());
    }
    let zero = Fraction::from(0);
    let mut children = Vec::new();
    let raw = match animation.node.as_ref() {
        Node::Hold(value) => {
            if let Some(value) = value {
                temporal::nonnegative("wait", *value)?;
            }
            Size {
                main: *value,
                horizon: *value,
            }
        }
        Node::Element(_) => Size::finite(zero),
        Node::Cell { duration, .. } => Size::finite(temporal::nonnegative("duration", *duration)?),
        Node::Container(items)
        | Node::Effect {
            children: items, ..
        }
        | Node::Scope {
            children: items, ..
        } => {
            let mut cursor = Some(zero);
            let mut horizon = Some(zero);
            let mut flow_count = 0;
            for child in items {
                let child = measure(child.clone(), depth + 1, budget)?;
                let margin = math::timeline_end(
                    child.animation.style.margin_start,
                    child.animation.style.margin_end,
                )?;
                let occupancy = add(child.size.main, Some(margin))?;
                let absolute = child.animation.style.position_mode == crate::PositionMode::Absolute;
                if !absolute && style.flow == Flow::Sequence && flow_count != 0 {
                    cursor = add(cursor, Some(style.gap))?;
                }
                let origin = if !absolute && style.flow == Flow::Sequence {
                    cursor
                } else {
                    Some(zero)
                };
                let shifted = add(origin, Some(child.animation.style.margin_start))?;
                // Relative offset affects the completion horizon, not normal flow occupancy.
                let shifted = shifted
                    .map(|v| math::arithmetic("+", v, child.animation.style.offset))
                    .transpose()?
                    .map(|v| v.max(zero));
                horizon = maximum(horizon, add(shifted, child.size.horizon)?);
                if !absolute {
                    if style.flow == Flow::Sequence {
                        cursor = add(cursor, occupancy)?;
                    } else {
                        cursor = maximum(cursor, occupancy);
                    }
                    flow_count += 1;
                }
                children.push(child);
            }
            if let Node::Effect { effect, .. } = animation.node.as_ref() {
                let own = Some(temporal::nonnegative("duration", effect.duration)?);
                cursor = maximum(cursor, own);
                horizon = maximum(horizon, own);
            }
            let padding = Some(math::timeline_end(style.padding_start, style.padding_end)?);
            Size {
                main: add(cursor, padding)?,
                horizon: add(maximum(horizon, cursor), padding)?,
            }
        }
        Node::Repeat {
            animation: child,
            count,
        } => {
            let child = measure(child.clone(), depth + 1, budget)?;
            let margins = math::timeline_end(
                child.animation.style.margin_start,
                child.animation.style.margin_end,
            )?;
            let occupied = add(child.size.main, Some(margins))?;
            let repeat_span = add(occupied, Some(style.gap))?;
            let padding = Some(math::timeline_end(style.padding_start, style.padding_end)?);
            let size = match count {
                Some(0) => Size {
                    main: padding,
                    horizon: padding,
                },
                Some(count) => {
                    if *count > 1 && occupied.is_none() {
                        return Err("finite repeat requires a finite cycle; an infinite child never reaches the next repetition".into());
                    }
                    let n = Fraction::from(*count as u64);
                    let between =
                        math::arithmetic("*", style.gap, Fraction::from((*count - 1) as u64))?;
                    let extent = add(
                        add(
                            occupied.map(|v| math::arithmetic("*", v, n)).transpose()?,
                            Some(between),
                        )?,
                        padding,
                    )?;
                    Size {
                        main: extent,
                        horizon: extent,
                    }
                }
                None => {
                    if repeat_span.is_none_or(|v| v <= zero)
                        || child.size.main != child.size.horizon
                        || child.animation.style.position_mode == crate::PositionMode::Absolute
                    {
                        return Err("repeat(forever) requires a positive finite relative cycle; put absolute on an enclosing Animation".into());
                    }
                    Size {
                        main: None,
                        horizon: None,
                    }
                }
            };
            children.push(child);
            size
        }
    };
    let mut size = match style.duration {
        TimeDimension::Fixed(v) => Size::finite(temporal::nonnegative("duration", v)?),
        TimeDimension::Infinite => Size {
            main: None,
            horizon: None,
        },
        _ => raw,
    };
    for value in [size.main, size.horizon].into_iter().flatten() {
        math::timeline_end(zero, value)?;
    }
    let extent = size.main;
    if style.overflow == Overflow::Hidden {
        size.horizon = extent;
    }
    if style.position_mode == crate::PositionMode::Absolute {
        size.main = Some(zero);
    }
    let targets: Arc<[Arc<Element>]> = match animation.node.as_ref() {
        Node::Element(element) => Arc::from([element.clone()]),
        Node::Scope { elements, .. } => {
            *budget = budget
                .checked_sub(elements.len())
                .ok_or("Animation scope targets exceed the temporal layout work budget")?;
            let mut unique = std::collections::BTreeMap::new();
            for element in elements {
                if let Some(old) = unique.insert(element.id.clone(), element.clone())
                    && !Arc::ptr_eq(&old, element)
                    && old != *element
                {
                    return Err(format!(
                        "Animation target identity '{}' refers to different elements",
                        element.id
                    )
                    .into());
                }
            }
            unique.into_values().collect::<Vec<_>>().into()
        }
        _ if children.len() == 1 => children[0].targets.clone(),
        _ => {
            let mut unique = std::collections::BTreeMap::new();
            for child in &children {
                for element in child.targets.iter() {
                    unique.insert(element.id.clone(), element.clone());
                }
            }
            unique.into_values().collect::<Vec<_>>().into()
        }
    };
    Ok(Measured {
        animation,
        children,
        intrinsic: raw,
        size,
        extent,
        targets,
    })
}
struct Output {
    placements: Vec<Placement>,
    receipts: Vec<Receipt>,
    budget: usize,
    clock: Option<Arc<math::cycle::Cycle>>,
}
struct Resolution {
    extent: Option<Fraction>,
    constrains_children: bool,
}
fn extendable_source(node: &Measured) -> bool {
    node.extent.is_none() || node.animation.style.duration == TimeDimension::MatchParent
}

fn resolve_extent(
    node: &Measured,
    parent: Option<Fraction>,
    root: bool,
    constrained: bool,
) -> Result<Resolution, LayoutError> {
    let style = &node.animation.style;
    let (extent, constrains_children) = match style.duration {
        TimeDimension::Fixed(value) => (Some(value), true),
        TimeDimension::Infinite => (None, false),
        TimeDimension::MatchParent => {
            if root && parent.is_none() {
                return Err(LayoutError::MissingParent);
            }
            let parent = parent
                .filter(|_| constrained)
                .ok_or(LayoutError::CyclicParent)?;
            (
                Some(
                    math::arithmetic(
                        "-",
                        parent,
                        math::timeline_end(style.margin_start, style.margin_end)?,
                    )?
                    .max(0.into()),
                ),
                true,
            )
        }
        TimeDimension::Auto
            if root
                && parent.is_some()
                && matches!(
                    node.animation.node.as_ref(),
                    Node::Container(_) | Node::Scope { .. }
                ) =>
        {
            (parent, true)
        }
        TimeDimension::Auto => (node.extent, false),
    };
    Ok(Resolution {
        extent,
        constrains_children,
    })
}

fn place(
    node: &Measured,
    origin: Fraction,
    parent: Option<Fraction>,
    root: bool,
    constrained: bool,
    inherited_clip: Option<(Fraction, Fraction)>,
    output: &mut Output,
) -> Result<(), LayoutError> {
    output.budget = output
        .budget
        .checked_sub(1)
        .ok_or("Animation exceeds its resolved temporal layout node budget")?;
    let style = &node.animation.style;
    let zero = Fraction::from(0);
    let resolution = resolve_extent(node, parent, root, constrained)?;
    let duration = resolution.extent;
    let children_constrained = resolution.constrains_children;
    let start = math::arithmetic("+", origin, style.offset)?;
    if start < zero {
        return Err("Animation offset places work before the beginning of the timeline".into());
    }
    // External presentation crops do not truncate an extendable source's native state.
    // Explicit descendant boxes keep their own clipping windows.
    let extendable_root = root && extendable_source(node);
    let own_clip = if style.overflow == Overflow::Hidden && !extendable_root {
        duration
            .map(|v| temporal::end(start, v).map(|end| (start, end)))
            .transpose()?
    } else {
        None
    };
    let clip = match (inherited_clip, own_clip) {
        (Some((a, b)), Some((c, d))) => Some((a.max(c), b.min(d).max(a.max(c)))),
        (a, b) => a.or(b),
    };
    let previous_clock = output.clock.clone();
    let index = output.placements.len();
    output.placements.push(Placement {
        animation: node.animation.clone(),
        targets: node.targets.clone(),
        start,
        duration,
        clip,
        cycle: None,
        clock: output.clock.clone(),
        end: 0,
    });
    output.receipts.push(Receipt {
        origin: node.animation.origin.clone(),
        intrinsic: node.intrinsic.main.map_or(Extent::Infinite, Extent::Finite),
        requested: style.duration,
        resolved: duration.map_or(Extent::Infinite, Extent::Finite),
        start,
        parent,
        clip,
        clock: output.clock.clone(),
    });
    if node.children.is_empty()
        || matches!(
            node.animation.node.as_ref(),
            Node::Repeat { count: Some(0), .. }
        )
    {
        output.placements[index].end = index + 1;
        output.clock = previous_clock;
        return Ok(());
    }
    let padding = temporal::end(style.padding_start, style.padding_end)?;
    let content = duration
        .map(|v| math::arithmetic("-", v, padding).map(|v| v.max(zero)))
        .transpose()?;
    let repeat = if let Node::Repeat {
        count: Some(count), ..
    } = node.animation.node.as_ref()
    {
        *count
    } else {
        1
    };
    let repeating = matches!(node.animation.node.as_ref(), Node::Repeat { .. });
    let mut occupied = zero;
    let mut infinite_occupancy = false;
    let flow_children = node
        .children
        .iter()
        .filter(|child| child.animation.style.position_mode != crate::PositionMode::Absolute);
    let flow_count = flow_children.clone().count();
    for (i, child) in flow_children.enumerate() {
        let child_size = resolve_extent(child, content, false, children_constrained)?.extent;
        if let Some(v) = child_size {
            let v = temporal::end(
                v,
                temporal::end(
                    child.animation.style.margin_start,
                    child.animation.style.margin_end,
                )?,
            )?;
            occupied = if style.flow == Flow::Sequence {
                if i > 0 {
                    occupied = temporal::end(occupied, style.gap)?;
                }
                temporal::end(occupied, v)?
            } else {
                occupied.max(v)
            };
        } else {
            infinite_occupancy = true;
        }
    }
    let single_occupied = occupied;
    if repeating && repeat > 1 {
        occupied = temporal::end(
            math::arithmetic("*", occupied, Fraction::from(repeat as u64))?,
            math::arithmetic("*", style.gap, Fraction::from((repeat - 1) as u64))?,
        )?;
    }
    let (lead, extra) = if let Some(content) = content.filter(|_| !infinite_occupancy) {
        temporal::alignment(
            style.justify,
            content,
            occupied,
            if repeating { repeat } else { flow_count },
        )?
    } else {
        (zero, zero)
    };
    let mut cursor = temporal::end(start, temporal::end(style.padding_start, lead)?)?;
    let cycle = if repeating {
        let spacing = temporal::end(style.gap, extra)?;
        let period = temporal::end(single_occupied, spacing)?;
        if period <= zero {
            return Err("repeat requires a positive finite cycle; add wait(...) or use the original instantaneous animation once".into());
        }
        if previous_clock.is_some() {
            return Err("nested repeat clocks require flattening; combine repeat counts or move the inner repeat outside the repeated animation".into());
        }
        let clock = math::cycle::Cycle::new(cursor, temporal::end(cursor, period)?)?;
        let until = if let Node::Repeat {
            count: Some(count), ..
        } = node.animation.node.as_ref()
        {
            Some(temporal::end(
                cursor,
                math::arithmetic(
                    "-",
                    math::arithmetic("*", period, Fraction::from(*count as u64))?,
                    spacing,
                )?,
            )?)
        } else {
            None
        };
        let until = match (until, clip) {
            (Some(until), Some((_, end))) => Some(until.min(end)),
            (until, None) => until,
            (None, Some((_, end))) => Some(end),
        };
        if clip.is_some_and(|(begin, end)| begin > clock.start() && end > begin) {
            return Err("repeat begins before its parent time window; move the repeat inside the window or remove its negative offset".into());
        }
        let active = until.is_none_or(|end| end >= clock.start());
        let bounded = until
            .filter(|_| active)
            .map(|end| clock.limited(end))
            .transpose()?
            .unwrap_or(clock);
        output.clock = active.then(|| Arc::new(bounded));
        output.placements[index].cycle = Some(bounded);
        output.placements[index].clock = output.clock.clone();
        output.receipts[index].clock = output.clock.clone();
        Some(clock)
    } else {
        None
    };
    {
        let mut flow_index = 0;
        for (child_index, child) in node.children.iter().enumerate() {
            let absolute = child.animation.style.position_mode == crate::PositionMode::Absolute;
            let at = if absolute {
                temporal::end(
                    start,
                    temporal::end(style.padding_start, child.animation.style.margin_start)?,
                )?
            } else if style.flow == Flow::Parallel
                && !matches!(node.animation.node.as_ref(), Node::Repeat { .. })
            {
                let occupied = resolve_extent(child, content, false, children_constrained)?
                    .extent
                    .map(|v| {
                        temporal::end(
                            v,
                            temporal::end(
                                child.animation.style.margin_start,
                                child.animation.style.margin_end,
                            )?,
                        )
                    })
                    .transpose()?;
                let shift = match (style.justify, content, occupied) {
                    (Justify::End | Justify::Center, Some(content), Some(occupied)) => {
                        temporal::alignment(style.justify, content, occupied, 1)?.0
                    }
                    _ => math::arithmetic(
                        "+",
                        lead,
                        math::arithmetic("*", extra, Fraction::from(flow_index as u64))?,
                    )?,
                };
                temporal::end(
                    start,
                    temporal::end(
                        style.padding_start,
                        temporal::end(shift, child.animation.style.margin_start)?,
                    )?,
                )?
            } else {
                temporal::end(cursor, child.animation.style.margin_start)?
            };
            place(
                child,
                at,
                content,
                false,
                children_constrained,
                if let Some(clock) = cycle.filter(|_| style.overflow == Overflow::Hidden) {
                    let (a, b) = clip.unwrap_or((clock.start(), clock.end()));
                    Some((
                        a.max(clock.start()),
                        b.min(clock.end()).max(a.max(clock.start())),
                    ))
                } else {
                    clip
                },
                output,
            )?;
            if !absolute
                && (style.flow == Flow::Sequence
                    || matches!(node.animation.node.as_ref(), Node::Repeat { .. }))
            {
                let Some(length) =
                    resolve_extent(child, content, false, children_constrained)?.extent
                else {
                    if !node.children.iter().skip(child_index + 1).any(|next| {
                        next.animation.style.position_mode != crate::PositionMode::Absolute
                    }) {
                        continue;
                    }
                    return Err("an infinite animation must be the last blocking item; use absolute positioning to place it outside temporal flow".into());
                };
                cursor = temporal::end(
                    cursor,
                    temporal::end(
                        length,
                        temporal::end(
                            child.animation.style.margin_start,
                            child.animation.style.margin_end,
                        )?,
                    )?,
                )?;
                if flow_index + 1 < flow_count {
                    cursor = temporal::end(cursor, temporal::end(style.gap, extra)?)?;
                }
            }
            if !absolute {
                flow_index += 1;
            }
        }
    }
    output.placements[index].end = output.placements.len();
    output.clock = previous_clock;
    Ok(())
}
