use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use shrimply_project_document::project::{LayerVisibility, Project, Time, VideoItemContent};
use shrimply_property_model::timeline_value::*;
use uuid::Uuid;

use crate::InspectedItem as SelectedItem;
use crate::keyframe_editor::{self, KeyframeEditorActions, KeyframeGraph};
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::timeline_value::{
    ExpressionOutput, LayeredSections, evaluate_expression, expression_section, layered_control,
};
use crate::{InspectorContext, keyframe_model};

pub(crate) type BoolGet =
    for<'a> fn(&'a Project, SelectedItem) -> Option<&'a TimelineValue<TimelineBool>>;
pub(crate) type BoolGetMut =
    for<'a> fn(&'a mut Project, SelectedItem) -> Option<&'a mut TimelineValue<TimelineBool>>;

#[derive(Clone)]
pub(crate) enum BoolTarget {
    ItemValue {
        get: BoolGet,
        get_mut: BoolGetMut,
        scope: &'static str,
        mutated: fn(&mut Project, SelectedItem),
    },
    ItemVisibility,
    Layer {
        id: Uuid,
        path: String,
    },
}

impl BoolTarget {
    fn get<'a>(
        &self,
        project: &'a Project,
        key: SelectedItem,
    ) -> Option<&'a TimelineValue<TimelineBool>> {
        let item = project.video_item(&key)?;
        match self {
            Self::ItemValue { get, .. } => get(project, key),
            Self::ItemVisibility => Some(&item.visibility),
            Self::Layer { id, path, .. } => {
                let VideoItemContent::LayeredImage(image) = &item.content else {
                    return None;
                };
                image
                    .layers
                    .iter()
                    .find(|layer| layer.id == *id || layer.path == *path)?
                    .visibility
                    .as_ref()
            }
        }
    }

    fn get_mut<'a>(
        &self,
        project: &'a mut Project,
        key: SelectedItem,
        source_value: bool,
    ) -> Option<&'a mut TimelineValue<TimelineBool>> {
        match self {
            Self::ItemValue { get_mut, .. } => get_mut(project, key),
            Self::ItemVisibility => Some(&mut project.video_item_mut(&key)?.visibility),
            Self::Layer { id, path } => {
                let item = project.video_item_mut(&key)?;
                let VideoItemContent::LayeredImage(image) = &mut item.content else {
                    return None;
                };
                let index = image
                    .layers
                    .iter()
                    .position(|layer| layer.id == *id || layer.path == *path)
                    .unwrap_or_else(|| {
                        image.layers.push(LayerVisibility {
                            id: *id,
                            path: path.clone(),
                            visibility: None,
                        });
                        image.layers.len() - 1
                    });
                Some(image.layers[index].visibility.get_or_insert_with(|| {
                    TimelineValue::<TimelineBool>::new_const(source_value.into())
                }))
            }
        }
    }

    fn scope(&self) -> String {
        match self {
            Self::ItemValue { scope, .. } => (*scope).to_string(),
            Self::ItemVisibility => "item".to_string(),
            Self::Layer { path, .. } => format!("layered-image:{path}"),
        }
    }

    fn did_mutate(&self, project: &mut Project, key: SelectedItem) {
        if let Self::ItemValue { mutated, .. } = self {
            mutated(project, key);
        }
    }
}

pub(crate) fn bool_control(
    label: &str,
    value: &TimelineValue<TimelineBool>,
    source_value: bool,
    context: &InspectorContext,
    target: BoolTarget,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return gtk::Switch::builder()
            .active(value.fallback().get())
            .build()
            .upcast();
    };
    let position = player_state::snapshot(&context.player_state).position;
    let local_time =
        crate::video::visual_local_time(&context.project.borrow(), key.clone(), position)
            .unwrap_or(Time::ZERO);
    let current = value.value_at(local_time).get();
    let keyframes_enabled = matches!(value.base, TimelineBase::Keyframes(_));
    let expression_enabled = value
        .expression
        .as_ref()
        .is_some_and(|expression| expression.enabled);
    let toggle = gtk::Switch::builder()
        .active(current)
        .valign(gtk::Align::Center)
        .build();
    let syncing_toggle = Rc::new(Cell::new(false));
    let project = context.project.clone();
    let player = context.player_state.clone();
    let target_for_toggle = target.clone();
    let syncing_toggle_for_signal = syncing_toggle.clone();
    let toggle_key = key.clone();
    toggle.connect_active_notify(move |toggle| {
        if syncing_toggle_for_signal.get() {
            return;
        }
        update_value(
            &project,
            &player,
            toggle_key.clone(),
            &target_for_toggle,
            source_value,
            toggle.is_active(),
        );
    });
    let toggle_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    toggle_row.set_hexpand(true);
    toggle_row.set_halign(gtk::Align::End);
    toggle_row.append(&toggle);

    let mut sections = LayeredSections::default();
    let mut graph_widget = None;
    if keyframes_enabled {
        let graph = keyframe_model::step_graph(value);
        let visible_area = bool_time_range(&context.project.borrow(), key.clone())
            .unwrap_or((Time::ZERO, Time::ZERO));
        let built = keyframe_editor::build(
            context,
            graph,
            visible_area,
            format!("bool:{}", target.scope()),
            actions(context, key.clone(), target.clone(), source_value),
        );
        graph_widget = Some(built.update_graph.clone());
        sections.set_keyframe(built.widget);
    }
    if expression_enabled {
        sections.push_expression(expression_editor(
            context,
            key.clone(),
            target.clone(),
            source_value,
        ));
    }

    connect_display_refresh(
        context,
        key.clone(),
        target.clone(),
        source_value,
        BoolDisplay {
            toggle: toggle.clone(),
            syncing_toggle,
            graph: graph_widget,
        },
    );

    let project = context.project.clone();
    let player = context.player_state.clone();
    let refresh = context.refresh.clone();
    let expression_project = context.project.clone();
    let expression_player = context.player_state.clone();
    let expression_refresh = context.refresh.clone();
    let expression_target = target.clone();
    let keyframe_key = key.clone();
    let expression_key = key;
    layered_control(
        label,
        value,
        toggle_row.upcast(),
        sections,
        move |enabled| {
            if toggle_keyframes(
                &project,
                &player,
                keyframe_key.clone(),
                &target,
                source_value,
                enabled,
            ) {
                refresh();
            }
        },
        move |enabled| {
            if set_expression_enabled(
                &expression_project,
                &expression_player,
                expression_key.clone(),
                &expression_target,
                source_value,
                enabled,
            ) {
                expression_refresh();
            }
        },
    )
}

struct BoolDisplay {
    toggle: gtk::Switch,
    syncing_toggle: Rc<Cell<bool>>,
    graph: Option<Rc<dyn Fn(KeyframeGraph)>>,
}

fn connect_display_refresh(
    context: &InspectorContext,
    key: SelectedItem,
    target: BoolTarget,
    source_value: bool,
    display: BoolDisplay,
) {
    let project = context.project.clone();
    let player = context.player_state.clone();
    let toggle = display.toggle.downgrade();
    let graph = display.graph;
    let syncing_toggle = display.syncing_toggle;
    let alive = Rc::downgrade(&context.listener_scope);
    let alive_for_prune = alive.clone();
    player_state::connect_while_alive_named(
        &context.player_state,
        "inspector bool display refresh",
        move || alive_for_prune.upgrade().is_some(),
        move |event| {
            if !matches!(
                event,
                player_state::PlayerEvent::State(_) | player_state::PlayerEvent::Project(_)
            ) || alive.upgrade().is_none()
            {
                return;
            }
            let Some(toggle) = toggle.upgrade() else {
                return;
            };
            let position = player_state::snapshot(&player).position;
            let project = project.borrow();
            let Some(local_time) = crate::video::visual_local_time(&project, key.clone(), position)
            else {
                return;
            };
            let current = target
                .get(&project, key.clone())
                .map_or(source_value, |value| {
                    if let Some(update_graph) = &graph
                        && matches!(value.base, TimelineBase::Keyframes(_))
                    {
                        update_graph(keyframe_model::step_graph(value));
                    }
                    value.value_at(local_time).get()
                });
            if toggle.is_active() != current {
                syncing_toggle.set(true);
                toggle.set_active(current);
                syncing_toggle.set(false);
            }
        },
    );
}

fn expression_editor(
    context: &InspectorContext,
    key: SelectedItem,
    target: BoolTarget,
    source_value: bool,
) -> gtk::Widget {
    let source = target
        .get(&context.project.borrow(), key.clone())
        .and_then(|value| value.expression_source().map(str::to_string));
    let project = context.project.clone();
    let player = context.player_state.clone();
    let editor_target = target.clone();
    let editor_key = key.clone();
    let output_target = target;
    let output_key = key;
    expression_section(
        context,
        "inspector bool expression feedback",
        move |refresh| {
            crate::rhai_editor::editor(
                source,
                crate::rhai_editor::ExpressionValue::Bool,
                move |source| {
                    update_expression_source(
                        &project,
                        &player,
                        editor_key.clone(),
                        &editor_target,
                        source_value,
                        source,
                    );
                    refresh();
                },
            )
        },
        move |project, position, audio, cache| {
            let value = output_target.get(project, output_key.clone())?;
            let outcome = evaluate_expression(project, &output_key, position, audio, cache, value)?;
            Some(ExpressionOutput {
                value: outcome.value.get().to_string(),
                error: outcome.error,
            })
        },
    )
}

fn actions(
    context: &InspectorContext,
    key: SelectedItem,
    target: BoolTarget,
    source_value: bool,
) -> KeyframeEditorActions {
    let project = context.project.clone();
    let player = context.player_state.clone();
    let refresh_add = context.refresh.clone();
    let refresh_delete = context.refresh.clone();
    KeyframeEditorActions {
        add_at_time: {
            let key = key.clone();
            let project = project.clone();
            let player = player.clone();
            let target = target.clone();
            Rc::new(move |time| {
                add_key(&project, &player, key.clone(), &target, source_value, time);
                refresh_add();
            })
        },
        delete_at_time: {
            let key = key.clone();
            let project = project.clone();
            let player = player.clone();
            let target = target.clone();
            Rc::new(move |time| {
                delete_key(&project, &player, key.clone(), &target, source_value, time);
                refresh_delete();
            })
        },
        update_point: {
            let key = key.clone();
            let project = project.clone();
            let player = player.clone();
            let target = target.clone();
            Rc::new(move |old, time, _| {
                move_key(
                    &project,
                    &player,
                    key.clone(),
                    &target,
                    source_value,
                    old,
                    time,
                )
            })
        },
        clipboard: crate::keyframe_editor::KeyframeClipboardActions::Local {
            copy: {
                let key = key.clone();
                let project = project.clone();
                let target = target.clone();
                Rc::new(move |times| {
                    target
                        .get(&project.borrow(), key.clone())
                        .and_then(|value| keyframe_model::copy_keyframes(value, times))
                })
            },
            paste: {
                let key = key.clone();
                let project = project.clone();
                let player = player.clone();
                let target = target.clone();
                Rc::new(move |clipboard, time| {
                    let mut project = project.borrow_mut();
                    let value = target.get_mut(&mut project, key.clone(), source_value)?;
                    let times = keyframe_model::paste_keyframes(value, clipboard, time)?;
                    target.did_mutate(&mut project, key.clone());
                    shrimply_project_document::project::commit_edit(
                        &project,
                        "paste-bool-keyframes",
                    );
                    drop(project);
                    player_state::refresh_project(
                        &player,
                        ProjectChange {
                            video: true,
                            inspector: true,
                            ..Default::default()
                        },
                    );
                    Some(times)
                })
            },
        },
        set_interpolation: None,
        text_interpolation: None,
        toggle_playback: Rc::new(move || player_state::toggle_playing(&player)),
    }
}

fn update_value(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source_value: bool,
    next: bool,
) {
    let position = player_state::snapshot(player).position;
    let mut project = project.borrow_mut();
    let Some(time) = project.keyframe_time(&key, position) else {
        return;
    };
    let frame_step = keyframe_editor::project_frame_step(&project, Some(&key));
    let Some(value) = target.get_mut(&mut project, key.clone(), source_value) else {
        return;
    };
    keyframe_model::set_discrete_value(value, time, next.into(), frame_step);
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, "visual-bool");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

fn toggle_keyframes(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source_value: bool,
    enabled: bool,
) -> bool {
    let position = player_state::snapshot(player).position;
    let mut project = project.borrow_mut();
    let evaluation_time = bool_local_time(&project, key.clone(), position).unwrap_or(Time::ZERO);
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return false;
    };
    let Some(value) = target.get_mut(&mut project, key.clone(), source_value) else {
        return false;
    };
    let current = value.value_at(evaluation_time);
    if !keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled) {
        return false;
    }
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, "visual-bool-keyframes");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    true
}

fn set_expression_enabled(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source_value: bool,
    enabled: bool,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.get_mut(&mut project, key.clone(), source_value) else {
        return false;
    };
    let changed = keyframe_model::set_expression_enabled(
        value,
        enabled,
        shrimply_inspector_core::timeline_value::SCALAR_EXPRESSION_DEFAULT,
    );
    if !changed {
        return false;
    }
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, "visual-bool-expression");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    true
}

fn update_expression_source(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source_value: bool,
    source: String,
) {
    let mut project = project.borrow_mut();
    let Some(value) = target.get_mut(&mut project, key.clone(), source_value) else {
        return;
    };
    let Some(expression) = &mut value.expression else {
        return;
    };
    if expression.source == source {
        return;
    }
    expression.source = source;
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, "visual-bool-expression");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: false,
            ..Default::default()
        },
    );
}

fn add_key(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source: bool,
    time: Time,
) {
    let mut project = project.borrow_mut();
    let step = keyframe_editor::project_frame_step(&project, Some(&key));
    let Some(value) = target.get_mut(&mut project, key.clone(), source) else {
        return;
    };
    if !keyframe_model::add_discrete_keyframe(value, time, step) {
        return;
    }
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, "add-bool-keyframe");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

fn delete_key(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source: bool,
    time: Time,
) {
    let mut project = project.borrow_mut();
    let step = keyframe_editor::project_frame_step(&project, Some(&key));
    let Some(value) = target.get_mut(&mut project, key.clone(), source) else {
        return;
    };
    if !keyframe_model::delete_discrete_keyframe(value, time, step) {
        return;
    }
    target.did_mutate(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, "delete-bool-keyframe");
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

fn move_key(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: &BoolTarget,
    source: bool,
    old: Time,
    time: Time,
) {
    let mut project = project.borrow_mut();
    let Some(value) = target.get_mut(&mut project, key.clone(), source) else {
        return;
    };
    let changed = keyframe_model::move_discrete_keyframe(value, old, time);
    if changed {
        target.did_mutate(&mut project, key);
        shrimply_project_document::project::commit_coalesced_edit(&project, "move-bool-keyframe");
    }
    drop(project);
    player_state::refresh_project(
        player,
        keyframe_model::live_refresh(ProjectChange {
            video: true,
            ..Default::default()
        }),
    );
}

fn bool_local_time(project: &Project, key: SelectedItem, position: Time) -> Option<Time> {
    crate::video::visual_local_time(project, key, position)
}

fn bool_time_range(project: &Project, key: SelectedItem) -> Option<(Time, Time)> {
    crate::video::visual_visible_area(project, key)
}
