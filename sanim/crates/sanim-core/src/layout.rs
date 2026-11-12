//! Resolve parent-local points and percentages against an external logical viewport.
mod flow;
use crate::{
    Animation, Element, Fraction, Geometry, Vec2,
    math::{float_arithmetic as calc, to_f64},
};

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
            resolve(element, width, height)?;
        }
        Ok(elements)
    }
}

pub fn resolve(element: &mut Element, parent_width: f64, parent_height: f64) -> Result<(), String> {
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
    let (own_min, own_max) = if matches!(element.geometry, Geometry::Group { .. }) {
        (
            [0.0; 2],
            [
                calc(
                    "*",
                    element
                        .layout
                        .width
                        .map(to_f64)
                        .transpose()?
                        .unwrap_or(parent_width),
                    to_f64(element.scale)?,
                )?,
                calc(
                    "*",
                    element
                        .layout
                        .height
                        .map(to_f64)
                        .transpose()?
                        .unwrap_or(parent_height),
                    to_f64(element.scale)?,
                )?,
            ],
        )
    } else {
        let mut local = element.clone();
        local.position = Vec2::zero(crate::Unit::Length);
        local.rotation = Fraction::from(0);
        let b = crate::verify::bounds(&local)?;
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
    if let Geometry::Group { children } = &mut element.geometry {
        let width = element
            .layout
            .width
            .map(to_f64)
            .transpose()?
            .unwrap_or(parent_width);
        let height = element
            .layout
            .height
            .map(to_f64)
            .transpose()?
            .unwrap_or(parent_height);
        if let Some(direction) = element.layout.direction {
            let positions = flow::positions(children, &element.layout, width, height, direction)?;
            for (child, position) in children.iter_mut().zip(positions.iter()) {
                child.position = Vec2 {
                    values: *position,
                    unit: crate::Unit::Length,
                };
                child.layout.x_align = crate::Align::Start;
                child.layout.y_align = crate::Align::Start;
                resolve(child, width, height)?;
            }
        } else {
            let padding = to_f64(element.layout.padding)?;
            for child in children {
                resolve(
                    child,
                    (width - 2. * padding).max(0.),
                    (height - 2. * padding).max(0.),
                )?;
                child.position.values[0] = calc("+", child.position.values[0], padding)?;
                child.position.values[1] = calc("+", child.position.values[1], padding)?;
            }
        }
    }
    if !element.position.values[0].is_finite() || !element.position.values[1].is_finite() {
        return Err("layout arithmetic overflow".into());
    }
    Ok(())
}

pub fn extent(element: &Element) -> Result<(f64, f64), String> {
    let (w, h) = match &element.geometry {
        Geometry::Circle { radius } => (
            calc("*", to_f64(*radius)?, 2.0)?,
            calc("*", to_f64(*radius)?, 2.0)?,
        ),
        Geometry::Rectangle { width, height } => (to_f64(*width)?, to_f64(*height)?),
        Geometry::Group { .. } => (
            to_f64(
                element
                    .layout
                    .width
                    .ok_or("flow child Group requires width")?,
            )?,
            to_f64(
                element
                    .layout
                    .height
                    .ok_or("flow child Group requires height")?,
            )?,
        ),
        _ => {
            let mut local = element.clone();
            local.position = Vec2::zero(crate::Unit::Length);
            let bounds = crate::verify::bounds(&local)?;
            return Ok((
                calc("-", bounds.max.values[0], bounds.min.values[0])?,
                calc("-", bounds.max.values[1], bounds.min.values[1])?,
            ));
        }
    };
    Ok((
        calc("*", w, to_f64(element.scale)?)?,
        calc("*", h, to_f64(element.scale)?)?,
    ))
}
