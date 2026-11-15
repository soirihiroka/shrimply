use crate::{Element, Geometry, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

/// COW allocation guards for shared font, text, media and captured type payloads.
/// Weak guards preserve allocation identity without keeping expired payloads alive.
#[derive(Clone, Debug, Default)]
pub struct Budget {
    assets: imbl::OrdMap<usize, Weak<Vec<u8>>>,
    media: imbl::OrdMap<usize, Weak<crate::media::Asset>>,
    text: imbl::OrdMap<usize, Weak<str>>,
    range_origins: imbl::OrdMap<usize, Weak<crate::code::view::RangeOrigin>>,
    type_environments: imbl::OrdMap<usize, Weak<BTreeMap<String, crate::Type>>>,
    plans: imbl::OrdMap<usize, Weak<crate::plan::Animation>>,
    shared_visits: u64,
    minimum_depth: Option<usize>,
}
/// Cost independent of the caller's shared-resource receipt set.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cost {
    pub bytes: usize,
    pub items: usize,
    pub depth: usize,
}
impl Budget {
    fn shared(&mut self) {
        self.shared_visits = self
            .shared_visits
            .checked_add(1)
            .expect("shared resource visit counter overflow");
    }
    fn observe_depth(&mut self, depth: usize) {
        self.minimum_depth = Some(self.minimum_depth.map_or(depth, |old| old.min(depth)));
    }
    pub fn metadata_bytes(&self) -> usize {
        self.assets.len() * FONT_ENTRY_BYTES
            + self.media.len() * MEDIA_ENTRY_BYTES
            + self.text.len() * TEXT_ENTRY_BYTES
            + self.range_origins.len() * RANGE_ORIGIN_ENTRY_BYTES
            + self.type_environments.len() * TYPE_ENVIRONMENT_ENTRY_BYTES
            + self.plans.len() * PLAN_ENTRY_BYTES
    }
    /// Charge a captured immutable type environment once per COW execution budget.
    pub fn charge_types(
        &mut self,
        types: &Arc<BTreeMap<String, crate::Type>>,
        bytes: &mut usize,
    ) -> Result<(), String> {
        if types.is_empty() {
            return Ok(());
        }
        let id = Arc::as_ptr(types) as usize;
        if self
            .type_environments
            .get(&id)
            .and_then(Weak::upgrade)
            .is_some_and(|previous| Arc::ptr_eq(&previous, types))
        {
            return Ok(());
        }
        let mut remaining = *bytes;
        for (name, ty) in types.iter() {
            crate::type_budget::charge(ty, &mut remaining)?;
            remaining = remaining
                .checked_sub(name.len() + std::mem::size_of::<String>())
                .ok_or("captured type environment exceeds the retained-data budget")?;
        }
        *bytes = remaining
            .checked_sub(
                TYPE_ENVIRONMENT_ENTRY_BYTES
                    + std::mem::size_of::<BTreeMap<String, crate::Type>>()
                    + 2 * std::mem::size_of::<usize>(),
            )
            .ok_or("captured type environment exceeds the retained-data budget")?;
        self.type_environments.insert(id, Arc::downgrade(types));
        Ok(())
    }
    fn contains(&self, bytes: &Arc<Vec<u8>>) -> bool {
        self.assets
            .get(&(Arc::as_ptr(bytes) as usize))
            .and_then(Weak::upgrade)
            .is_some_and(|existing| Arc::ptr_eq(&existing, bytes))
    }
}
const PLAN_ENTRY_BYTES: usize = std::mem::size_of::<(usize, Weak<crate::plan::Animation>)>()
    + std::mem::size_of::<imbl::OrdMap<usize, Weak<crate::plan::Animation>>>();
const TYPE_ENVIRONMENT_ENTRY_BYTES: usize =
    std::mem::size_of::<imbl::OrdMap<usize, Weak<BTreeMap<String, crate::Type>>>>()
        + std::mem::size_of::<(usize, Weak<BTreeMap<String, crate::Type>>)>();
const RANGE_ORIGIN_ENTRY_BYTES: usize =
    std::mem::size_of::<imbl::OrdMap<usize, Weak<crate::code::view::RangeOrigin>>>()
        + std::mem::size_of::<(usize, Weak<crate::code::view::RangeOrigin>)>();
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
        budget.shared();
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
        fonts: &mut Budget,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("generated value nesting limit exceeded".into());
        }
        fonts.observe_depth(depth);
        take(bytes, items, std::mem::size_of_val(value))?;
        match value {
            serde_json::Value::String(s) => {
                *bytes = bytes
                    .checked_sub(s.len())
                    .ok_or("generated value byte budget exceeded")?;
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    json(value, bytes, items, depth - 1, fonts)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    json(value, bytes, items, depth - 1, fonts)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn plan(
        value: &Arc<crate::plan::Animation>,
        bytes: &mut usize,
        items: &mut usize,
        depth: usize,
        budget: &mut Budget,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("animation plan nesting limit exceeded".into());
        }
        budget.shared();
        budget.observe_depth(depth);
        let id = Arc::as_ptr(value) as usize;
        if budget
            .plans
            .get(&id)
            .and_then(Weak::upgrade)
            .is_some_and(|old| Arc::ptr_eq(&old, value))
        {
            return Ok(());
        }
        take(
            bytes,
            items,
            PLAN_ENTRY_BYTES
                + std::mem::size_of::<crate::plan::Animation>()
                + std::mem::size_of::<crate::plan::Node>(),
        )?;
        if let Some(origin) = &value.origin {
            shared_text(&origin.location.source, bytes, items, budget)?;
            if let Some(binding) = &origin.binding {
                shared_text(binding, bytes, items, budget)?;
            }
        }
        use crate::plan::Node;
        match value.node.as_ref() {
            Node::Element(element) => visit(
                &Value::Element(element.clone()),
                bytes,
                items,
                depth - 1,
                budget,
            )?,
            Node::Container(children)
            | Node::Scope { children, .. }
            | Node::Effect { children, .. } => {
                if let Node::Scope { elements, .. } = value.node.as_ref() {
                    for element in elements {
                        visit(
                            &Value::Element(element.clone()),
                            bytes,
                            items,
                            depth - 1,
                            budget,
                        )?;
                    }
                }
                if let Node::Effect { effect, .. } = value.node.as_ref() {
                    shared_text(&effect.operation, bytes, items, budget)?;
                    for (key, arg) in &effect.arguments {
                        take(bytes, items, key.len())?;
                        visit(arg, bytes, items, depth - 1, budget)?;
                    }
                }
                for child in children {
                    plan(child, bytes, items, depth - 1, budget)?;
                }
            }
            Node::Cell { to, .. } => visit(to, bytes, items, depth - 1, budget)?,
            Node::Repeat { animation, .. } => plan(animation, bytes, items, depth - 1, budget)?,
            Node::Hold(_) => {}
        }
        budget.plans.insert(id, Arc::downgrade(value));
        Ok(())
    }
    fn code_range(
        range: &crate::code::view::Range,
        bytes: &mut usize,
        items: &mut usize,
        budget: &mut Budget,
    ) -> Result<(), String> {
        let Some(origin) = &range.origin else {
            return Ok(());
        };
        budget.shared();
        let id = Arc::as_ptr(origin) as usize;
        if budget
            .range_origins
            .get(&id)
            .and_then(Weak::upgrade)
            .is_some_and(|previous| Arc::ptr_eq(&previous, origin))
        {
            return Ok(());
        }
        take(
            bytes,
            items,
            RANGE_ORIGIN_ENTRY_BYTES
                + std::mem::size_of_val(origin.as_ref())
                + std::mem::size_of::<(usize, usize)>(),
        )?;
        shared_text(&origin.owner, bytes, items, budget)?;
        shared_text(&origin.revision, bytes, items, budget)?;
        if let Some(scope) = &origin.scope_id {
            shared_text(scope, bytes, items, budget)?;
        }
        budget.range_origins.insert(id, Arc::downgrade(origin));
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
        for span in &options.spans {
            code_range(&span.range, bytes, items, fonts)?;
        }
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
        fonts.observe_depth(depth);
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
                    for range in snapshot.selection.iter() {
                        code_range(range, bytes, items, fonts)?;
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
                fonts.shared();
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
            Geometry::Box { children, .. } | Geometry::Code { children, .. } => {
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
        fonts.observe_depth(depth);
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
            Value::Data(value) => json(value, bytes, items, depth - 1, fonts)?,
            Value::Dict(values) => {
                for (key, value) in values.iter() {
                    *bytes = bytes
                        .checked_sub(key.len())
                        .ok_or("generated value byte budget exceeded")?;
                    json(value, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::List(_) | Value::Array { .. } => {
                let values = value.list()?;
                *bytes = bytes
                    .checked_sub(values.metadata_bytes())
                    .ok_or("generated collection metadata byte budget exceeded")?;
                if let Some(cost) = values.budget_cost(depth) {
                    *bytes = bytes
                        .checked_sub(cost.bytes)
                        .ok_or("generated value byte budget exceeded")?;
                    *items = items
                        .checked_sub(cost.items)
                        .ok_or("generated value item budget exceeded")?;
                    fonts.observe_depth(depth - cost.depth + 1);
                } else {
                    let (before_bytes, before_items, shared) =
                        (*bytes, *items, fonts.shared_visits);
                    let outer_depth = fonts.minimum_depth.take();
                    let result = values
                        .iter()
                        .try_for_each(|value| visit(value, bytes, items, depth - 1, fonts));
                    let nested_depth = fonts.minimum_depth.take();
                    fonts.minimum_depth = outer_depth;
                    if let Some(nested) = nested_depth {
                        fonts.observe_depth(nested);
                    }
                    result?;
                    if shared == fonts.shared_visits {
                        values.remember_budget_cost(Cost {
                            bytes: before_bytes - *bytes,
                            items: before_items - *items,
                            depth: nested_depth.map_or(1, |minimum| depth - minimum + 1),
                        });
                    }
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
            Value::Animation(value) => plan(value, bytes, items, depth - 1, fonts)?,
            Value::Predicate(value) => {
                element(&value.left, bytes, items, depth - 1, fonts)?;
                if let Some(right) = &value.right {
                    element(right, bytes, items, depth - 1, fonts)?;
                }
            }
            Value::Font(font) => {
                let size = match font {
                    crate::Font::Asset(values) => {
                        fonts.shared();
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
