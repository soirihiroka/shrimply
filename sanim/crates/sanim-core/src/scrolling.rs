//! Native scroll viewports share the ordinary layout tree and drawing transforms.
use crate::{Element, Geometry, Unit, Vec2, math, verify};

pub fn validate(element: &Element, offset: Vec2) -> Result<(), String> {
    if element.geometry.children().is_none() {
        return Err(format!(
            "{} cannot scroll; use a container or CodeView",
            element.id
        ));
    }
    if !matches!(offset.unit, Unit::Length | Unit::Percent)
        || offset.values.iter().any(|value| !value.is_finite())
    {
        return Err(
            "scroll requires a finite Vec2 of pt offsets or percentages of the scroll range".into(),
        );
    }
    for (axis, overflow) in [element.layout.overflow_x, element.layout.overflow_y]
        .into_iter()
        .enumerate()
    {
        if !overflow.clips() && offset.values[axis] != 0. {
            return Err(format!(
                "{}.scroll cannot move the {} axis while overflow_{} is visible; use auto, scroll or hidden",
                element.id,
                ["x", "y"][axis],
                ["x", "y"][axis]
            ));
        }
    }
    Ok(())
}

/// Local viewport edges. Rect children use centered coordinates; other containers use top-left.
pub fn viewport(element: &Element) -> Result<[f64; 4], String> {
    let (width, height, origin) = match element.geometry.as_ref() {
        Geometry::Rectangle { width, height, .. } => {
            (math::to_f64(*width)?, math::to_f64(*height)?, true)
        }
        _ => (
            dimension(element.layout.width, "width")?,
            dimension(element.layout.height, "height")?,
            false,
        ),
    };
    if width < 0. || height < 0. {
        return Err("scroll viewport dimensions must be nonnegative".into());
    }
    let [left, top] = if origin {
        [-width / 2., -height / 2.]
    } else {
        [0., 0.]
    };
    Ok([left, top, left + width, top + height])
}

fn dimension(value: Option<crate::Number>, name: &str) -> Result<f64, String> {
    let value = value.ok_or_else(|| format!("scroll viewport requires an explicit {name}"))?;
    if value.unit != Unit::Length {
        return Err("scroll viewport percentages require resolved layout evidence".into());
    }
    math::to_f64(value.value)
}

/// Clamp after child layout, so resize, folding, font changes and list edits update the range.
pub fn resolve(element: &mut Element) -> Result<(), String> {
    let Some(requested) = element.layout.scroll else {
        return Ok(());
    };
    validate(element, requested)?;
    let mut maximum = range(element)?;
    for (axis, overflow) in [element.layout.overflow_x, element.layout.overflow_y]
        .into_iter()
        .enumerate()
    {
        if !overflow.clips() {
            maximum[axis] = 0.;
        }
    }
    element.layout.scroll_range = maximum;
    let mut offset = requested.values;
    for axis in 0..2 {
        if requested.unit == Unit::Percent {
            offset[axis] = math::float_arithmetic("*", offset[axis], maximum[axis])?;
        }
        offset[axis] = offset[axis].clamp(0., maximum[axis]);
    }
    element.layout.scroll = Some(Vec2 {
        values: offset,
        unit: Unit::Length,
    });
    Ok(())
}

pub fn range(element: &Element) -> Result<[f64; 2], String> {
    let viewport = viewport(element)?;
    let padding = math::to_f64(element.layout.padding)?;
    let mut maximum = [0.0f64; 2];
    if let Some(children) = element.geometry.children() {
        for child in children {
            let bounds = content_bounds(child)?;
            for axis in 0..2 {
                maximum[axis] = maximum[axis].max(bounds[axis + 2] + padding - viewport[axis + 2]);
            }
        }
    }
    if let Geometry::Code {
        document,
        options,
        presentation,
        ..
    } = element.geometry.as_ref()
    {
        let height = if let Some(event) = presentation {
            let from = code_height(
                &event.from.document,
                &event.from.folded,
                event.from.options.line_height,
            )?;
            let to = code_height(
                &event.to.document,
                &event.to.folded,
                event.to.options.line_height,
            )?;
            math::motion_lerp(from, to, event.progress, true)?.max(0.)
        } else {
            code_height(document, &Default::default(), options.line_height)?
        };
        maximum[1] = maximum[1].max(height + padding - viewport[3]);
    }
    Ok(maximum)
}

/// Source line to displayed row. The extra sentinel line gives the logical row count.
pub fn visible_row(
    document: &crate::code::Document,
    folded: &imbl::OrdSet<std::sync::Arc<str>>,
    line: usize,
) -> Result<usize, String> {
    let count = document.text.lines().count().max(1);
    if line == 0 || line > count + 1 {
        return Err(format!(
            "CodeView line {line} is outside its 1..={count} source lines"
        ));
    }
    let mut line = line;
    let mut ranges: Vec<_> = document
        .scopes
        .iter()
        .filter(|scope| folded.contains(&scope.id))
        .map(|scope| (scope.start_line, scope.end_line))
        .collect();
    ranges.sort_unstable();
    for &(first, last) in &ranges {
        if first < line && line <= last {
            line = first;
        }
    }
    let mut hidden = 0;
    let mut previous_end = 0;
    for (first, last) in ranges {
        let low = (first + 1).max(previous_end + 1);
        let high = last.min(line.saturating_sub(1));
        if high >= low {
            hidden += high - low + 1;
        }
        previous_end = previous_end.max(last);
    }
    Ok(line.saturating_sub(1 + hidden))
}

fn code_height(
    document: &crate::code::Document,
    folded: &imbl::OrdSet<std::sync::Arc<str>>,
    line_height: crate::Fraction,
) -> Result<f64, String> {
    let sentinel = document.text.lines().count().max(1) + 1;
    math::float_arithmetic(
        "*",
        visible_row(document, folded, sentinel)? as f64,
        math::to_f64(line_height)?,
    )
}

fn content_bounds(element: &Element) -> Result<[f64; 4], String> {
    let mut local = if element.layout.overflow_x.clips() && element.layout.overflow_y.clips() {
        viewport(element)?
    } else if matches!(
        element.geometry.as_ref(),
        Geometry::Box { .. } | Geometry::Code { .. }
    ) {
        let mut bounds = [0.0f64; 4];
        if let Some(width) = element.layout.width {
            bounds[2] = math::to_f64(width.value)?;
        }
        if let Some(height) = element.layout.height {
            bounds[3] = math::to_f64(height.value)?;
        }
        if let Some(children) = element.geometry.children() {
            for child in children {
                let child = content_bounds(child)?;
                for axis in 0..2 {
                    bounds[axis] = bounds[axis].min(child[axis]);
                    bounds[axis + 2] = bounds[axis + 2].max(child[axis + 2]);
                }
            }
        }
        bounds
    } else {
        let mut local = element.clone();
        local.position = Vec2::zero(Unit::Length);
        local.rotation = 0.into();
        local.scale = 1.into();
        local.horizontal = Default::default();
        let bounds = verify::bounds(&local)?;
        [
            bounds.min.values[0],
            bounds.min.values[1],
            bounds.max.values[0],
            bounds.max.values[1],
        ]
    };
    if element.layout.clips() {
        let viewport = viewport(element)?;
        for (axis, overflow) in [element.layout.overflow_x, element.layout.overflow_y]
            .into_iter()
            .enumerate()
        {
            if overflow.clips() {
                local[axis] = viewport[axis];
                local[axis + 2] = viewport[axis + 2];
            }
        }
    }
    math::linear::transformed_box(
        local,
        element.position.values,
        math::to_f64(element.rotation)?,
        math::to_f64(element.scale)?,
        [
            math::to_f64(element.horizontal.scale)?,
            math::to_f64(element.horizontal.origin.value)?,
        ],
    )
}
