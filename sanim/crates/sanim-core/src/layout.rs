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
        if width <= Fraction::from(0) || height <= Fraction::from(0) {
            return Err("viewport dimensions must be positive".into());
        }
        let width = to_f64(width)?;
        let height = to_f64(height)?;
        let mut elements = self.sample(time)?;
        for element in &mut elements {
            resolve_with_lists(element, width, height, Some((&self.list_views, time)))?;
        }
        Ok(elements)
    }
}

pub fn resolve(element: &mut Element, parent_width: f64, parent_height: f64) -> Result<(), String> {
    resolve_with_lists(element, parent_width, parent_height, None)
}
fn resolve_with_lists(
    element: &mut Element,
    parent_width: f64,
    parent_height: f64,
    lists: Option<(&crate::list_views::ListViews, Fraction)>,
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
    // Alignment anchors the element's bounds to the corresponding parent edge.
    let (own_min, own_max) = if matches!(element.geometry.as_ref(), Geometry::Group { .. }) {
        (
            [0.0; 2],
            [
                calc(
                    "*",
                    element
                        .layout
                        .width
                        .map(|number| to_f64(number.value))
                        .transpose()?
                        .unwrap_or(parent_width),
                    to_f64(element.scale)?,
                )?,
                calc(
                    "*",
                    element
                        .layout
                        .height
                        .map(|number| to_f64(number.value))
                        .transpose()?
                        .unwrap_or(parent_height),
                    to_f64(element.scale)?,
                )?,
            ],
        )
    } else {
        let b =
            crate::verify::bounds_at(element, Vec2::zero(crate::Unit::Length), Fraction::from(0))?;
        (b.min.values, b.max.values)
    };
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
            calc("-", calc("*", parent, anchor)?, own)?,
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
            let padding = to_f64(element.layout.padding)?;
            for child in children.iter_mut() {
                resolve_dimensions(
                    child,
                    (width - 2. * padding).max(0.),
                    (height - 2. * padding).max(0.),
                )?;
            }
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
                Some(flow::positions(
                    children,
                    &element.layout,
                    width,
                    height,
                    direction,
                )?)
            } else {
                None
            };
            for (index, child) in children.iter_mut().enumerate() {
                let mut position = if let Some(animated) = &animated {
                    animated[index].position
                } else {
                    positions.as_ref().expect("static flow positions")[index]
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
                resolve_with_lists(child, width, height, lists)?;
            }
        } else {
            let padding = to_f64(element.layout.padding)?;
            for child in children.iter_mut() {
                resolve_with_lists(
                    child,
                    (width - 2. * padding).max(0.),
                    (height - 2. * padding).max(0.),
                    lists,
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
    Ok(())
}

pub fn extent(element: &Element) -> Result<(f64, f64), String> {
    let (w, h) = match element.geometry.as_ref() {
        Geometry::Circle { radius } => (
            calc("*", to_f64(*radius)?, 2.0)?,
            calc("*", to_f64(*radius)?, 2.0)?,
        ),
        Geometry::Rectangle { width, height, .. } => (to_f64(*width)?, to_f64(*height)?),
        Geometry::Group { .. } => (
            to_f64(
                element
                    .layout
                    .width
                    .ok_or("flow child Group requires width")?
                    .value,
            )?,
            to_f64(
                element
                    .layout
                    .height
                    .ok_or("flow child Group requires height")?
                    .value,
            )?,
        ),
        _ => {
            let bounds = crate::verify::bounds_at(
                element,
                Vec2::zero(crate::Unit::Length),
                element.rotation,
            )?;
            return Ok((
                calc("-", bounds.max.values[0], bounds.min.values[0])?,
                calc("-", bounds.max.values[1], bounds.min.values[1])?,
            ));
        }
    };
    let stroke = element
        .stroke
        .as_ref()
        .map(|stroke| to_f64(stroke.width))
        .transpose()?
        .unwrap_or(0.);
    Ok((
        calc("*", calc("+", w, stroke)?, to_f64(element.scale)?)?,
        calc("*", calc("+", h, stroke)?, to_f64(element.scale)?)?,
    ))
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
        let positions = flow::positions(&elements, style, width, height, direction)?;
        Ok::<_, String>(positions)
    };
    let from_positions = endpoint(selection.from)?;
    let to_positions = endpoint(selection.to)?;
    let from: std::collections::BTreeMap<_, _> = crate::list_views::elements(selection.from)?
        .zip(from_positions.iter())
        .map(|(element, position)| (element.id.as_str(), *position))
        .collect();
    let to: std::collections::BTreeMap<_, _> = crate::list_views::elements(selection.to)?
        .zip(to_positions.iter())
        .map(|(element, position)| (element.id.as_str(), *position))
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
                || number.value <= Fraction::from(0)
            {
                return Err(format!(
                    "{}.{} must resolve to a positive finite length inside its parent",
                    element.id,
                    if axis == 0 { "width" } else { "height" }
                ));
            }
            if let Geometry::Rectangle { width, height, .. } = element.geometry.as_ref() {
                let current = if axis == 0 { *width } else { *height };
                if current != number.value {
                    let Geometry::Rectangle { width, height, .. } =
                        Arc::make_mut(&mut element.geometry)
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
    Ok(())
}
