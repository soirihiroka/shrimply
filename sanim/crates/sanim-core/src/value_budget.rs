use crate::{Element, Geometry, Value};
use std::sync::{Arc, Weak};

/// COW allocation guards for shared native font, text and encoded-media payloads. A guard never keeps
/// the font bytes alive and cannot confuse a reused address with an older asset.
#[derive(Clone, Debug, Default)]
pub struct Budget {
    assets: imbl::OrdMap<usize, Weak<Vec<u8>>>,
    media: imbl::OrdMap<usize, Weak<crate::media::Asset>>,
    text: imbl::OrdMap<usize, Weak<str>>,
}
impl Budget {
    pub fn metadata_bytes(&self) -> usize {
        self.assets.len() * FONT_ENTRY_BYTES
            + self.media.len() * MEDIA_ENTRY_BYTES
            + self.text.len() * TEXT_ENTRY_BYTES
    }
    fn contains(&self, bytes: &Arc<Vec<u8>>) -> bool {
        self.assets
            .get(&(Arc::as_ptr(bytes) as usize))
            .and_then(Weak::upgrade)
            .is_some_and(|existing| Arc::ptr_eq(&existing, bytes))
    }
}
const TEXT_ENTRY_BYTES: usize = std::mem::size_of::<imbl::OrdMap<usize, Weak<str>>>()
    + std::mem::size_of::<(usize, Weak<str>)>();
const MEDIA_ENTRY_BYTES: usize =
    std::mem::size_of::<imbl::OrdMap<usize, Weak<crate::media::Asset>>>()
        + std::mem::size_of::<(usize, Weak<crate::media::Asset>)>();
const FONT_ENTRY_BYTES: usize = std::mem::size_of::<imbl::OrdMap<usize, Weak<Vec<u8>>>>()
    + std::mem::size_of::<(usize, Weak<Vec<u8>>)>()
    + std::mem::size_of::<Vec<u8>>()
    + std::mem::size_of::<(usize, usize)>();

/// Charge generated values before retaining or copying them into expression caches.
/// Traversal itself stops at the configured limits; no serialization allocation.
pub fn charge(
    value: &Value,
    bytes: &mut usize,
    items: &mut usize,
    max_depth: usize,
) -> Result<(), String> {
    charge_with_budget(value, bytes, items, max_depth, &mut Budget::default())
}

/// Charge several independently retained roots against one shared payload budget.
/// Reuse the context for a frame or execution prefix; clone it to checkpoint COW state.
pub fn charge_with_budget(
    value: &Value,
    bytes: &mut usize,
    items: &mut usize,
    max_depth: usize,
    fonts: &mut Budget,
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
    fn shared_text(
        text: &Arc<str>,
        bytes: &mut usize,
        items: &mut usize,
        budget: &mut Budget,
    ) -> Result<(), String> {
        let id = Arc::as_ptr(text) as *const () as usize;
        if !budget
            .text
            .get(&id)
            .and_then(Weak::upgrade)
            .is_some_and(|previous| Arc::ptr_eq(&previous, text))
        {
            take(
                bytes,
                items,
                TEXT_ENTRY_BYTES + std::mem::size_of::<(usize, usize)>() + text.len(),
            )?;
            budget.text.insert(id, Arc::downgrade(text));
        }
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
    fn code_options(
        options: &crate::code::view::Options,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
        fonts: &mut Budget,
    ) -> Result<(), String> {
        visit(
            &Value::Font(options.font.clone()),
            bytes,
            items,
            depth,
            fonts,
        )?;
        let metadata = std::mem::size_of_val(options)
            + std::mem::size_of_val(options.spans.as_slice())
            + options
                .theme
                .styles
                .iter()
                .map(|(key, style)| key.len() + std::mem::size_of_val(style))
                .sum::<usize>();
        take(bytes, items, metadata)
    }
    fn element(
        value: &Element,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
        fonts: &mut Budget,
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
        if let Geometry::Code {
            document,
            options,
            presentation,
            ..
        } = value.geometry.as_ref()
        {
            visit(
                &Value::Code(document.clone()),
                bytes,
                items,
                depth - 1,
                fonts,
            )?;
            code_options(options, bytes, items, depth - 1, fonts)?;
            if let Some(presentation) = presentation {
                let mut previous = None;
                for snapshot in [&presentation.from, &presentation.to] {
                    if !Arc::ptr_eq(document, &snapshot.document)
                        && previous.is_none_or(|other: &crate::code::view::Snapshot| {
                            !Arc::ptr_eq(&other.document, &snapshot.document)
                        })
                    {
                        visit(
                            &Value::Code(snapshot.document.clone()),
                            bytes,
                            items,
                            depth - 1,
                            fonts,
                        )?;
                    }
                    if !Arc::ptr_eq(options, &snapshot.options)
                        && previous.is_none_or(|other: &crate::code::view::Snapshot| {
                            !Arc::ptr_eq(&other.options, &snapshot.options)
                        })
                    {
                        code_options(&snapshot.options, bytes, items, depth - 1, fonts)?;
                    }
                    let metadata = std::mem::size_of_val(snapshot.as_ref())
                        + snapshot
                            .folded
                            .iter()
                            .map(|id| std::mem::size_of_val(id) + id.len())
                            .sum::<usize>()
                        + std::mem::size_of_val(snapshot.selection.as_ref());
                    take(bytes, items, metadata)?;
                    previous = Some(snapshot.as_ref());
                }
                take(bytes, items, std::mem::size_of_val(presentation))?;
            }
        }
        match value.geometry.as_ref() {
            Geometry::Media { asset, .. } => {
                let id = Arc::as_ptr(asset) as usize;
                if !fonts
                    .media
                    .get(&id)
                    .and_then(Weak::upgrade)
                    .is_some_and(|existing| Arc::ptr_eq(&existing, asset))
                {
                    take(
                        bytes,
                        items,
                        MEDIA_ENTRY_BYTES
                            + std::mem::size_of_val(asset.as_ref())
                            + asset.bytes().len()
                            + asset.fingerprint().len(),
                    )?;
                    fonts.media.insert(id, Arc::downgrade(asset));
                }
            }
            Geometry::Box { children, .. }
            | Geometry::Rectangle { children, .. }
            | Geometry::Code { children, .. } => {
                for child in children {
                    element(child, bytes, items, depth - 1, fonts)?;
                }
            }
            Geometry::Text {
                text,
                font,
                outline,
                ..
            } => {
                visit(&Value::Font(font.clone()), bytes, items, depth - 1, fonts)?;
                shared_text(text, bytes, items, fonts)?;
                *bytes = bytes
                    .checked_sub(outline.len() * std::mem::size_of::<crate::TextSegment>())
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
        fonts: &mut Budget,
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
                            visit(value, bytes, items, depth - 1, fonts)?;
                            if let Some(format) = format {
                                visit(format, bytes, items, depth - 1, fonts)?;
                            }
                        }
                    }
                }
            }
            Value::Sql(sql) => {
                *bytes = bytes
                    .checked_sub(sql.text.len())
                    .ok_or("SQL template exceeds generated value byte budget")?;
                visit(&sql.parameters, bytes, items, depth - 1, fonts)?;
            }
            Value::SqlIdent(name) => {
                *bytes = bytes
                    .checked_sub(name.len())
                    .ok_or("SQL identifier exceeds generated value byte budget")?;
            }
            Value::String(s) => shared_text(s, bytes, items, fonts)?,
            Value::Code(document) => {
                shared_text(&document.text, bytes, items, fonts)?;
                *bytes = bytes
                    .checked_sub(
                        std::mem::size_of_val(document.as_ref())
                            + document.syntax.retained_bytes
                            + document.path.len()
                            + std::mem::size_of_val(document.scopes.as_ref())
                            + std::mem::size_of_val(document.highlights.as_ref())
                            + std::mem::size_of_val(document.diagnostics.as_ref()),
                    )
                    .ok_or("Code document exceeds generated value byte budget")?;
                for scope in document.scopes.iter() {
                    *bytes = bytes
                        .checked_sub(scope.id.len() + scope.label.len())
                        .ok_or("Code syntax tree exceeds generated value byte budget")?;
                }
                for highlight in document.highlights.iter() {
                    *bytes = bytes
                        .checked_sub(highlight.kind.len())
                        .ok_or("Code highlighting exceeds generated value byte budget")?;
                }
                for issue in document.diagnostics.iter() {
                    *bytes = bytes
                        .checked_sub(issue.message.len())
                        .ok_or("Code diagnostics exceed generated value byte budget")?;
                }
                if let crate::code::Content::Patch(patch) = &document.content {
                    for file in &patch.files {
                        *bytes = bytes
                            .checked_sub(file.old_path.len() + file.new_path.len())
                            .ok_or("Code patch exceeds generated value byte budget")?;
                        for hunk in &file.hunks {
                            for line in &hunk.lines {
                                let (crate::code::PatchLine::Added(text)
                                | crate::code::PatchLine::Removed(text)
                                | crate::code::PatchLine::Context(text)) = line;
                                *bytes = bytes
                                    .checked_sub(text.len() + std::mem::size_of_val(line))
                                    .ok_or("Code patch exceeds generated value byte budget")?;
                            }
                        }
                    }
                }
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
                    visit(value, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::Record(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    visit(value, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::Some(value) => visit(value, bytes, items, depth - 1, fonts)?,
            Value::Variant { fields, .. } => {
                for value in fields.iter() {
                    visit(value, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::Element(value) => element(value, bytes, items, depth - 1, fonts)?,
            Value::Predicate(value) => {
                element(&value.left, bytes, items, depth - 1, fonts)?;
                if let Some(right) = &value.right {
                    element(right, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::Font(font) => {
                let size = match font {
                    crate::Font::Asset(values) => {
                        if fonts.contains(values) {
                            return Ok(());
                        }
                        let size = values
                            .len()
                            .checked_add(FONT_ENTRY_BYTES)
                            .ok_or("shared font byte budget overflow")?;
                        *bytes = bytes
                            .checked_sub(size)
                            .ok_or("generated value byte budget exceeded")?;
                        // Publish the identity only after its payload and guard were charged.
                        fonts
                            .assets
                            .insert(Arc::as_ptr(values) as usize, Arc::downgrade(values));
                        return Ok(());
                    }
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
    visit(value, bytes, items, max_depth, fonts)
}
