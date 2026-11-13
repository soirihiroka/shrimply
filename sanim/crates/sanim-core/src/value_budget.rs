use crate::{Element, Geometry, Value};

/// Charge generated values before retaining or copying them into expression caches.
/// Traversal itself stops at the configured limits; no serialization allocation.
pub fn charge(
    value: &Value,
    bytes: &mut usize,
    items: &mut usize,
    max_depth: usize,
) -> Result<(), String> {
    fn take(bytes: &mut usize, items: &mut usize, size: usize) -> Result<(), String> {
        *bytes = bytes
            .checked_sub(size)
            .ok_or("generated value byte budget exceeded")?;
        *items = items
            .checked_sub(1)
            .ok_or("generated value item budget exceeded")?;
        Ok(())
    }
    fn json(
        value: &serde_json::Value,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("generated value nesting limit exceeded".into());
        }
        take(bytes, items, std::mem::size_of_val(value))?;
        match value {
            serde_json::Value::String(s) => {
                *bytes = bytes
                    .checked_sub(s.len())
                    .ok_or("generated value byte budget exceeded")?;
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    json(value, bytes, items, depth - 1)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    json(value, bytes, items, depth - 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn element(
        value: &Element,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("generated element nesting limit exceeded".into());
        }
        take(bytes, items, std::mem::size_of_val(value) + value.id.len())?;
        if let crate::Paint::LinearGradient { stops, .. } = &value.fill {
            *bytes = bytes
                .checked_sub(std::mem::size_of_val(stops.as_ref()))
                .ok_or("gradient paint exceeds generated value byte budget")?;
        }
        if let Some(stroke) = &value.stroke
            && let crate::Paint::LinearGradient { stops, .. } = &stroke.paint
        {
            *bytes = bytes
                .checked_sub(std::mem::size_of_val(stops.as_ref()))
                .ok_or("gradient border exceeds generated value byte budget")?;
        }
        match value.geometry.as_ref() {
            Geometry::Group { children } | Geometry::Rectangle { children, .. } => {
                for child in children {
                    element(child, bytes, items, depth - 1)?;
                }
            }
            Geometry::Text { text, outline, .. } => {
                *bytes = bytes
                    .checked_sub(
                        text.len() + outline.len() * std::mem::size_of::<crate::TextSegment>(),
                    )
                    .ok_or("generated value byte budget exceeded")?;
            }
            Geometry::Polygon { points } | Geometry::Polyline { points, .. } => {
                *bytes = bytes
                    .checked_sub(std::mem::size_of_val(points.as_slice()))
                    .ok_or("generated value byte budget exceeded")?;
            }
            _ => {}
        }
        Ok(())
    }
    fn visit(
        value: &Value,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("generated value nesting limit exceeded".into());
        }
        take(bytes, items, std::mem::size_of_val(value))?;
        match value {
            Value::DataFrame(frame) => {
                *bytes = bytes
                    .checked_sub(frame.retained_bytes)
                    .ok_or("DataFrame plan metadata exceeds generated value byte budget")?;
                *items = items
                    .checked_sub(frame.retained_items)
                    .ok_or("DataFrame retained input exceeds generated value item budget")?;
            }
            Value::Series(series) => {
                *bytes = bytes
                    .checked_sub(
                        series
                            .frame
                            .retained_bytes
                            .saturating_add(series.expression_bytes),
                    )
                    .ok_or(
                        "Series expression and retained input exceed generated value byte budget",
                    )?;
                *items = items
                    .checked_sub(series.frame.retained_items.saturating_add(series.nodes))
                    .ok_or(
                        "Series expression and retained input exceed generated value item budget",
                    )?;
            }
            Value::Template(template) => {
                for part in template.parts.iter() {
                    take(bytes, items, std::mem::size_of_val(part))?;
                    match part {
                        crate::TemplatePart::Text(text) => {
                            *bytes = bytes
                                .checked_sub(text.len())
                                .ok_or("template literal exceeds generated value byte budget")?
                        }
                        crate::TemplatePart::Hole { value, format, .. } => {
                            visit(value, bytes, items, depth - 1)?;
                            if let Some(format) = format {
                                visit(format, bytes, items, depth - 1)?;
                            }
                        }
                    }
                }
            }
            Value::Sql(sql) => {
                *bytes = bytes
                    .checked_sub(sql.text.len())
                    .ok_or("SQL template exceeds generated value byte budget")?;
                visit(&sql.parameters, bytes, items, depth - 1)?;
            }
            Value::SqlIdent(name) => {
                *bytes = bytes
                    .checked_sub(name.len())
                    .ok_or("SQL identifier exceeds generated value byte budget")?;
            }
            Value::String(s) => {
                *bytes = bytes
                    .checked_sub(s.len())
                    .ok_or("generated value byte budget exceeded")?;
            }
            Value::Data(value) => json(value, bytes, items, depth - 1)?,
            Value::Dict(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    json(value, bytes, items, depth - 1)?;
                }
            }
            Value::List(_) | Value::Array { .. } => {
                for value in value.list()? {
                    visit(value, bytes, items, depth - 1)?;
                }
            }
            Value::Record(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    visit(value, bytes, items, depth - 1)?;
                }
            }
            Value::Some(value) => visit(value, bytes, items, depth - 1)?,
            Value::Variant { fields, .. } => {
                for value in fields.iter() {
                    visit(value, bytes, items, depth - 1)?;
                }
            }
            Value::Element(value) => element(value, bytes, items, depth - 1)?,
            Value::Predicate(value) => {
                element(&value.left, bytes, items, depth - 1)?;
                if let Some(right) = &value.right {
                    element(right, bytes, items, depth - 1)?;
                }
            }
            Value::Font(font) => {
                let size = match font {
                    crate::Font::Asset(values) => values.len(),
                    crate::Font::Family(families) => families
                        .iter()
                        .map(|family| std::mem::size_of::<String>() + family.len())
                        .sum(),
                };
                *bytes = bytes
                    .checked_sub(size)
                    .ok_or("generated value byte budget exceeded")?;
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, bytes, items, max_depth)
}
