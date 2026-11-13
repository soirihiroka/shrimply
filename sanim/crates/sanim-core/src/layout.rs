//! Resolve parent-local points and percentages against an external logical viewport.
mod flow;
use crate::{
    Animation, Element, Fraction, Geometry, Vec2,
    math::{float_arithmetic as calc, to_f64},
};
use std::sync::Arc;

impl Animation {
    pub fn layout(
        &self,
        time: Fraction,
        width: Fraction,
        height: Fraction,
    ) -> Result<Vec<Element>, String> {
        self.layout_with(time, width, height, |_, _| Ok(()))
    }

    /// Resolve host-native content after sampling, before its bounds enter flow layout.
    pub fn layout_with(
        &self,
        time: Fraction,
        width: Fraction,
        height: Fraction,
        mut native: impl FnMut(&mut Element, Fraction) -> Result<(), String>,
    ) -> Result<Vec<Element>, String> {
        if width <= Fraction::from(0) || height <= Fraction::from(0) {
            return Err("viewport dimensions must be positive".into());
        }
        let width = to_f64(width)?;
        let height = to_f64(height)?;
        let mut elements = self.sample_with(time, &mut native)?;
        for element in &mut elements {
            let mut measured = Measurement::default();
            measure_boxes(element, width, Some(height), &mut measured)?;
            resolve_with_lists(
                element,
                width,
                height,
                Some((&self.list_views, time)),
                &measured,
            )?;
        }
        Ok(elements)
    }
}

pub fn resolve(element: &mut Element, parent_width: f64, parent_height: f64) -> Result<(), String> {
    let mut measured = Measurement::default();
    measure_boxes(element, parent_width, Some(parent_height), &mut measured)?;
    resolve_with_lists(element, parent_width, parent_height, None, &measured)
}
#[derive(Default)]
struct Measurement {
    // Only boxes, percentages and flex items need their original constraint
    // recipe retained during this frame's bottom-up measure pass.
    inputs: std::collections::BTreeMap<String, [Option<crate::Number>; 2]>,
    plans: std::collections::BTreeMap<String, Arc<flow::Plan>>,
}
impl Measurement {
    fn remember(&mut self, element: &Element) {
        self.inputs
            .entry(element.id.clone())
            .or_insert([element.layout.width, element.layout.height]);
    }
    fn restore(&self, element: &mut Element) {
        if let Some([width, height]) = self.inputs.get(&element.id) {
            element.layout.width = *width;
            element.layout.height = *height;
        }
        if element.geometry.children().is_some()
            && let Some(children) = Arc::make_mut(&mut element.geometry).children_mut()
        {
            for child in children {
                self.restore(child);
            }
        }
    }
}

/// Automatic block heights are measured bottom-up. Changed flex constraints
/// remeasure only the affected subtree, using original percentages rather than
/// treating an earlier resolved point size as a new source value.
fn measure_boxes(
    element: &mut Element,
    parent_width: f64,
    parent_height: Option<f64>,
    measured: &mut Measurement,
) -> Result<(), String> {
    let is_box = matches!(element.geometry.as_ref(), Geometry::Box { .. });
    if is_box
        || [element.layout.width, element.layout.height]
            .into_iter()
            .flatten()
            .any(|value| value.unit == crate::Unit::Percent)
    {
        measured.remember(element);
    }
    if parent_height.is_none()
        && element
            .layout
            .height
            .is_some_and(|height| height.unit == crate::Unit::Percent)
    {
        return Err(format!(
            "{}.height is a percentage inside an automatic-height Box; set the parent's height to break the layout cycle",
            element.id
        ));
    }
    resolve_dimensions(element, parent_width, parent_height.unwrap_or(0.))?;
    let padding = to_f64(element.layout.padding)?;
    let physical = match element.geometry.as_ref() {
        Geometry::Rectangle { width, height, .. } | Geometry::Media { width, height, .. } => {
            Some([to_f64(*width)?, to_f64(*height)?])
        }
        _ => None,
    };
    let width = element
        .layout
        .width
        .map(|width| to_f64(width.value))
        .transpose()?
        .or(physical.map(|size| size[0]))
        .unwrap_or(parent_width);
    let height = element
        .layout
        .height
        .map(|height| to_f64(height.value))
        .transpose()?
        .or(physical.map(|size| size[1]))
        .or(if is_box { None } else { parent_height });
    let available_width = (width - 2. * padding).max(0.);
    let available_height = height.map(|height| (height - 2. * padding).max(0.));
    if element.geometry.children().is_some()
        && let Some(children) = Arc::make_mut(&mut element.geometry).children_mut()
    {
        for child in children.iter_mut() {
            if element.layout.flex {
                measured.remember(child);
            }
            measure_boxes(child, available_width, available_height, measured)?;
        }
        if is_box && let Some(direction) = element.layout.direction {
            let mut plan = flow::positions(children, &element.layout, width, height, direction)?;
            if element.layout.flex {
                let main_axis = usize::from(direction == crate::Direction::Column);
                let mut changed = false;
                for (child, item) in children.iter_mut().zip(plan.items.iter()) {
                    let natural = extent(child)?;
                    if [natural.0, natural.1].iter().zip(item.size).any(|(a, b)| {
                        (a - b).abs() > f64::from(f32::EPSILON) * a.abs().max(b.abs()).max(1.)
                    }) {
                        measured.restore(child);
                        let inputs = measured
                            .inputs
                            .get(&child.id)
                            .copied()
                            .unwrap_or([child.layout.width, child.layout.height]);
                        for (axis, field) in [&mut child.layout.width, &mut child.layout.height]
                            .into_iter()
                            .enumerate()
                        {
                            if axis == main_axis || inputs[axis].is_some() {
                                *field = Some(crate::Number {
                                    value: crate::math::from_f64(item.size[axis])?,
                                    unit: crate::Unit::Length,
                                });
                            }
                        }
                        measure_boxes(child, available_width, available_height, measured)?;
                        changed = true;
                    }
                }
                if changed {
                    plan = flow::positions(children, &element.layout, width, height, direction)?;
                }
            }
            element.layout.width = Some(crate::Number {
                value: crate::math::from_f64(plan.size[0])?,
                unit: crate::Unit::Length,
            });
            element.layout.height = Some(crate::Number {
                value: crate::math::from_f64(plan.size[1])?,
                unit: crate::Unit::Length,
            });
            measured.plans.insert(element.id.clone(), plan);
        } else if is_box && (element.layout.width.is_none() || element.layout.height.is_none()) {
            let mut low = [0.0f64; 2];
            let mut high = [0.0f64; 2];
            for child in children.iter() {
                let bounds = crate::verify::bounds(child)?;
                for axis in 0..2 {
                    low[axis] = low[axis].min(bounds.min.values[axis]);
                    high[axis] = high[axis].max(bounds.max.values[axis]);
                }
            }
            for (axis, field) in [&mut element.layout.width, &mut element.layout.height]
                .into_iter()
                .enumerate()
            {
                if field.is_none() {
                    *field = Some(crate::Number {
                        value: crate::math::from_f64(high[axis] - low[axis] + 2. * padding)?,
                        unit: crate::Unit::Length,
                    });
                }
            }
        }
    }
    Ok(())
}

fn resolve_with_lists(
    element: &mut Element,
    parent_width: f64,
    parent_height: f64,
    lists: Option<(&crate::list_views::ListViews, Fraction)>,
    measured: &Measurement,
) -> Result<(), String> {
    resolve_dimensions(element, parent_width, parent_height)?;
    if element.horizontal.origin.unit == crate::Unit::Percent {
        let width = element
            .layout
            .width
            .map(|number| to_f64(number.value))
            .transpose()?
            .unwrap_or(parent_width);
        element.horizontal.origin = crate::Number {
            value: crate::math::from_f64(calc(
                "+",
                calc("*", to_f64(element.horizontal.origin.value)?, width)?,
                to_f64(element.horizontal.offset)?,
            )?)?,
            unit: crate::Unit::Length,
        };
        element.horizontal.offset = Fraction::from(0);
    }
    if element.position.unit == crate::Unit::Percent {
        element.position = Vec2 {
            values: [
                calc("*", element.position.values[0], parent_width)?,
                calc("*", element.position.values[1], parent_height)?,
            ],
            unit: crate::Unit::Length,
        };
    }
    // The layout box can exceed intrinsic paint bounds without stretching a
    // leaf shape. Padding belongs inside containers and outside intrinsic leaves.
    let (own_min, own_max) = frame_bounds(element, Some([parent_width, parent_height]))?;
    for (axis, parent) in [parent_width, parent_height].into_iter().enumerate() {
        let anchor = if axis == 0 {
            element.layout.x_align
        } else {
            element.layout.y_align
        }
        .fraction();
        let anchor = to_f64(anchor)?;
        let own = calc(
            "+",
            own_min[axis],
            calc("*", calc("-", own_max[axis], own_min[axis])?, anchor)?,
        )?;
        element.position.values[axis] = calc(
            "+",
            element.position.values[axis],
            calc(
                "+",
                calc("-", calc("*", parent, anchor)?, own)?,
                element.layout.position_offset[axis],
            )?,
        )?;
    }
    let rectangle = match element.geometry.as_ref() {
        Geometry::Rectangle { width, height, .. } => Some((to_f64(*width)?, to_f64(*height)?)),
        _ => None,
    };
    if element
        .geometry
        .children()
        .is_some_and(|children| !children.is_empty())
        && let Some(children) = Arc::make_mut(&mut element.geometry).children_mut()
    {
        let width = rectangle
            .map(|size| size.0)
            .or(element
                .layout
                .width
                .map(|number| to_f64(number.value))
                .transpose()?)
            .unwrap_or(parent_width);
        let height = rectangle
            .map(|size| size.1)
            .or(element
                .layout
                .height
                .map(|number| to_f64(number.value))
                .transpose()?)
            .unwrap_or(parent_height);
        if let Some(direction) = element.layout.direction {
            let transition = lists
                .map(|(lists, time)| lists.selection(&element.id, time))
                .transpose()?
                .flatten();
            let animated =
                if let Some(selection) = transition.as_ref().filter(|selection| selection.active) {
                    Some(list_positions(
                        children,
                        &element.layout,
                        width,
                        height,
                        direction,
                        selection,
                    )?)
                } else {
                    None
                };
            let positions = if animated.is_none() {
                Some(if let Some(plan) = measured.plans.get(&element.id) {
                    plan.clone()
                } else {
                    flow::positions(children, &element.layout, width, Some(height), direction)?
                })
            } else {
                None
            };
            for (index, child) in children.iter_mut().enumerate() {
                if element.layout.flex
                    && let Some(plan) = &positions
                {
                    let size = plan.items[index].size;
                    child.layout.width = Some(crate::Number {
                        value: crate::math::from_f64(size[0])?,
                        unit: crate::Unit::Length,
                    });
                    child.layout.height = Some(crate::Number {
                        value: crate::math::from_f64(size[1])?,
                        unit: crate::Unit::Length,
                    });
                }
                let mut position = if let Some(animated) = &animated {
                    animated[index].position
                } else {
                    positions.as_ref().expect("static flow positions").items[index].position
                };
                if let Some(animated) = &animated {
                    let visibility = animated[index].opacity;
                    child.opacity = crate::math::visual_product(child.opacity, visibility)?;
                    let scale = animated[index].scale;
                    if scale != Fraction::from(1) {
                        let before = extent(child)?;
                        child.scale = crate::math::visual_product(child.scale, scale)?;
                        let after = extent(child)?;
                        position[0] = calc(
                            "+",
                            position[0],
                            calc("/", calc("-", before.0, after.0)?, 2.)?,
                        )?;
                        position[1] = calc(
                            "+",
                            position[1],
                            calc("/", calc("-", before.1, after.1)?, 2.)?,
                        )?;
                    }
                }
                child.position = Vec2 {
                    values: position,
                    unit: crate::Unit::Length,
                };
                child.layout.x_align = crate::Align::Start;
                child.layout.y_align = crate::Align::Start;
                resolve_with_lists(child, width, height, lists, measured)?;
            }
        } else {
            let padding = to_f64(element.layout.padding)?;
            for child in children.iter_mut() {
                resolve_with_lists(
                    child,
                    (width - 2. * padding).max(0.),
                    (height - 2. * padding).max(0.),
                    lists,
                    measured,
                )?;
                child.position.values[0] = calc("+", child.position.values[0], padding)?;
                child.position.values[1] = calc("+", child.position.values[1], padding)?;
            }
        }
        if rectangle.is_some() {
            for child in children {
                child.position.values[0] = calc("-", child.position.values[0], width / 2.)?;
                child.position.values[1] = calc("-", child.position.values[1], height / 2.)?;
            }
        }
    }
    if !element.position.values[0].is_finite() || !element.position.values[1].is_finite() {
        return Err("layout arithmetic overflow".into());
    }
    crate::scrolling::resolve(element)?;
    Ok(())
}

fn frame_bounds(
    element: &Element,
    parent: Option<[f64; 2]>,
) -> Result<([f64; 2], [f64; 2]), String> {
    let scale = to_f64(element.scale)?;
    if matches!(
        element.geometry.as_ref(),
        Geometry::Code { .. } | Geometry::Box { .. }
    ) {
        let mut size = [0.; 2];
        for (axis, dimension) in [element.layout.width, element.layout.height]
            .into_iter()
            .enumerate()
        {
            size[axis] = dimension
                .map(|number| to_f64(number.value))
                .transpose()?
                .or_else(|| parent.map(|parent| parent[axis]))
                .ok_or("flow child Box requires resolved dimensions")?
                * scale;
        }
        return Ok(([0.; 2], size));
    }
    let rotation = if parent.is_none()
        && !matches!(
            element.geometry.as_ref(),
            Geometry::Circle { .. } | Geometry::Rectangle { .. } | Geometry::Media { .. }
        ) {
        element.rotation
    } else {
        Fraction::from(0)
    };
    let bounds = crate::verify::bounds_at(element, Vec2::zero(crate::Unit::Length), rotation)?;
    let mut low = bounds.min.values;
    let mut high = bounds.max.values;
    let padding = if element.geometry.children().is_none() {
        to_f64(element.layout.padding)? * scale
    } else {
        0.
    };
    let dimensions = if matches!(
        element.geometry.as_ref(),
        Geometry::Rectangle { .. } | Geometry::Media { .. }
    ) {
        [None, None]
    } else {
        [element.layout.width, element.layout.height]
    };
    for (axis, dimension) in dimensions.into_iter().enumerate() {
        low[axis] = calc("-", low[axis], padding)?;
        high[axis] = if let Some(dimension) = dimension {
            calc("+", low[axis], calc("*", to_f64(dimension.value)?, scale)?)?
        } else {
            calc("+", high[axis], padding)?
        };
    }
    Ok((low, high))
}

pub fn extent(element: &Element) -> Result<(f64, f64), String> {
    let (low, high) = frame_bounds(element, None)?;
    Ok((calc("-", high[0], low[0])?, calc("-", high[1], low[1])?))
}

/// Resolve both endpoint flows through the same Taffy cache. Removed children are
/// overlays, so they never occupy space in the destination flow.
struct ListPlacement {
    position: [f64; 2],
    opacity: Fraction,
    scale: Fraction,
}

fn list_positions(
    children: &imbl::Vector<Element>,
    style: &crate::Layout,
    width: f64,
    height: f64,
    direction: crate::Direction,
    selection: &crate::list_views::Selection<'_>,
) -> Result<Vec<ListPlacement>, String> {
    // Entry/exit should be a subtle emphasis, not shrinking the item to a dot.
    const LIST_SCALE_PERCENT: u64 = 92;
    let minimum_scale = Fraction::new(LIST_SCALE_PERCENT, 100u64);
    let by_id: std::collections::BTreeMap<_, _> = children
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect();
    let endpoint = |items: &crate::List| {
        let elements: imbl::Vector<_> = crate::list_views::elements(items)?
            .map(|element| {
                by_id
                    .get(element.id.as_str())
                    .copied()
                    .unwrap_or(element)
                    .clone()
            })
            .collect();
        let positions = flow::positions(&elements, style, width, Some(height), direction)?;
        Ok::<_, String>(positions)
    };
    let from_positions = endpoint(selection.from)?;
    let to_positions = endpoint(selection.to)?;
    let from: std::collections::BTreeMap<_, _> = crate::list_views::elements(selection.from)?
        .zip(from_positions.items.iter())
        .map(|(element, item)| (element.id.as_str(), item.position))
        .collect();
    let to: std::collections::BTreeMap<_, _> = crate::list_views::elements(selection.to)?
        .zip(to_positions.items.iter())
        .map(|(element, item)| (element.id.as_str(), item.position))
        .collect();
    children
        .iter()
        .map(|child| {
            let a = from.get(child.id.as_str());
            let b = to.get(child.id.as_str());
            let leaving = a.is_some() && b.is_none();
            let entering = a.is_none() && b.is_some();
            let visibility = if leaving {
                crate::math::arithmetic("-", 1.into(), selection.appearance)?
            } else if entering {
                selection.appearance
            } else {
                1.into()
            };
            let a = a
                .or(b)
                .ok_or("list transition is missing a child identity")?;
            let b = b.unwrap_or(a);
            let scale = if leaving {
                crate::math::visual_lerp(minimum_scale, 1.into(), visibility)?
            } else if entering {
                crate::math::visual_lerp(minimum_scale, 1.into(), selection.progress.max(0.into()))?
            } else {
                1.into()
            };
            Ok(ListPlacement {
                position: [
                    crate::math::motion_lerp(a[0], b[0], selection.progress, true)?,
                    crate::math::motion_lerp(a[1], b[1], selection.progress, true)?,
                ],
                opacity: visibility,
                scale,
            })
        })
        .collect()
}

fn resolve_dimensions(
    element: &mut Element,
    parent_width: f64,
    parent_height: f64,
) -> Result<(), String> {
    for (axis, parent) in [parent_width, parent_height].into_iter().enumerate() {
        let number = if axis == 0 {
            &mut element.layout.width
        } else {
            &mut element.layout.height
        };
        if let Some(number) = number {
            if number.unit == crate::Unit::Percent {
                number.value = crate::math::from_f64(calc("*", to_f64(number.value)?, parent)?)?;
                number.unit = crate::Unit::Length;
            }
            if number.unit != crate::Unit::Length
                || !crate::math::finite(&number.value)
                || number.value < Fraction::from(0)
            {
                return Err(format!(
                    "{}.{} must resolve to a positive finite length inside its parent",
                    element.id,
                    if axis == 0 { "width" } else { "height" }
                ));
            }
            if let Geometry::Rectangle { width, height, .. }
            | Geometry::Media { width, height, .. } = element.geometry.as_ref()
            {
                let current = if axis == 0 { *width } else { *height };
                if current != number.value {
                    let (Geometry::Rectangle { width, height, .. }
                    | Geometry::Media { width, height, .. }) = Arc::make_mut(&mut element.geometry)
                    else {
                        unreachable!()
                    };
                    if axis == 0 {
                        *width = number.value
                    } else {
                        *height = number.value
                    }
                }
            }
        }
    }
    if let Geometry::Media {
        asset,
        width,
        height,
    } = element.geometry.as_ref()
    {
        let wanted = if element.layout.width.is_some() && element.layout.height.is_none() {
            let [width,height]=crate::math::media::aspect_size([asset.width,asset.height],0,*width)?;
            Some((width,height))
        } else if element.layout.height.is_some() && element.layout.width.is_none() {
            let [width,height]=crate::math::media::aspect_size([asset.width,asset.height],1,*height)?;
            Some((width,height))
        } else {None};
        if let Some((wanted_width, wanted_height)) = wanted
            && (*width != wanted_width || *height != wanted_height)
        {
            let Geometry::Media { width, height, .. } = Arc::make_mut(&mut element.geometry) else {
                unreachable!()
            };
            *width = wanted_width;
            *height = wanted_height;
        }
    }
    Ok(())
}
