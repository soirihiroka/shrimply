use super::*;
use sanim_core::code::view::State;
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
            selection: Arc::from([]),
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
        && let Some((argument, _)) =
            sanim_core::code::view::INPUTS
                .iter()
                .find(|(argument, property)| {
                    args.contains_key(*argument)
                        && context
                            .animation
                            .reactive
                            .binding(&view.id, *property)
                            .is_some()
                })
    {
        return Err(format!(
            "CodeView.style cannot also write signal-controlled {argument}; change the corresponding mutable signal input"
        ));
    }
    let current = sample_document(view, state.document.clone(), &context)?;
    if !Arc::ptr_eq(&current, &state.document) {
        state.document = current;
        state.selection = Arc::from([]);
    }
    let before = state.snapshot();
    match name {
        "CodeView.patch" => {
            let target_size = args
                .get("code")
                .map(Value::string)
                .transpose()?
                .map(str::len)
                .or_else(|| {
                    args.get("to")
                        .and_then(|value| document(value).ok())
                        .map(|document| document.text.len())
                })
                .unwrap_or(0);
            context.charge_work(state.document.text.len().saturating_add(target_size))?;
            let document = patch_edit::apply(&state.document, args, &view.id)?;
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
            state.selection = Arc::from([]);
            Arc::make_mut(&mut state.options)
                .spans
                .retain(|span| span.range.origin.is_none());
        }
        "CodeView.select" => {
            if let Some(value) = args.get("ranges") {
                context.created.push(value.clone());
            }
            let selection: Vec<Range> = args
                .get("ranges")
                .map(|value| value.list()?.iter().map(range).collect())
                .transpose()?
                .unwrap_or_default();
            sanim_core::code::view::validate_ranges(&view.id, &state.document, &selection)?;
            state.selection = selection.into();
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
            if let Some(value) = args.get("line_numbers") {
                options.line_numbers = value.boolean()?;
            }
            if let Some(value) = args.get("scope_indicators") {
                options.scope_indicators = value.boolean()?;
            }
            sanim_core::code::view::validate_ranges(
                &view.id,
                &state.document,
                &options
                    .spans
                    .iter()
                    .map(|span| span.range.clone())
                    .collect::<Vec<_>>(),
            )?;
            state.options = Arc::new(options);
        }
        "CodeView.fold" | "CodeView.unfold" => {
            let selected = range(args.get("range").ok_or("fold/unfold requires range=; select one with view.range(name=...) or view.range(line=...)")?)?;
            let scope = selectors::fold_target(&state.document, &view.id, &selected)?;
            if name == "CodeView.fold"
                && scope.hidden_end_line(&state.document.text)? <= scope.start_line
            {
                return Err(format!(
                    "cannot fold '{}': no complete body lines can be hidden because its closing line contains other source; put the sibling declaration on a separate line",
                    scope.label
                ));
            }
            let id = scope.id.clone();
            if name == "CodeView.fold" {
                if !state.folded.contains(&id) {
                    context.created.push(Value::String(id.clone()));
                }
                state.folded.insert(id);
            } else {
                state.folded.remove(&id);
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
    let selection_bytes = if Arc::ptr_eq(&before.selection, &after.selection) {
        0
    } else {
        after
            .selection
            .len()
            .saturating_mul(std::mem::size_of::<Range>())
    };
    let options_bytes = if Arc::ptr_eq(&before.options, &after.options) {
        0
    } else {
        std::mem::size_of::<Options>().saturating_add(
            after
                .options
                .spans
                .capacity()
                .saturating_mul(std::mem::size_of::<sanim_core::code::view::Override>()),
        )
    };
    let buffers = selection_bytes.saturating_add(options_bytes);
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
