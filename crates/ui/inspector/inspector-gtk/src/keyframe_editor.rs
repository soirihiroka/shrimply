use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use gtk::{gdk, gio};
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::{FrameGraph, SearchMenuItem, matches_query, searchable_popover};
use shrimply_editor_state::preferences;
use shrimply_inspector_core::keyframe_graph::{
    FrameGraphAction, FrameGraphComponentAction, FrameGraphState,
};
use shrimply_math_interpolation::Interpolation;
use shrimply_project_document::project::{ItemAddress, Project, Time};
use shrimply_property_model::timeline_value::TextInterpolation;
use uuid::Uuid;

use super::{InspectorContext, keyframe_model};
use crate::player_state;

pub(crate) use super::keyframe_graph::{KeyframeGraph, KeyframePoint, RawSegment, SpeedSegment};
pub(crate) use keyframe_model::project_frame_step;

thread_local! {
    static KEYFRAME_CLIPBOARD: keyframe_model::KeyframeClipboardCache = const { keyframe_model::KeyframeClipboardCache::new() };
}

pub(crate) struct BuiltKeyframeEditor {
    pub(crate) widget: gtk::Widget,
    pub(crate) frame_graph: FrameGraph,
    pub(crate) update_graph: Rc<dyn Fn(KeyframeGraph)>,
    update_playhead: Rc<dyn Fn()>,
}

pub(crate) type CopyKeyframes = Rc<dyn Fn(&[Time]) -> Option<keyframe_model::KeyframeClipboard>>;
pub(crate) type PasteKeyframes =
    Rc<dyn Fn(&keyframe_model::KeyframeClipboard, &[Time]) -> Option<Vec<Time>>>;
pub(crate) type ManagedCopyKeyframes = Rc<dyn Fn(&[Time]) -> Option<usize>>;
pub(crate) type ManagedPasteKeyframes = Rc<dyn Fn(Time) -> Option<usize>>;

pub(crate) enum KeyframeClipboardActions {
    Local {
        copy: CopyKeyframes,
        paste: PasteKeyframes,
    },
    Managed {
        copy: ManagedCopyKeyframes,
        paste: ManagedPasteKeyframes,
    },
}

pub(crate) struct KeyframeEditorActions {
    pub(crate) add_at_time: Rc<dyn Fn(Time)>,
    pub(crate) delete_at_time: Rc<dyn Fn(Time)>,
    pub(crate) update_point: Rc<dyn Fn(Time, Time, f64)>,
    pub(crate) clipboard: KeyframeClipboardActions,
    pub(crate) set_interpolation: Option<Rc<dyn Fn(Uuid, Interpolation)>>,
    pub(crate) text_interpolation: Option<TextInterpolationActions>,
    pub(crate) toggle_playback: Rc<dyn Fn()>,
}

pub(crate) struct TextInterpolationActions {
    pub(crate) get: Rc<dyn Fn(Uuid) -> Option<TextInterpolation>>,
    pub(crate) set: Rc<dyn Fn(Uuid, TextInterpolation)>,
}

struct GraphActionContext {
    actions: Rc<KeyframeEditorActions>,
    project: Rc<RefCell<Project>>,
    selected_item: Option<ItemAddress>,
    select_time: Rc<dyn Fn(Time)>,
    graph_area: Rc<RefCell<Option<gtk::GLArea>>>,
}

pub(crate) fn build(
    context: &InspectorContext,
    graph: KeyframeGraph,
    visible_area: (Time, Time),
    view_state_scope: impl Into<String>,
    actions: KeyframeEditorActions,
) -> BuiltKeyframeEditor {
    let project = context.project.clone();
    let selected_item = context.selected_item.clone();
    let frame_step = project_frame_step(&project.borrow(), selected_item.as_ref());
    let item_range = keyframe_model::bounded_visible_area(
        &project.borrow(),
        selected_item.as_ref(),
        visible_area,
    );
    let playhead = local_playhead(context);
    let mut initial = FrameGraphState::new(graph.clone(), item_range, frame_step, playhead());
    configure_state(
        &mut initial,
        &context.preferences,
        actions.text_interpolation.is_some(),
    );
    let state = context.keyframe_graph_state(view_state_scope, initial);
    state.replace_active_graph(graph);
    state.set_view(
        item_range,
        frame_step,
        playhead(),
        keyframe_model::graph_snapping(&context.preferences),
        true,
        actions.text_interpolation.is_some(),
    );

    let action_context = Rc::new(GraphActionContext {
        actions: Rc::new(actions),
        project: project.clone(),
        selected_item: selected_item.clone(),
        select_time: select_time(context),
        graph_area: Rc::new(RefCell::new(None)),
    });
    let frame_graph = FrameGraph::with_shared_components(state.clone(), {
        let context = action_context.clone();
        move |action| dispatch_action(&context, action)
    });
    action_context
        .graph_area
        .replace(Some(frame_graph.graph_area().clone()));

    let update_graph = {
        let frame_graph = frame_graph.clone();
        let preferences = context.preferences.clone();
        let text_interpolation = action_context.actions.text_interpolation.is_some();
        let playhead = playhead.clone();
        Rc::new(move |updated| {
            let project = project.borrow();
            let item_range = keyframe_model::bounded_visible_area(
                &project,
                selected_item.as_ref(),
                visible_area,
            );
            let frame_step = project_frame_step(&project, selected_item.as_ref());
            drop(project);
            state.replace_active_graph(updated);
            state.set_view(
                item_range,
                frame_step,
                playhead(),
                keyframe_model::graph_snapping(&preferences),
                true,
                text_interpolation,
            );
            frame_graph.refresh();
        }) as Rc<dyn Fn(KeyframeGraph)>
    };
    let update_playhead = {
        let frame_graph = frame_graph.clone();
        let playhead = playhead.clone();
        Rc::new(move || {
            if frame_graph.graph_area().is_mapped() {
                frame_graph.set_playhead(playhead());
            }
        }) as Rc<dyn Fn()>
    };
    frame_graph.graph_area().connect_map({
        let frame_graph = frame_graph.clone();
        let playhead = playhead.clone();
        move |_| frame_graph.set_playhead(playhead())
    });

    BuiltKeyframeEditor {
        widget: frame_graph.widget().clone().upcast(),
        frame_graph,
        update_graph,
        update_playhead,
    }
}

pub(crate) fn connect_graph_refresh_impl(
    context: &InspectorContext,
    label: &'static str,
    editor: &BuiltKeyframeEditor,
    graph: impl Fn() -> Option<KeyframeGraph> + 'static,
) {
    let update_graph = editor.update_graph.clone();
    let update_playhead = editor.update_playhead.clone();
    let graph = Rc::new(graph);
    editor.frame_graph.graph_area().connect_map({
        let graph = graph.clone();
        let update_graph = update_graph.clone();
        move |_| {
            if let Some(graph) = graph() {
                update_graph(graph);
            }
        }
    });
    let graph_area = editor.frame_graph.graph_area().clone();
    let alive = Rc::downgrade(&context.listener_scope);
    player_state::connect_while_alive_named(
        &context.player_state,
        label,
        move || alive.upgrade().is_some(),
        move |event| match event {
            player_state::PlayerEvent::State(_) => update_playhead(),
            player_state::PlayerEvent::Project(_) => {
                if graph_area.is_mapped()
                    && let Some(graph) = graph()
                {
                    update_graph(graph);
                }
            }
        },
    );
}

pub(crate) use connect_graph_refresh_impl as connect_graph_refresh;

fn configure_state(
    state: &mut FrameGraphState,
    preferences: &preferences::SharedPreferences,
    text_interpolation: bool,
) {
    let (enabled, radius) = keyframe_model::graph_snapping(preferences);
    state.set_snapping(enabled, radius);
    state.set_external_clipboard(true);
    state.set_text_interpolation(text_interpolation);
}

fn local_playhead(context: &InspectorContext) -> Rc<dyn Fn() -> Time> {
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let selected_item = context.selected_item.clone();
    Rc::new(move || {
        let position = player_state::snapshot(&player_state).position;
        selected_item
            .as_ref()
            .and_then(|key| project.borrow().keyframe_time(key, position))
            .unwrap_or(position)
    })
}

fn select_time(context: &InspectorContext) -> Rc<dyn Fn(Time)> {
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let selected_item = context.selected_item.clone();
    Rc::new(move |time| {
        let position = selected_item
            .as_ref()
            .and_then(|key| project.borrow().keyframe_timeline_time(key, time))
            .unwrap_or(time);
        player_state::seek_time(&player_state, position);
    })
}

fn dispatch_action(context: &GraphActionContext, action: FrameGraphComponentAction) {
    assert_eq!(action.component, 0, "production keyframe graph is scalar");
    match action.action {
        FrameGraphAction::PlayheadChanged(time) => (context.select_time)(time),
        FrameGraphAction::KeysMoved(moves) => {
            for key_move in moves {
                let time = {
                    let project = context.project.borrow();
                    keyframe_model::canonical_keyframe_time(
                        &project,
                        context.selected_item.as_ref(),
                        key_move.time,
                    )
                };
                let Some(time) = time else {
                    continue;
                };
                (context.actions.update_point)(key_move.old_time, time, key_move.value);
            }
        }
        FrameGraphAction::KeysDeleted(times) => {
            for time in times {
                (context.actions.delete_at_time)(time);
            }
        }
        FrameGraphAction::KeyAdded(point) => {
            let time = {
                let project = context.project.borrow();
                keyframe_model::canonical_keyframe_time(
                    &project,
                    context.selected_item.as_ref(),
                    point.time,
                )
            };
            if let Some(time) = time {
                (context.actions.add_at_time)(time);
            }
        }
        FrameGraphAction::CopyRequested(times) => copy_keyframes(context, &times),
        FrameGraphAction::PasteRequested(time) => paste_keyframes(context, time),
        FrameGraphAction::TogglePlayback => (context.actions.toggle_playback)(),
        FrameGraphAction::EditFinished => {
            shrimply_project_document::project::finish_coalesced_edit();
        }
        FrameGraphAction::InterpolationRequested {
            owner_id,
            interpolation,
            ..
        } => {
            if let Some(set) = &context.actions.set_interpolation {
                set(owner_id, interpolation);
            }
        }
        FrameGraphAction::TextInterpolationRequested { owner_id, x, y } => {
            show_text_interpolation(context, owner_id, x, y);
        }
        FrameGraphAction::KeysChanged(_) | FrameGraphAction::KeysPasted(_) => {
            panic!("authoritative keyframe graph received a local-only mutation")
        }
    }
}

fn copy_keyframes(context: &GraphActionContext, selected: &[Time]) {
    let count = match &context.actions.clipboard {
        KeyframeClipboardActions::Local { copy, .. } => {
            let Some(mut clipboard) = copy(selected) else {
                KEYFRAME_CLIPBOARD.with(|stored| stored.replace(None));
                return;
            };
            let project = context.project.borrow();
            if !keyframe_model::normalize_clipboard_times(
                &project,
                context.selected_item.as_ref(),
                &mut clipboard,
            ) {
                KEYFRAME_CLIPBOARD.with(|stored| stored.replace(None));
                return;
            }
            drop(project);
            let count = clipboard.len();
            KEYFRAME_CLIPBOARD.with(|stored| stored.replace(Some(clipboard)));
            count
        }
        KeyframeClipboardActions::Managed { copy, .. } => {
            let Some(count) = copy(selected).filter(|count| *count > 0) else {
                return;
            };
            count
        }
    };
    let Some(area) = context.graph_area.borrow().clone() else {
        return;
    };
    area.display()
        .clipboard()
        .set_text(keyframe_model::KEYFRAME_CLIPBOARD_MARKER);
    show_count_toast(&area, count, false);
}

fn paste_keyframes(context: &GraphActionContext, time: Time) {
    let paste: Rc<dyn Fn() -> Option<usize>> = match &context.actions.clipboard {
        KeyframeClipboardActions::Local { paste, .. } => {
            let Some(clipboard) = KEYFRAME_CLIPBOARD.with(|stored| stored.borrow().clone()) else {
                return;
            };
            let Some(times) = keyframe_model::clipboard_paste_times(
                &context.project.borrow(),
                context.selected_item.as_ref(),
                &clipboard,
                time,
            ) else {
                return;
            };
            let paste = paste.clone();
            Rc::new(move || paste(&clipboard, &times).map(|times| times.len()))
        }
        KeyframeClipboardActions::Managed { paste, .. } => {
            let paste = paste.clone();
            Rc::new(move || paste(time))
        }
    };
    let Some(area) = context.graph_area.borrow().clone() else {
        return;
    };
    area.display()
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |result| {
            if result.ok().flatten().as_deref() != Some(keyframe_model::KEYFRAME_CLIPBOARD_MARKER) {
                return;
            }
            let Some(count) = paste() else {
                return;
            };
            show_count_toast(&area, count, true);
        });
}

fn show_count_toast(area: &gtk::GLArea, count: usize, pasted: bool) {
    if let Some(message) = shrimply_inspector_core::document::graph::confirmation(count, pasted) {
        shrimply_components_gtk::toast::show_confirmation_text_for_widget(area, &message);
    }
}

fn show_text_interpolation(context: &GraphActionContext, owner_id: Uuid, x: f64, y: f64) {
    let Some(actions) = &context.actions.text_interpolation else {
        return;
    };
    let Some(selected) = (actions.get)(owner_id) else {
        return;
    };
    let Some(area) = context.graph_area.borrow().clone() else {
        return;
    };
    let set = actions.set.clone();
    let popover = searchable_popover(
        tr!("Search interpolations").as_ref(),
        280,
        180,
        240,
        move |query| {
            TextInterpolation::ALL
                .into_iter()
                .filter(|mode| matches_query(mode.label(), query))
                .map(|mode| {
                    let set = set.clone();
                    SearchMenuItem::new(tr!(mode.label()).as_ref(), move || set(owner_id, mode))
                        .selected(mode == selected)
                        .tooltip(mode.tooltip())
                })
                .collect()
        },
    );
    popover.set_parent(&area);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| popover.unparent());
    popover.popup();
}
