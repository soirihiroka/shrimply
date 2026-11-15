use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use crate::InspectedItem as SelectedItem;
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::timeline_value::*;
use crate::ui::{NumberPicker, NumberPickerHandle};
use shrimply_project_document::project::{Project, Time};

use crate::keyframe_editor::{self, KeyframeEditorActions};
use crate::timeline_value::{LayeredSections, layered_control};
use crate::{InspectorContext, keyframe_model};

pub(crate) type ScalarGet = for<'a> fn(&'a Project, SelectedItem) -> Option<&'a TimelineValue<f32>>;
pub(crate) type ScalarGetMut =
    for<'a> fn(&'a mut Project, SelectedItem) -> Option<&'a mut TimelineValue<f32>>;

#[derive(Clone, Copy)]
pub(crate) enum ScalarAccess {
    Item {
        get: ScalarGet,
        get_mut: ScalarGetMut,
    },
    ItemScoped {
        get: ScalarGet,
        get_mut: ScalarGetMut,
        value_id: Uuid,
    },
    ItemWithMutation {
        get: ScalarGet,
        get_mut: ScalarGetMut,
        mutated: fn(&mut Project, SelectedItem),
    },
    Modifier {
        id: Uuid,
        value_id: Uuid,
    },
    PaintPalette {
        value_id: Uuid,
    },
    Scene3dScoped {
        value_id: Uuid,
    },
}

impl ScalarAccess {
    fn get(self, project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
        match self {
            Self::Item { get, .. } | Self::ItemWithMutation { get, .. } => {
                get(project, key.clone())
            }
            Self::ItemScoped { get, value_id, .. } => {
                get(project, key.clone()).filter(|value| value.id == value_id)
            }
            Self::Modifier { id, value_id } => crate::modifiers::number(
                project
                    .video_item(&key)?
                    .modifiers
                    .iter()
                    .find(|m| m.id == id)?,
                value_id,
            ),
            Self::PaintPalette { value_id } => {
                let shrimply_project_document::project::VideoItemContent::Paint(paint) =
                    &project.video_item(&key)?.content
                else {
                    return None;
                };
                paint.palette.iter().find_map(|entry| {
                    let texture = entry.texture.as_ref()?;
                    [&texture.repeat_scale, &texture.rotation_degrees]
                        .into_iter()
                        .find(|value| value.id == value_id)
                })
            }
            Self::Scene3dScoped { value_id } => scene_3d(project, key.clone())
                .and_then(|scene| shrimply_inspector_core::scene_3d::number(scene, value_id)),
        }
    }
    fn get_mut(self, project: &mut Project, key: SelectedItem) -> Option<&mut TimelineValue<f32>> {
        match self {
            Self::Item { get_mut, .. } | Self::ItemWithMutation { get_mut, .. } => {
                get_mut(project, key.clone())
            }
            Self::ItemScoped {
                get_mut, value_id, ..
            } => get_mut(project, key.clone()).filter(|value| value.id == value_id),
            Self::Modifier { id, value_id } => crate::modifiers::number_mut(
                project
                    .video_item_mut(&key)?
                    .modifiers
                    .iter_mut()
                    .find(|m| m.id == id)?,
                value_id,
            ),
            Self::PaintPalette { value_id } => {
                let shrimply_project_document::project::VideoItemContent::Paint(paint) =
                    &mut project.video_item_mut(&key)?.content
                else {
                    return None;
                };
                paint.palette.iter_mut().find_map(|entry| {
                    let texture = entry.texture.as_mut()?;
                    [&mut texture.repeat_scale, &mut texture.rotation_degrees]
                        .into_iter()
                        .find(|value| value.id == value_id)
                })
            }
            Self::Scene3dScoped { value_id } => scene_3d_mut(project, key.clone())
                .and_then(|scene| shrimply_inspector_core::scene_3d::number_mut(scene, value_id)),
        }
    }

    fn mark_mutated(self, project: &mut Project, key: SelectedItem) {
        match self {
            Self::ItemWithMutation { mutated, .. } => mutated(project, key),
            Self::PaintPalette { .. } => crate::paint::bump_revision_for_key(project, key),
            _ => {}
        }
    }
}

fn scene_3d(project: &Project, key: SelectedItem) -> Option<&shrimply_scene_3d::ObjScene> {
    let item = project.video_item(&key)?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &item.content else {
        return None;
    };
    Some(scene)
}

fn scene_3d_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut shrimply_scene_3d::ObjScene> {
    let item = project.video_item_mut(&key)?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &mut item.content else {
        return None;
    };
    Some(scene)
}

#[derive(Clone, Copy)]
pub(crate) struct ScalarTarget {
    pub(crate) access: ScalarAccess,
    pub(crate) scope_id: Option<Uuid>,
    pub(crate) local_time: fn(&Project, SelectedItem, Time) -> Option<Time>,
    pub(crate) duration: fn(&Project, SelectedItem) -> Option<Time>,
    pub(crate) refresh: ProjectChange,
    pub(crate) commit_name: &'static str,
}

pub(crate) use shrimply_inspector_core::timeline_value::scalar::ScalarConstraint as ScalarClamp;

#[derive(Clone, Copy)]
pub(crate) struct ScalarSpec {
    pub(crate) drag_step: f64,
    pub(crate) digits: usize,
    pub(crate) integer: bool,
    pub(crate) width_chars: i32,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
    pub(crate) unit_name: Option<&'static str>,
    pub(crate) rotating_icon: Option<(&'static str, f64)>,
    pub(crate) display: fn(f32) -> f64,
    pub(crate) store: fn(f64) -> f32,
    pub(crate) clamp: ScalarClamp,
}

pub(crate) fn scalar_control(
    label: &str,
    value: &TimelineValue<f32>,
    context: &InspectorContext,
    target: ScalarTarget,
    spec: ScalarSpec,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return layered_control(
            label,
            value,
            scalar_picker((spec.display)(value.fallback()), context, target, spec),
            LayeredSections::default(),
            |_| {},
            |_| {},
        );
    };
    let local_time = current_local_time(context, target, key.clone()).unwrap_or(Time::ZERO);
    let display = (spec.display)(value.value_at(local_time));
    let keyframes_enabled = matches!(value.base, TimelineBase::Keyframes(_));
    let expression_enabled = value
        .expression
        .as_ref()
        .is_some_and(|expression| expression.enabled);
    let picker = scalar_picker(display, context, target, spec);

    let mut sections = LayeredSections::default();
    if keyframes_enabled {
        let graph = keyframe_model::scalar_graph(value, display, spec.display);
        let duration = {
            let project = context.project.borrow();
            (target.duration)(&project, key.clone()).unwrap_or(Time::ZERO)
        };
        let built = keyframe_editor::build(
            context,
            graph,
            (Time::ZERO, duration),
            format!(
                "scalar:{}:{:?}:{label}",
                target.commit_name, target.scope_id
            ),
            keyframe_actions(context, key.clone(), target, spec),
        );
        let project = context.project.clone();
        let player = context.player_state.clone();
        let graph_key = key.clone();
        keyframe_editor::connect_graph_refresh(
            context,
            "inspector scalar keyframe graph refresh",
            &built,
            move || {
                let position = player_state::snapshot(&player).position;
                let project = project.borrow();
                let value = target.access.get(&project, graph_key.clone())?;
                let static_value = (target.local_time)(&project, graph_key.clone(), position)
                    .map(|time| (spec.display)(value.value_at(time)))
                    .unwrap_or_else(|| (spec.display)(value.fallback()));
                Some(keyframe_model::scalar_graph(
                    value,
                    static_value,
                    spec.display,
                ))
            },
        );
        sections.set_keyframe(built.widget);
    }

    if expression_enabled {
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
        picker,
        sections,
        move |enabled| {
            if set_keyframes_enabled(
                &keyframe_project,
                &keyframe_player_state,
                keyframe_key.clone(),
                target,
                spec,
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

fn scalar_picker(
    value: f64,
    context: &InspectorContext,
    target: ScalarTarget,
    spec: ScalarSpec,
) -> gtk::Widget {
    let mut picker = if spec.integer {
        NumberPicker::integer_builder(value.round() as i64)
    } else {
        NumberPicker::builder(value)
    }
    .drag_step(spec.drag_step)
    .digits(spec.digits)
    .width_chars(spec.width_chars);
    if let Some(minimum) = spec.minimum {
        picker = picker.minimum(minimum);
    }
    if let (Some(minimum), Some(maximum)) = (spec.minimum, spec.maximum) {
        picker = picker.accepted_range(minimum, maximum);
    }
    if let Some(unit_name) = spec.unit_name {
        picker = picker.unit_name(unit_name);
    }
    if let Some((icon_name, offset_degrees)) = spec.rotating_icon {
        picker = picker.rotating_prefix_icon_name_with_offset(icon_name, offset_degrees);
    }
    let Some(key) = context.selected_item.clone() else {
        return picker.build();
    };
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let commit_project = context.project.clone();
    let commit_player_state = context.player_state.clone();
    let integer_key = key.clone();
    let float_key = key.clone();
    let parts = if spec.integer {
        picker
            .on_change_integer::<i64>(move |next| {
                update_scalar_live(
                    &project,
                    &player_state,
                    integer_key.clone(),
                    target,
                    spec,
                    next as f64,
                );
            })
            .on_commit_integer::<i64>(move |_| {
                shrimply_project_document::project::commit_edit(
                    &commit_project.borrow(),
                    target.commit_name,
                );
                player_state::refresh_project(&commit_player_state, target.refresh);
            })
            .build_with_handle()
    } else {
        picker
            .on_change(move |next| {
                update_scalar_live(
                    &project,
                    &player_state,
                    float_key.clone(),
                    target,
                    spec,
                    next,
                );
            })
            .on_commit(move |_| {
                shrimply_project_document::project::commit_edit(
                    &commit_project.borrow(),
                    target.commit_name,
                );
                player_state::refresh_project(&commit_player_state, target.refresh);
            })
            .build_with_handle()
    };
    connect_scalar_display(context, key.clone(), target, spec, parts.handle);
    parts.widget
}

fn connect_scalar_display(
    context: &InspectorContext,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
    handle: NumberPickerHandle,
) {
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let alive = Rc::downgrade(&context.listener_scope);
    let handle = handle.downgrade();
    player_state::connect_while_alive_named(
        &context.player_state,
        "inspector scalar base display",
        {
            let alive = alive.clone();
            move || alive.upgrade().is_some()
        },
        move |event| {
            if !matches!(
                event,
                player_state::PlayerEvent::State(_) | player_state::PlayerEvent::Project(_)
            ) {
                return;
            }
            let Some(handle) = handle.upgrade() else {
                return;
            };
            let position = player_state::snapshot(&player_state).position;
            let value = {
                let project = project.borrow();
                let Some(local_time) = (target.local_time)(&project, key.clone(), position) else {
                    return;
                };
                let Some(value) = target.access.get(&project, key.clone()) else {
                    return;
                };
                value.value_at(local_time)
            };
            handle.set_f64((spec.display)(value));
        },
    );
}

fn expression_editor(
    context: &InspectorContext,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
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
        "inspector scalar expression output",
        move |refresh| {
            crate::rhai_editor::editor(
                source,
                crate::rhai_editor::ExpressionValue::Scalar,
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
                value: format!(
                    "{:.*}{}",
                    spec.digits,
                    (spec.display)(outcome.value),
                    spec.unit_name.unwrap_or_default(),
                ),
                error: outcome.error,
            })
        },
    )
}

fn keyframe_actions(
    context: &InspectorContext,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
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
            if add_keyframe_at_time(
                &add_project,
                &add_player,
                add_key.clone(),
                target,
                spec,
                time,
            ) {
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
                spec,
                (old_time, time, value),
            );
        }),
        clipboard: crate::keyframe_editor::KeyframeClipboardActions::Local {
            copy: Rc::new(move |times| {
                target
                    .access
                    .get(&copy_project.borrow(), copy_key.clone())
                    .and_then(|value| keyframe_model::copy_keyframes(value, times))
            }),
            paste: Rc::new(move |clipboard, time| {
                let mut project = paste_project.borrow_mut();
                let value = target.access.get_mut(&mut project, paste_key.clone())?;
                let times = keyframe_model::paste_keyframes(value, clipboard, time)?;
                shrimply_inspector_core::timeline_value::scalar::constrain_keyframes(
                    value, &times, spec.clamp,
                );
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

fn update_scalar_live(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
    value: f64,
) {
    let position = player_state::snapshot(player_state).position;
    let mut project = project.borrow_mut();
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return;
    };
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return;
    };
    if !shrimply_inspector_core::timeline_value::scalar::set_displayed_value(
        number,
        keyframe_time,
        value,
        spec.store,
        spec.clamp,
    ) {
        return;
    }
    target.access.mark_mutated(&mut project, key);
    drop(project);
    let mut refresh = keyframe_model::live_refresh(target.refresh);
    refresh.audio_waveforms = false;
    player_state::refresh_project(player_state, refresh);
}

fn set_keyframes_enabled(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
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
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::scalar::set_keyframes_enabled(
        number,
        evaluation_time,
        keyframe_time,
        enabled,
        spec.clamp,
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
    target: ScalarTarget,
    enabled: bool,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    let changed = keyframe_model::set_expression_enabled(
        number,
        enabled,
        shrimply_inspector_core::timeline_value::SCALAR_EXPRESSION_DEFAULT,
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

fn update_expression_source(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ScalarTarget,
    source: String,
) {
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return;
    };
    if !shrimply_inspector_core::timeline_value::scalar::set_expression_source(number, source) {
        return;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    let mut refresh = target.refresh;
    refresh.inspector = false;
    player_state::refresh_project(player_state, refresh);
}

fn add_keyframe_at_time(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ScalarTarget,
    spec: ScalarSpec,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::scalar::add_keyframe(number, time, spec.clamp) {
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
    target: ScalarTarget,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !keyframe_model::delete_scalar_keyframe(number, time) {
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
    target: ScalarTarget,
    spec: ScalarSpec,
    point: (Time, Time, f64),
) -> bool {
    let (old_time, time, value) = point;
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !shrimply_inspector_core::timeline_value::scalar::move_displayed_keyframe(
        number, old_time, time, value, spec.store, spec.clamp,
    ) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
    true
}

fn set_keyframe_interpolation(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ScalarTarget,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(number) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if !keyframe_model::set_scalar_interpolation(number, owner_id, interpolation) {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
    true
}

fn current_local_time(
    context: &InspectorContext,
    target: ScalarTarget,
    key: SelectedItem,
) -> Option<Time> {
    let project = context.project.borrow();
    let position = player_state::snapshot(&context.player_state).position;
    (target.local_time)(&project, key.clone(), position)
}
