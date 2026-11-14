use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
};

use crate::InspectedItem as SelectedItem;
use glam::Vec3;
use shrimply_inspector_core::gaussian_3d::{VECTOR_COMMIT, VECTOR_EXPRESSION_COMMIT};
use shrimply_inspector_core::timeline_value::vector::vec3 as shared;
use shrimply_project_document::project::{Project, Time};
use shrimply_property_model::timeline_value::{Interpolation, TimelineBase, TimelineValue};
use uuid::Uuid;

use crate::{
    InspectorContext,
    keyframe_editor::{self, KeyframeEditorActions},
    keyframe_model,
    player_state::{self, ProjectChange, SharedPlayerState},
    timeline_value::{ExpressionOutput, LayeredSections, evaluate_expression, expression_section},
    ui::{Number3Picker, NumberPickerHandle},
};

type ItemGet = for<'a> fn(&'a Project, SelectedItem) -> Option<&'a TimelineValue<Vec3>>;
type ItemGetMut = for<'a> fn(&'a mut Project, SelectedItem) -> Option<&'a mut TimelineValue<Vec3>>;

#[derive(Clone, Copy)]
enum Vec3Access {
    Scene3dScoped { value_id: Uuid },
    Item { get: ItemGet, get_mut: ItemGetMut },
}

#[derive(Clone, Copy)]
pub(crate) struct Vec3Target {
    access: Vec3Access,
    timeline_id: Option<Uuid>,
    commit_name: &'static str,
    expression_commit_name: &'static str,
    minimum: Option<f64>,
    degrees: bool,
    lock: bool,
}

pub(crate) struct Vec3TargetBuilder(Vec3Target);

impl Vec3Target {
    pub(crate) fn item_builder(get: ItemGet, get_mut: ItemGetMut) -> Vec3TargetBuilder {
        Vec3TargetBuilder(Self {
            access: Vec3Access::Item { get, get_mut },
            timeline_id: None,
            commit_name: VECTOR_COMMIT,
            expression_commit_name: VECTOR_EXPRESSION_COMMIT,
            minimum: None,
            degrees: false,
            lock: false,
        })
    }

    pub(crate) fn scene_builder(value_id: Uuid) -> Vec3TargetBuilder {
        Vec3TargetBuilder(Self {
            access: Vec3Access::Scene3dScoped { value_id },
            timeline_id: Some(value_id),
            commit_name: VECTOR_COMMIT,
            expression_commit_name: VECTOR_EXPRESSION_COMMIT,
            minimum: None,
            degrees: false,
            lock: false,
        })
    }

    fn value(self, project: &Project, key: SelectedItem) -> Option<&TimelineValue<Vec3>> {
        let value = match self.access {
            Vec3Access::Scene3dScoped { value_id } => selected_scene(project, key.clone())
                .and_then(|scene| shrimply_inspector_core::scene_3d::vector3(scene, value_id)),
            Vec3Access::Item { get, .. } => get(project, key.clone()),
        }?;
        if self
            .timeline_id
            .is_some_and(|timeline_id| value.id != timeline_id)
        {
            return None;
        }
        Some(value)
    }

    fn value_mut(
        self,
        project: &mut Project,
        key: SelectedItem,
    ) -> Option<&mut TimelineValue<Vec3>> {
        let timeline_id = self.timeline_id;
        let value = match self.access {
            Vec3Access::Scene3dScoped { value_id } => selected_scene_mut(project, key.clone())
                .and_then(|scene| shrimply_inspector_core::scene_3d::vector3_mut(scene, value_id)),
            Vec3Access::Item { get_mut, .. } => get_mut(project, key.clone()),
        }?;
        if timeline_id.is_some_and(|timeline_id| value.id != timeline_id) {
            return None;
        }
        Some(value)
    }

    pub(crate) fn presentation(
        mut self,
        control: &shrimply_inspector_core::InspectorControl,
        commit_name: &'static str,
        expression_commit_name: &'static str,
    ) -> Self {
        assert_eq!(control.commit_name, commit_name);
        assert_eq!(control.keyframe_commit_name, commit_name);
        assert_eq!(control.expression_commit_name, expression_commit_name);
        self.timeline_id = Some(
            control
                .timeline_id
                .expect("shared Vec3 presentation must identify its timeline"),
        );
        self.commit_name = commit_name;
        self.expression_commit_name = expression_commit_name;
        self
    }
}

impl Vec3TargetBuilder {
    pub(crate) fn degrees(mut self) -> Self {
        self.0.degrees = true;
        self
    }

    pub(crate) fn minimum(mut self, minimum: f64) -> Self {
        self.0.minimum = Some(minimum);
        self
    }

    pub(crate) fn lock(mut self) -> Self {
        self.0.lock = true;
        self
    }

    pub(crate) fn build(self) -> Vec3Target {
        self.0
    }
}

pub(crate) fn control(
    label: &str,
    value: &TimelineValue<Vec3>,
    context: &InspectorContext,
    target: Vec3Target,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return crate::timeline_value::layered_control(
            label,
            value,
            picker(value.fallback(), context, target),
            LayeredSections::default(),
            |_| {},
            |_| {},
        );
    };
    let time = local_time(context, key.clone()).unwrap_or(Time::ZERO);
    let current = shared::value_at(value, time);
    let mut sections = LayeredSections::default();
    if matches!(value.base, TimelineBase::Keyframes(_)) {
        let built = keyframe_editor::build(
            context,
            super::speed_graph(value),
            visible_area(&context.project.borrow(), key.clone())
                .unwrap_or((Time::ZERO, Time::ZERO)),
            format!("vec3:{}:{label}", value.id),
            actions(context, key.clone(), target),
        );
        let project = context.project.clone();
        let graph_key = key.clone();
        keyframe_editor::connect_graph_refresh(
            context,
            "inspector vec3 graph refresh",
            &built,
            move || {
                target
                    .value(&project.borrow(), graph_key.clone())
                    .map(super::speed_graph)
            },
        );
        sections.set_keyframe(built.widget);
    }
    if value.expression.as_ref().is_some_and(|value| value.enabled) {
        sections.push_expression(expression_editor(context, key.clone(), target));
    }

    let keyframe_project = context.project.clone();
    let keyframe_player = context.player_state.clone();
    let keyframe_refresh = context.refresh.clone();
    let expression_project = context.project.clone();
    let expression_player = context.player_state.clone();
    let expression_refresh = context.refresh.clone();
    let keyframe_key = key.clone();
    let expression_key = key;
    crate::timeline_value::layered_control(
        label,
        value,
        picker(current, context, target),
        sections,
        move |enabled| {
            if toggle_keyframes(
                &keyframe_project,
                &keyframe_player,
                keyframe_key.clone(),
                target,
                enabled,
            ) {
                keyframe_refresh();
            }
        },
        move |enabled| {
            if toggle_expression(
                &expression_project,
                &expression_player,
                expression_key.clone(),
                target,
                enabled,
            ) {
                expression_refresh();
            }
        },
    )
}

fn picker(value: Vec3, context: &InspectorContext, target: Vec3Target) -> gtk::Widget {
    let mut picker = Number3Picker::builder(value.x as f64, value.y as f64, value.z as f64)
        .prefixes(["X", "Y", "Z"])
        .drag_step(if target.degrees { 1.0 } else { 0.1 })
        .digits(2)
        .width_chars(5);
    if target.degrees {
        picker = picker.unit_name("°");
    }
    if let Some(minimum) = target.minimum {
        picker = picker.minimum(minimum);
    }
    if target.lock {
        picker = picker.enable_lock();
    }
    let Some(key) = context.selected_item.clone() else {
        return picker.build_with_handles().widget;
    };
    for axis in 0..shared::COMPONENT_COUNT {
        let project = context.project.clone();
        let player = context.player_state.clone();
        let update_key = key.clone();
        picker = picker.on_change(axis, move |next| {
            update_component(&project, &player, update_key.clone(), target, axis, next)
        });
        let project = context.project.clone();
        let player = context.player_state.clone();
        picker = picker.on_commit(axis, move |_| {
            shrimply_project_document::project::commit_edit(&project.borrow(), target.commit_name);
            player_state::refresh_project(
                &player,
                ProjectChange {
                    video: true,
                    ..Default::default()
                },
            );
        });
    }
    let parts = picker.build_with_handles();
    connect_display(context, key.clone(), target, parts.handles);
    parts.widget
}

fn update_component(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    axis: usize,
    next: f64,
) {
    if !next.is_finite() {
        return;
    }
    let position = player_state::snapshot(player).position;
    let mut project = project.borrow_mut();
    let Some(evaluation_time) = crate::video::visual_local_time(&project, key.clone(), position)
    else {
        return;
    };
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return;
    };
    let Some(value) = target.value_mut(&mut project, key.clone()) else {
        return;
    };
    if shared::set_component(
        value,
        evaluation_time,
        keyframe_time,
        axis,
        next,
        target.minimum,
    ) {
        drop(project);
        player_state::refresh_project(
            player,
            keyframe_model::live_refresh(ProjectChange {
                video: true,
                ..Default::default()
            }),
        );
    }
}

fn connect_display(
    context: &InspectorContext,
    key: SelectedItem,
    target: Vec3Target,
    handles: [NumberPickerHandle; 3],
) {
    let project = context.project.clone();
    let player = context.player_state.clone();
    let handles = handles.map(|handle| handle.downgrade());
    let alive = Rc::downgrade(&context.listener_scope);
    player_state::connect_while_alive_named(
        &context.player_state,
        "inspector vec3 display",
        move || alive.upgrade().is_some(),
        move |event| {
            if !matches!(event, player_state::PlayerEvent::State(_)) {
                return;
            }
            let snapshot = player_state::snapshot(&player);
            let position = snapshot.position;
            let project = project.borrow();
            let Some(time) = crate::video::visual_local_time(&project, key.clone(), position)
            else {
                return;
            };
            let Some(value) = target
                .value(&project, key.clone())
                .map(|value| shared::value_at(value, time))
            else {
                return;
            };
            for (axis, handle) in handles.iter().enumerate() {
                if let Some(handle) = handle.upgrade() {
                    handle.set_f64(value[axis] as f64);
                }
            }
        },
    );
}

fn toggle_keyframes(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    enabled: bool,
) -> bool {
    let position = player_state::snapshot(player).position;
    let mut project = project.borrow_mut();
    let Some(evaluation_time) = crate::video::visual_local_time(&project, key.clone(), position)
    else {
        return false;
    };
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return false;
    };
    let Some(value) = target.value_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shared::set_keyframes_enabled(value, evaluation_time, keyframe_time, enabled) {
        return false;
    }
    commit_refresh(project, player, true, target.commit_name);
    true
}

fn toggle_expression(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    enabled: bool,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.value_mut(&mut project, key.clone()) else {
        return false;
    };
    let changed = shared::set_expression_enabled(value, enabled);
    if changed {
        commit_refresh(project, player, true, target.expression_commit_name);
    }
    changed
}

fn expression_editor(
    context: &InspectorContext,
    key: SelectedItem,
    target: Vec3Target,
) -> gtk::Widget {
    let source = target
        .value(&context.project.borrow(), key.clone())
        .and_then(|value| value.expression_source().map(str::to_string));
    let project = context.project.clone();
    let player = context.player_state.clone();
    let editor_key = key.clone();
    let output_key = key;
    expression_section(
        context,
        "inspector vec3 expression output",
        move |refresh| {
            crate::rhai_editor::editor(
                source,
                crate::rhai_editor::ExpressionValue::Vec3,
                move |source| {
                    let mut project = project.borrow_mut();
                    let Some(value) = target.value_mut(&mut project, editor_key.clone()) else {
                        return;
                    };
                    if !shared::set_expression_source(value, source) {
                        return;
                    }
                    shrimply_project_document::project::commit_coalesced_edit(
                        &project,
                        target.expression_commit_name,
                    );
                    drop(project);
                    player_state::refresh_project(
                        &player,
                        ProjectChange {
                            video: true,
                            ..Default::default()
                        },
                    );
                    refresh();
                },
            )
        },
        move |project, position, audio, cache| {
            let value = target.value(project, output_key.clone())?;
            let outcome = evaluate_expression(project, &output_key, position, audio, cache, value)?;
            Some(ExpressionOutput {
                value: shared::format_value(outcome.value, ["X", "Y", "Z"], 2, ""),
                error: outcome.error,
            })
        },
    )
}

fn actions(
    context: &InspectorContext,
    key: SelectedItem,
    target: Vec3Target,
) -> KeyframeEditorActions {
    let project = context.project.clone();
    let player = context.player_state.clone();
    KeyframeEditorActions {
        add_at_time: mutation(&project, &player, key.clone(), target, add_key),
        delete_at_time: mutation(&project, &player, key.clone(), target, delete_key),
        update_point: {
            let key = key.clone();
            let project = project.clone();
            let player = player.clone();
            Rc::new(move |old, time, _| {
                update_point(&project, &player, key.clone(), target, old, time)
            })
        },
        clipboard: crate::keyframe_editor::KeyframeClipboardActions::Local {
            copy: {
                let key = key.clone();
                let project = project.clone();
                Rc::new(move |times| {
                    target
                        .value(&project.borrow(), key.clone())
                        .and_then(|value| shared::copy_keyframes(value, times))
                })
            },
            paste: {
                let key = key.clone();
                let project = project.clone();
                let player = player.clone();
                Rc::new(move |clipboard, time| {
                    let mut project = project.borrow_mut();
                    let value = target.value_mut(&mut project, key.clone())?;
                    let times = shared::paste_keyframes(value, clipboard, time)?;
                    commit_refresh(project, &player, true, target.commit_name);
                    Some(times)
                })
            },
        },
        set_interpolation: Some({
            let key = key.clone();
            let project = project.clone();
            let player = player.clone();
            Rc::new(move |owner_id, interpolation| {
                set_interpolation(
                    &project,
                    &player,
                    key.clone(),
                    target,
                    owner_id,
                    interpolation,
                )
            })
        }),
        text_interpolation: None,
        toggle_playback: Rc::new(move || player_state::toggle_playing(&player)),
    }
}

fn mutation(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    mutate: fn(&mut TimelineValue<Vec3>, Time, Time) -> bool,
) -> Rc<dyn Fn(Time)> {
    let project = project.clone();
    let player = player.clone();
    Rc::new(move |time| {
        let mut project = project.borrow_mut();
        let frame_step = keyframe_editor::project_frame_step(&project, Some(&key));
        let Some(value) = target.value_mut(&mut project, key.clone()) else {
            return;
        };
        mutate(value, time, frame_step);
        commit_refresh(project, &player, true, target.commit_name);
    })
}

fn add_key(value: &mut TimelineValue<Vec3>, time: Time, _: Time) -> bool {
    shared::add_keyframe(value, time)
}

fn delete_key(value: &mut TimelineValue<Vec3>, time: Time, frame_step: Time) -> bool {
    shared::delete_keyframe(value, time, frame_step)
}

fn update_point(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    old: Time,
    time: Time,
) {
    let mut project = project.borrow_mut();
    let Some(value) = target.value_mut(&mut project, key.clone()) else {
        return;
    };
    if !shared::move_keyframes(value, &[(old, time)]) {
        return;
    }
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(
        player,
        keyframe_model::live_refresh(ProjectChange {
            video: true,
            ..Default::default()
        }),
    );
}

fn set_interpolation(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    key: SelectedItem,
    target: Vec3Target,
    owner_id: uuid::Uuid,
    interpolation: Interpolation,
) {
    let mut project = project.borrow_mut();
    let Some(value) = target.value_mut(&mut project, key.clone()) else {
        return;
    };
    if !shared::set_interpolation(value, owner_id, interpolation) {
        return;
    }
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(
        player,
        keyframe_model::live_refresh(ProjectChange {
            video: true,
            ..Default::default()
        }),
    );
}

fn local_time(context: &InspectorContext, key: SelectedItem) -> Option<Time> {
    crate::video::visual_local_time(
        &context.project.borrow(),
        key.clone(),
        player_state::snapshot(&context.player_state).position,
    )
}

fn visible_area(project: &Project, key: SelectedItem) -> Option<(Time, Time)> {
    crate::video::visual_visible_area(project, key)
}

fn selected_scene(project: &Project, key: SelectedItem) -> Option<&shrimply_scene_3d::ObjScene> {
    let item = project.video_item(&key)?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &item.content else {
        return None;
    };
    Some(scene)
}

fn selected_scene_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut shrimply_scene_3d::ObjScene> {
    let item = project.video_item_mut(&key)?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &mut item.content else {
        return None;
    };
    Some(scene)
}

fn commit_refresh(
    project: RefMut<'_, Project>,
    player: &SharedPlayerState,
    inspector: bool,
    commit_name: &str,
) {
    shrimply_project_document::project::commit_edit(&project, commit_name);
    drop(project);
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector,
            ..Default::default()
        },
    );
}
