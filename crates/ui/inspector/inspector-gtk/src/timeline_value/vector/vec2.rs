use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use glam::Vec2;

use crate::InspectedItem as SelectedItem;
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::timeline_value::*;
use crate::ui::Number2Picker;
use crate::{
    InspectorContext, keyframe_model,
    timeline_value::{
        ExpressionOutput, LayeredSections, evaluate_expression, expression_section, layered_control,
    },
};
use shrimply_project_document::project::{Project, Time};

use crate::keyframe_editor::{self, KeyframeEditorActions};

pub(crate) type VecGet =
    for<'a> fn(&'a Project, SelectedItem) -> Option<&'a TimelineValue<glam::Vec2>>;
pub(crate) type VecGetMut =
    for<'a> fn(&'a mut Project, SelectedItem) -> Option<&'a mut TimelineValue<glam::Vec2>>;

#[derive(Clone, Copy)]
pub(crate) enum VecAccess {
    ItemScoped {
        get: VecGet,
        get_mut: VecGetMut,
        value_id: Uuid,
    },
    ItemWithMutation {
        get: VecGet,
        get_mut: VecGetMut,
        value_id: Uuid,
        mutated: fn(&mut Project, SelectedItem),
    },
    Modifier {
        id: Uuid,
        value_id: Uuid,
    },
}
impl VecAccess {
    fn get(self, p: &Project, k: SelectedItem) -> Option<&TimelineValue<glam::Vec2>> {
        match self {
            Self::ItemScoped { get, value_id, .. }
            | Self::ItemWithMutation { get, value_id, .. } => {
                get(p, k.clone()).filter(|value| value.id == value_id)
            }
            Self::Modifier { id, value_id } => crate::modifiers::number2(
                p.video_item(&k)?.modifiers.iter().find(|m| m.id == id)?,
                value_id,
            ),
        }
    }
    fn get_mut(self, p: &mut Project, k: SelectedItem) -> Option<&mut TimelineValue<glam::Vec2>> {
        match self {
            Self::ItemScoped {
                get_mut, value_id, ..
            }
            | Self::ItemWithMutation {
                get_mut, value_id, ..
            } => get_mut(p, k.clone()).filter(|value| value.id == value_id),
            Self::Modifier { id, value_id } => crate::modifiers::number2_mut(
                p.video_item_mut(&k)?
                    .modifiers
                    .iter_mut()
                    .find(|m| m.id == id)?,
                value_id,
            ),
        }
    }

    fn mark_mutated(self, project: &mut Project, key: SelectedItem) {
        if let Self::ItemWithMutation { mutated, .. } = self {
            mutated(project, key);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct VecTarget {
    pub(crate) access: VecAccess,
    pub(crate) scope_id: Option<Uuid>,
    pub(crate) local_time: fn(&Project, SelectedItem, Time) -> Option<Time>,
    pub(crate) duration: fn(&Project, SelectedItem) -> Option<Time>,
    pub(crate) refresh: ProjectChange,
    pub(crate) commit_name: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct VecSpec {
    pub(crate) first_prefix: &'static str,
    pub(crate) second_prefix: &'static str,
    pub(crate) drag_step: f64,
    pub(crate) digits: usize,
    pub(crate) width_chars: i32,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
    pub(crate) unit_name: &'static str,
}

pub(crate) fn vec_control(
    label: &str,
    value: &TimelineValue<glam::Vec2>,
    context: &InspectorContext,
    target: VecTarget,
    spec: VecSpec,
) -> gtk::Widget {
    vec_control_with_lock(label, value, context, target, spec, false)
}

pub(crate) fn vec_control_with_lock(
    label: &str,
    value: &TimelineValue<glam::Vec2>,
    context: &InspectorContext,
    target: VecTarget,
    spec: VecSpec,
    lock: bool,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return layered_control(
            label,
            value,
            picker(value.fallback(), context, target, spec, lock),
            LayeredSections::default(),
            |_| {},
            |_| {},
        );
    };
    let position = player_state::snapshot(&context.player_state).position;
    let local_time = {
        let project = context.project.borrow();
        (target.local_time)(&project, key.clone(), position).unwrap_or(Time::ZERO)
    };
    let current =
        shrimply_inspector_core::timeline_value::vector::vec2::value_at(value, local_time);
    let layers = shrimply_inspector_core::LayeredState::from(value);
    let mut sections = LayeredSections::default();
    if layers.keyframes {
        let graph = super::speed_graph(value);
        let duration = {
            let project = context.project.borrow();
            (target.duration)(&project, key.clone()).unwrap_or(Time::ZERO)
        };
        let built = keyframe_editor::build(
            context,
            graph,
            (Time::ZERO, duration),
            format!("vec:{}:{:?}:{label}", target.commit_name, target.scope_id),
            keyframe_actions(context, key.clone(), target),
        );
        let project = context.project.clone();
        let graph_key = key.clone();
        keyframe_editor::connect_graph_refresh(
            context,
            "inspector vec keyframe graph refresh",
            &built,
            move || {
                target
                    .access
                    .get(&project.borrow(), graph_key.clone())
                    .map(super::speed_graph)
            },
        );
        sections.set_keyframe(built.widget);
    }
    if layers.expression {
        sections.push_expression(expression_editor(context, key.clone(), target, spec));
    }
    let keyframe_project = context.project.clone();
    let keyframe_player_state = context.player_state.clone();
    let keyframe_refresh = context.refresh.clone();
    let expression_project = context.project.clone();
    let expression_player_state = context.player_state.clone();
    let expression_refresh = context.refresh.clone();
    let keyframe_key = key.clone();
    let expression_key = key;
    layered_control(
        label,
        value,
        picker(current, context, target, spec, lock),
        sections,
        move |enabled| {
            if set_keyframes_enabled(
                &keyframe_project,
                &keyframe_player_state,
                keyframe_key.clone(),
                target,
                enabled,
            ) {
                keyframe_refresh();
            }
        },
        move |enabled| {
            if set_expression_enabled(
                &expression_project,
                &expression_player_state,
                expression_key.clone(),
                target,
                enabled,
            ) {
                expression_refresh();
            }
        },
    )
}

fn picker(
    value: Vec2,
    context: &InspectorContext,
    target: VecTarget,
    spec: VecSpec,
    lock: bool,
) -> gtk::Widget {
    let mut picker = Number2Picker::builder(value.x as f64, value.y as f64)
        .first_prefix(spec.first_prefix)
        .second_prefix(spec.second_prefix)
        .drag_step(spec.drag_step)
        .digits(spec.digits)
        .width_chars(spec.width_chars)
        .unit_name(spec.unit_name);
    if let Some(minimum) = spec.minimum {
        picker = picker.minimum(minimum);
    }
    if let Some(maximum) = spec.maximum {
        picker = picker.maximum(maximum);
    }
    if lock {
        picker = picker.enable_lock();
    }
    let Some(key) = context.selected_item.clone() else {
        return picker.build_with_handles().widget;
    };
    let first_project = context.project.clone();
    let first_player_state = context.player_state.clone();
    let second_project = context.project.clone();
    let second_player_state = context.player_state.clone();
    let first_commit_project = context.project.clone();
    let second_commit_project = context.project.clone();
    let first_commit_player_state = context.player_state.clone();
    let second_commit_player_state = context.player_state.clone();
    let first_key = key.clone();
    let second_key = key;
    picker
        .on_first_change(move |next| {
            update_component(
                &first_project,
                &first_player_state,
                first_key.clone(),
                target,
                0,
                next,
            );
        })
        .on_second_change(move |next| {
            update_component(
                &second_project,
                &second_player_state,
                second_key.clone(),
                target,
                1,
                next,
            );
        })
        .on_first_commit(move |_| {
            shrimply_project_document::project::commit_edit(
                &first_commit_project.borrow(),
                target.commit_name,
            );
            player_state::refresh_project(&first_commit_player_state, target.refresh);
        })
        .on_second_commit(move |_| {
            shrimply_project_document::project::commit_edit(
                &second_commit_project.borrow(),
                target.commit_name,
            );
            player_state::refresh_project(&second_commit_player_state, target.refresh);
        })
        .build_with_handles()
        .widget
}

fn update_component(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    component: usize,
    next: f64,
) {
    if !next.is_finite() {
        return;
    }
    let position = player_state::snapshot(player_state).position;
    let mut project = project.borrow_mut();
    let Some(evaluation_time) = (target.local_time)(&project, key.clone(), position) else {
        return;
    };
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return;
    };
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::set_component(
        value,
        evaluation_time,
        keyframe_time,
        component,
        next,
    ) {
        return;
    }
    target.access.mark_mutated(&mut project, key);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
}

fn set_keyframes_enabled(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    enabled: bool,
) -> bool {
    let position = player_state::snapshot(player_state).position;
    let mut project = project.borrow_mut();
    let Some(evaluation_time) = (target.local_time)(&project, key.clone(), position) else {
        return false;
    };
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return false;
    };
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::set_keyframes_enabled(
        value,
        evaluation_time,
        keyframe_time,
        enabled,
    ) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, target.refresh);
    true
}

fn set_expression_enabled(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    enabled: bool,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    let changed = shrimply_inspector_core::timeline_value::vector::vec2::set_expression_enabled(
        value, enabled,
    );
    if !changed {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, target.refresh);
    true
}

fn keyframe_actions(
    context: &InspectorContext,
    key: SelectedItem,
    target: VecTarget,
) -> KeyframeEditorActions {
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let add_project = project.clone();
    let add_player = player_state.clone();
    let delete_project = project.clone();
    let delete_player = player_state.clone();
    let point_project = project.clone();
    let point_player = player_state.clone();
    let copy_project = project.clone();
    let paste_project = project.clone();
    let paste_player = player_state.clone();
    let interpolation_project = project;
    let interpolation_player = player_state.clone();
    let playback_player = player_state;
    let refresh_add = context.refresh.clone();
    let refresh_delete = context.refresh.clone();
    let add_key = key.clone();
    let delete_key = key.clone();
    let point_key = key.clone();
    let copy_key = key.clone();
    let paste_key = key.clone();
    let interpolation_key = key;
    KeyframeEditorActions {
        add_at_time: Rc::new(move |time| {
            if add_keyframe_at_time(&add_project, &add_player, add_key.clone(), target, time) {
                refresh_add();
            }
        }),
        delete_at_time: Rc::new(move |time| {
            if delete_keyframe_at_time(
                &delete_project,
                &delete_player,
                delete_key.clone(),
                target,
                time,
            ) {
                refresh_delete();
            }
        }),
        update_point: Rc::new(move |old_time, time, value| {
            update_keyframe_point(
                &point_project,
                &point_player,
                point_key.clone(),
                target,
                old_time,
                time,
                value,
            );
        }),
        clipboard: crate::keyframe_editor::KeyframeClipboardActions::Local {
            copy: Rc::new(move |times| {
                target
                    .access
                    .get(&copy_project.borrow(), copy_key.clone())
                    .and_then(|value| {
                        shrimply_inspector_core::timeline_value::vector::vec2::copy_keyframes(
                            value, times,
                        )
                    })
            }),
            paste: Rc::new(move |clipboard, time| {
                let mut project = paste_project.borrow_mut();
                let value = target.access.get_mut(&mut project, paste_key.clone())?;
                let times = shrimply_inspector_core::timeline_value::vector::vec2::paste_keyframes(
                    value, clipboard, time,
                )?;
                target.access.mark_mutated(&mut project, paste_key.clone());
                shrimply_project_document::project::commit_edit(&project, target.commit_name);
                drop(project);
                player_state::refresh_project(&paste_player, target.refresh);
                Some(times)
            }),
        },
        set_interpolation: Some(Rc::new(move |owner_id, interpolation| {
            set_keyframe_interpolation(
                &interpolation_project,
                &interpolation_player,
                interpolation_key.clone(),
                target,
                owner_id,
                interpolation,
            );
        })),
        text_interpolation: None,
        toggle_playback: Rc::new(move || player_state::toggle_playing(&playback_player)),
    }
}

fn add_keyframe_at_time(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::add_keyframe(value, time) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, target.refresh);
    true
}

fn delete_keyframe_at_time(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::delete_keyframe(value, time) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, target.refresh);
    true
}

fn update_keyframe_point(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    old_time: Time,
    time: Time,
    _value: f64,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::move_keyframes(
        value,
        &[(old_time, time)],
    ) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    let mut refresh = target.refresh;
    refresh.inspector = false;
    player_state::refresh_project(player_state, refresh);
    true
}

fn set_keyframe_interpolation(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::set_interpolation(
        value,
        owner_id,
        interpolation,
    ) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
    true
}

fn expression_editor(
    context: &InspectorContext,
    key: SelectedItem,
    target: VecTarget,
    spec: VecSpec,
) -> gtk::Widget {
    let source = {
        let project = context.project.borrow();
        target
            .access
            .get(&project, key.clone())
            .and_then(|value| value.expression_source().map(str::to_string))
    };
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let editor_key = key.clone();
    let output_key = key;
    expression_section(
        context,
        "inspector vec expression output",
        move |refresh| {
            crate::rhai_editor::editor(
                source,
                crate::rhai_editor::ExpressionValue::Vec2,
                move |source| {
                    update_expression_source(
                        &project,
                        &player_state,
                        editor_key.clone(),
                        target,
                        source,
                    );
                    refresh();
                },
            )
        },
        move |project, position, audio, cache| {
            let value = target.access.get(project, output_key.clone())?;
            let outcome = evaluate_expression(project, &output_key, position, audio, cache, value)?;
            Some(ExpressionOutput {
                value: shrimply_inspector_core::timeline_value::vector::vec2::format_value(
                    outcome.value,
                    spec.first_prefix,
                    spec.second_prefix,
                    spec.digits,
                    spec.unit_name,
                ),
                error: outcome.error,
            })
        },
    )
}

fn update_expression_source(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: VecTarget,
    source: String,
) {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return;
    };
    if !shrimply_inspector_core::timeline_value::vector::vec2::set_expression_source(value, source)
    {
        return;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    let mut refresh = target.refresh;
    refresh.inspector = false;
    player_state::refresh_project(player_state, refresh);
}
