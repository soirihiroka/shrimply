use shrimply_components_gtk::tr;
use std::cell::RefCell;
use std::rc::Rc;

use super::{LayeredSections, layered_control};
use crate::InspectedItem as SelectedItem;
use crate::keyframe_editor::{self, KeyframeEditorActions};
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::timeline_value::*;
use crate::{InspectorContext, keyframe_model};
use shrimply_components_gtk::ui::{ColorPicker, WeakColorPickerHandle};
use shrimply_project_document::project::Time;
use shrimply_project_document::project::{Color, Project};
use uuid::Uuid;

pub(crate) type ColorGetMut =
    for<'a> fn(
        &'a mut Project,
        SelectedItem,
    ) -> Option<&'a mut TimelineValue<shrimply_property_model::Color<u8>>>;

#[derive(Clone, Copy)]
pub(crate) enum ColorAccess {
    ItemScoped {
        get_mut: ColorGetMut,
        value_id: Uuid,
    },
    Scene3dScoped {
        value_id: Uuid,
    },
    Modifier {
        id: Uuid,
        value_id: Uuid,
    },
    PaintPalette {
        value_id: Uuid,
    },
}
impl ColorAccess {
    fn get_mut(
        self,
        p: &mut Project,
        k: SelectedItem,
    ) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
        match self {
            Self::ItemScoped { get_mut, value_id } => {
                get_mut(p, k.clone()).filter(|value| value.id == value_id)
            }
            Self::Scene3dScoped { value_id } => scene_3d(p, k.clone())
                .and_then(|scene| shrimply_inspector_core::scene_3d::color_mut(scene, value_id)),
            Self::Modifier { id, value_id } => crate::modifiers::color_mut(
                &mut p
                    .video_item_mut(&k)?
                    .modifiers
                    .iter_mut()
                    .find(|m| m.id == id)?
                    .effect,
                value_id,
            ),
            Self::PaintPalette { value_id } => {
                let shrimply_project_document::project::VideoItemContent::Paint(paint) =
                    &mut p.video_item_mut(&k)?.content
                else {
                    return None;
                };
                paint
                    .palette
                    .iter_mut()
                    .map(|entry| &mut entry.color)
                    .find(|color| color.id == value_id)
            }
        }
    }

    fn mark_mutated(self, project: &mut Project, key: SelectedItem) {
        if let Self::PaintPalette { .. } = self {
            crate::paint::bump_revision_for_key(project, key);
        }
    }
}

fn scene_3d(project: &mut Project, key: SelectedItem) -> Option<&mut shrimply_scene_3d::ObjScene> {
    let item = project.video_item_mut(&key)?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &mut item.content else {
        return None;
    };
    Some(scene)
}

#[derive(Clone, Copy)]
pub(crate) struct ColorTarget {
    pub(crate) access: ColorAccess,
    pub(crate) scope_id: Option<Uuid>,
    pub(crate) local_time: fn(&Project, SelectedItem, Time) -> Option<Time>,
    pub(crate) duration: fn(&Project, SelectedItem) -> Option<Time>,
    pub(crate) refresh: ProjectChange,
    pub(crate) commit_name: &'static str,
}

pub(crate) fn color_control(
    label: &str,
    value: &TimelineValue<shrimply_property_model::Color<u8>>,
    context: &InspectorContext,
    target: ColorTarget,
) -> gtk::Widget {
    let timeline_id = value.id;
    let color =
        shrimply_inspector_core::timeline_color::value_at(value, current_time(context, target));
    let Some(key) = context.selected_item.clone() else {
        let button = ColorPicker::builder(color)
            .title(tr!(label).as_ref())
            .hexpand(true)
            .build();
        return layered_control(
            label,
            value,
            button,
            LayeredSections::default(),
            |_| {},
            |_| {},
        );
    };
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let refresh = context.refresh.clone();
    let button_key = key.clone();
    let picker = ColorPicker::builder(color)
        .title(tr!(label).as_ref())
        .hexpand(true)
        .on_change(move |color| {
            if update_color(
                &project,
                &player_state,
                button_key.clone(),
                target,
                timeline_id,
                color,
            ) {
                refresh();
            }
        })
        .build_with_handle();
    let button = picker.widget;
    connect_color_display(
        context,
        key.clone(),
        target,
        timeline_id,
        picker.handle.downgrade(),
    );
    let keyframes = matches!(value.base, TimelineBase::Keyframes(_));
    let expression = value.expression.as_ref().is_some_and(|v| v.enabled);
    let mut sections = LayeredSections::default();
    if keyframes {
        let duration = {
            let project = context.project.borrow();
            (target.duration)(&project, key.clone()).unwrap_or(Time::ZERO)
        };
        let built = keyframe_editor::build(
            context,
            shrimply_inspector_core::timeline_color::keyframe_graph(value),
            (Time::ZERO, duration),
            format!("color:{}:{:?}:{label}", target.commit_name, target.scope_id),
            keyframe_actions(context, key.clone(), target, timeline_id),
        );
        let project = context.project.clone();
        let graph_key = key.clone();
        keyframe_editor::connect_graph_refresh(
            context,
            "inspector color keyframe graph refresh",
            &built,
            move || {
                let project = project.borrow();
                let value = project.video_item(&graph_key).and_then(|item| {
                    shrimply_inspector_core::timeline_color::video_value(item, timeline_id)
                })?;
                shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id)
                    .ok()?;
                Some(shrimply_inspector_core::timeline_color::keyframe_graph(
                    value,
                ))
            },
        );
        sections.set_keyframe(built.widget);
    }
    if expression {
        sections.push_expression(expression_editor(context, key.clone(), target, timeline_id));
    }
    let kp = context.project.clone();
    let kpl = context.player_state.clone();
    let kr = context.refresh.clone();
    let ep = context.project.clone();
    let epl = context.player_state.clone();
    let er = context.refresh.clone();
    let keyframe_key = key.clone();
    let expression_key = key;
    layered_control(
        label,
        value,
        button,
        sections,
        move |enabled| {
            if toggle_keyframes(
                &kp,
                &kpl,
                keyframe_key.clone(),
                target,
                timeline_id,
                enabled,
            ) {
                kr();
            }
        },
        move |enabled| {
            toggle_expression(
                &ep,
                &epl,
                expression_key.clone(),
                target,
                timeline_id,
                enabled,
            );
            er();
        },
    )
}

fn expression_editor(
    context: &InspectorContext,
    key: SelectedItem,
    target: ColorTarget,
    timeline_id: Uuid,
) -> gtk::Widget {
    let source = context
        .project
        .borrow()
        .video_item(&key)
        .and_then(|item| shrimply_inspector_core::timeline_color::video_value(item, timeline_id))
        .and_then(|value| value.expression_source().map(str::to_string));
    let project = context.project.clone();
    let player = context.player_state.clone();
    let editor_key = key.clone();
    let output_key = key;
    expression_section(
        context,
        "inspector color expression output",
        move |refresh| {
            crate::rhai_editor::editor(
                source,
                crate::rhai_editor::ExpressionValue::Color,
                move |source| {
                    update_expression(
                        &project,
                        &player,
                        editor_key.clone(),
                        target,
                        timeline_id,
                        source,
                    );
                    refresh();
                },
            )
        },
        move |project, position, audio, cache| {
            let value = project.video_item(&output_key).and_then(|item| {
                shrimply_inspector_core::timeline_color::video_value(item, timeline_id)
            })?;
            let outcome = evaluate_expression(project, &output_key, position, audio, cache, value)?;
            Some(ExpressionOutput {
                value: format!(
                    "#{:02X}{:02X}{:02X}{:02X}",
                    outcome.value.r, outcome.value.g, outcome.value.b, outcome.value.a,
                ),
                error: outcome.error,
            })
        },
    )
}

fn connect_color_display(
    context: &InspectorContext,
    key: SelectedItem,
    target: ColorTarget,
    timeline_id: Uuid,
    picker: WeakColorPickerHandle,
) {
    let project = context.project.clone();
    let player = context.player_state.clone();
    let alive = Rc::downgrade(&context.listener_scope);
    let alive_for_prune = alive.clone();
    player_state::connect_while_alive_named(
        &context.player_state,
        "inspector color display refresh",
        move || alive_for_prune.upgrade().is_some(),
        move |event| {
            if !matches!(
                event,
                player_state::PlayerEvent::State(_) | player_state::PlayerEvent::Project(_)
            ) || alive.upgrade().is_none()
            {
                return;
            }
            let position = player_state::snapshot(&player).position;
            let project = project.borrow();
            let Some(time) = (target.local_time)(&project, key.clone(), position) else {
                return;
            };
            let Some(value) = project.video_item(&key).and_then(|item| {
                shrimply_inspector_core::timeline_color::video_value(item, timeline_id)
            }) else {
                return;
            };
            if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id)
                .is_ok()
            {
                picker.set_color(shrimply_inspector_core::timeline_color::value_at(
                    value, time,
                ));
            }
        },
    );
}

fn update_color(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    target: ColorTarget,
    timeline_id: Uuid,
    color: Color<u8>,
) -> bool {
    let mut project = project.borrow_mut();
    let position = player_state::snapshot(player_state).position;
    let Some(evaluation_time) = (target.local_time)(&project, key.clone(), position) else {
        return false;
    };
    let Some(keyframe_time) = project.keyframe_time(&key, position) else {
        return false;
    };
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::set_value(
            value,
            evaluation_time,
            keyframe_time,
            color,
        )
    {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_coalesced_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
    true
}

fn current_time(c: &InspectorContext, t: ColorTarget) -> Time {
    let p = c.project.borrow();
    c.selected_item
        .clone()
        .and_then(|key| {
            (t.local_time)(
                &p,
                key.clone(),
                player_state::snapshot(&c.player_state).position,
            )
        })
        .unwrap_or(Time::ZERO)
}
fn keyframe_actions(
    context: &InspectorContext,
    key: SelectedItem,
    target: ColorTarget,
    timeline_id: Uuid,
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
                timeline_id,
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
                timeline_id,
                time,
            ) {
                refresh_delete();
            }
        }),
        update_point: Rc::new(move |old_time, time, _| {
            update_keyframe_point(
                &point_project,
                &point_player,
                point_key.clone(),
                target,
                timeline_id,
                old_time,
                time,
            );
        }),
        clipboard: crate::keyframe_editor::KeyframeClipboardActions::Local {
            copy: Rc::new(move |times| {
                let mut project = copy_project.borrow_mut();
                target
                    .access
                    .get_mut(&mut project, copy_key.clone())
                    .filter(|value| {
                        shrimply_inspector_core::timeline_color::validate_timeline(
                            value,
                            timeline_id,
                        )
                        .is_ok()
                    })
                    .and_then(|value| {
                        shrimply_inspector_core::timeline_color::copy_keyframes(value, times)
                    })
            }),
            paste: Rc::new(move |clipboard, time| {
                let mut project = paste_project.borrow_mut();
                let value = target.access.get_mut(&mut project, paste_key.clone())?;
                shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id)
                    .ok()?;
                let times = shrimply_inspector_core::timeline_color::paste_keyframes(
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
                timeline_id,
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
    target: ColorTarget,
    timeline_id: Uuid,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::add_keyframe(value, time)
    {
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
    target: ColorTarget,
    timeline_id: Uuid,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::delete_keyframe(value, time)
    {
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
    target: ColorTarget,
    timeline_id: Uuid,
    old_time: Time,
    time: Time,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::move_keyframes(value, &[(old_time, time)])
    {
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
    target: ColorTarget,
    timeline_id: Uuid,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> bool {
    let mut project = project.borrow_mut();
    let Some(value) = target.access.get_mut(&mut project, key.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !matches!(
            shrimply_inspector_core::timeline_color::set_interpolation(
                value,
                owner_id,
                interpolation,
            ),
            Ok(true)
        )
    {
        return false;
    }
    target.access.mark_mutated(&mut project, key);
    shrimply_project_document::project::commit_edit(&project, target.commit_name);
    drop(project);
    player_state::refresh_project(player_state, keyframe_model::live_refresh(target.refresh));
    true
}

fn toggle_keyframes(
    p: &Rc<RefCell<Project>>,
    s: &SharedPlayerState,
    k: SelectedItem,
    t: ColorTarget,
    timeline_id: Uuid,
    enabled: bool,
) -> bool {
    let position = player_state::snapshot(s).position;
    let mut p = p.borrow_mut();
    let Some(evaluation_time) = (t.local_time)(&p, k.clone(), position) else {
        return false;
    };
    let Some(keyframe_time) = p.keyframe_time(&k, position) else {
        return false;
    };
    let Some(v) = t.access.get_mut(&mut p, k.clone()) else {
        return false;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(v, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::set_keyframes_enabled(
            v,
            evaluation_time,
            keyframe_time,
            enabled,
        )
    {
        return false;
    }
    t.access.mark_mutated(&mut p, k);
    shrimply_project_document::project::commit_edit(&p, t.commit_name);
    drop(p);
    player_state::refresh_project(s, t.refresh);
    true
}
fn toggle_expression(
    p: &Rc<RefCell<Project>>,
    s: &SharedPlayerState,
    k: SelectedItem,
    t: ColorTarget,
    timeline_id: Uuid,
    enabled: bool,
) {
    let mut p = p.borrow_mut();
    let Some(v) = t.access.get_mut(&mut p, k.clone()) else {
        return;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(v, timeline_id).is_err() {
        return;
    }
    let changed = shrimply_inspector_core::timeline_color::set_expression_enabled(v, enabled);
    if !changed {
        return;
    }
    t.access.mark_mutated(&mut p, k);
    shrimply_project_document::project::commit_edit(&p, t.commit_name);
    drop(p);
    player_state::refresh_project(s, t.refresh)
}
fn update_expression(
    p: &Rc<RefCell<Project>>,
    s: &SharedPlayerState,
    k: SelectedItem,
    t: ColorTarget,
    timeline_id: Uuid,
    source: String,
) {
    let mut p = p.borrow_mut();
    let Some(value) = t.access.get_mut(&mut p, k.clone()) else {
        return;
    };
    if shrimply_inspector_core::timeline_color::validate_timeline(value, timeline_id).is_err()
        || !shrimply_inspector_core::timeline_color::set_expression_source(value, source)
    {
        return;
    }
    t.access.mark_mutated(&mut p, k);
    shrimply_project_document::project::commit_coalesced_edit(&p, t.commit_name);
    drop(p);
    player_state::refresh_project(s, keyframe_model::live_refresh(t.refresh))
}
