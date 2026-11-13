use super::*;
use sanim_core::code::{Content, view::State};
use std::collections::BTreeSet;

pub(super) fn execute(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let Some(Value::Element(view)) = args.get("view") else {
        return Err("code operation requires view=CodeView(...)".into());
    };
    let Geometry::Code {
        document: original,
        options,
        ..
    } = view.geometry.as_ref()
    else {
        return Err(format!(
            "'{}' is not a CodeView; use CodeView(src=\"file.py\")",
            view.id
        ));
    };
    let start = *context.cursor;
    let end = math::timeline_end(start, context.duration)?;
    let mut state = if let Some(state) = context.animation.code_views.get(&view.id) {
        state.as_ref().clone()
    } else {
        if context.animation.find(&view.id).is_none() {
            context.animation.add(view.as_ref().clone(), start)?;
        }
        State {
            document: original.clone(),
            options: options.clone(),
            folded: imbl::OrdSet::new(),
            selection: vec![],
            end: start,
            events: Default::default(),
        }
    };
    if start < state.end {
        return Err(format!(
            "{name}: CodeView '{}' already has a pending edit ending at {}s; sequence edits or use separate views",
            view.id, state.end
        ));
    }
    if name == "CodeView.patch"
        && context
            .animation
            .reactive
            .binding(&view.id, Property::Code)
            .is_some()
    {
        return Err("CodeView.patch cannot also write signal-controlled code; assign the mutable Code source instead, or use a static CodeView for explicit timed patches".into());
    }
    if name == "CodeView.style"
        && [
            ("theme", Property::CodeTheme),
            ("spans", Property::CodeSpans),
        ]
        .iter()
        .any(|(argument, property)| {
            args.contains_key(*argument)
                && context
                    .animation
                    .reactive
                    .binding(&view.id, *property)
                    .is_some()
        })
    {
        return Err("CodeView.style cannot also write signal-controlled theme/spans; change the corresponding mutable signal input".into());
    }
    let current = sample_document(view, state.document.clone(), &context)?;
    if !Arc::ptr_eq(&current, &state.document) {
        state.document = current;
        state.selection.clear();
    }
    let before = state.snapshot();
    match name {
        "CodeView.patch" => {
            let target = document(args.get("to").ok_or("CodeView.patch could not load src; provide a readable unified .patch or .diff file")?)?;
            let document = if matches!(target.content, Content::Patch(_)) {
                state
                    .document
                    .apply(&target, args.get("file").map(Value::string).transpose()?)?
            } else {
                target.as_ref().clone()
            };
            context.charge_work(document.text.len())?;
            state.document = Arc::new(syntax::parse(document)?);
            context.created.push(Value::Code(state.document.clone()));
            let valid: BTreeSet<_> = state
                .document
                .scopes
                .iter()
                .map(|scope| scope.id.clone())
                .collect();
            state.folded = state
                .folded
                .iter()
                .filter(|scope| valid.contains(*scope))
                .cloned()
                .collect();
            // Selection ranges describe a source revision; retaining offsets after
            // an edit would silently select unrelated code. Choose them again.
            state.selection.clear();
        }
        "CodeView.select" => {
            if let Some(value) = args.get("ranges") {
                context.created.push(value.clone());
            }
            state.selection = args
                .get("ranges")
                .map(|value| value.list()?.iter().map(range).collect())
                .transpose()?
                .unwrap_or_default();
        }
        "CodeView.style" => {
            let mut options = state.options.as_ref().clone();
            if let Some(value) = args.get("spans") {
                context.created.push(value.clone());
                options.spans = spans(value)?;
            }
            if let Some(value) = args.get("theme") {
                context.created.push(value.clone());
                options.theme = Arc::new(theme(value)?);
            }
            state.options = Arc::new(options);
        }
        "CodeView.fold" | "CodeView.unfold" => {
            if !state.document.syntax_parsed {
                return Err(format!(
                    "{name}: {} has no native syntax tree; use a registered language or supply a grammar",
                    state.document.language.name()
                ));
            }
            if !state.document.diagnostics.is_empty() {
                let issue = &state.document.diagnostics[0];
                return Err(format!(
                    "{name}: fix syntax in '{}':{}:{} before folding ({})",
                    state.document.path, issue.line, issue.column, issue.message
                ));
            }
            if args.contains_key("scope") && args.contains_key("level") {
                return Err(format!("{name}: choose scope= or level=, not both"));
            }
            let scope = args.get("scope").map(Value::string).transpose()?;
            let level = args
                .get("level")
                .map(|value| integer(value, "fold level"))
                .transpose()?
                .unwrap_or(1);
            if level == 0 {
                return Err("fold level is 1-based; use level=1 for outermost scopes".into());
            }
            let matching: Vec<_> = state
                .document
                .scopes
                .iter()
                .filter(|candidate| {
                    scope.map_or(candidate.depth >= level, |scope| {
                        candidate.id.as_ref() == scope
                    })
                })
                .map(|scope| scope.id.clone())
                .collect();
            if matching.is_empty() && scope.is_some() {
                return Err(format!(
                    "{name}: scope {:?} does not exist; use view.scopes() to inspect stable syntax scope IDs",
                    scope
                ));
            }
            for id in matching {
                if name == "CodeView.fold" {
                    if !state.folded.contains(&id) {
                        context.created.push(Value::String(id.clone()));
                    }
                    state.folded.insert(id);
                } else {
                    state.folded.remove(&id);
                }
            }
        }
        _ => return Err(format!("unknown code operation '{name}'")),
    }
    let animation = if name == "CodeView.patch" {
        sanim_core::code::view::PatchAnimation::parse(
            args.get("animation")
                .map(Value::string)
                .transpose()?
                .unwrap_or("morph"),
        )?
    } else {
        sanim_core::code::view::PatchAnimation::Morph
    };
    let after = state.snapshot();
    if before == after {
        // The call still advances its declared duration, but keeps no event or
        // duplicate payload receipt when the presentation is unchanged.
        context.created.clear();
        return Ok(Value::Unit);
    }
    let buffers = before
        .selection
        .len()
        .saturating_add(after.selection.len().saturating_mul(2))
        .saturating_mul(std::mem::size_of::<Range>())
        .saturating_add(
            after
                .options
                .spans
                .len()
                .saturating_mul(std::mem::size_of::<sanim_core::code::view::Override>()),
        );
    let metadata = State::event_metadata_bytes()
        .saturating_add(buffers)
        .saturating_add(if context.animation.code_views.contains_key(&view.id) {
            0
        } else {
            std::mem::size_of::<State>()
                + view.id.len()
                + std::mem::size_of::<(String, Arc<State>)>()
        });
    *context.retained_bytes = context.retained_bytes.checked_sub(metadata)
        .ok_or("CodeView timeline metadata exceeds the configured retained-data byte budget; reduce changed edits or increase the host budget")?;
    state.events.insert(
        start,
        sanim_core::code::view::PresentationEvent {
            start,
            end,
            easing: context.easing,
            animation,
            from: before,
            to: after,
        },
    );
    state.end = end;
    context
        .animation
        .code_views
        .insert(view.id.clone(), Arc::new(state));
    Ok(Value::Unit)
}
