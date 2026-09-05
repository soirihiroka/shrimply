use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::player_state::SharedPlayerState;
use crate::project::Project;
use crate::selection_state::SharedSelectionState;
use uuid::Uuid;

use super::TimelineRuntime;
use super::external_content::{self, Content, Origin, Placement};

pub(super) fn setup(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
) {
    let mask_drop = gtk::DropTarget::new(glib::Bytes::static_type(), gdk::DragAction::COPY);
    mask_drop.set_preload(true);
    let mask_motion_project = project.clone();
    let mask_motion_runtime = runtime.clone();
    mask_drop.connect_motion(move |_, x, y| {
        mask_target_at(
            &mask_motion_project.borrow(),
            mask_motion_runtime.borrow().scene.view(),
            x,
            y,
        )
        .map_or(gdk::DragAction::empty(), |_| gdk::DragAction::COPY)
    });
    let mask_area = area.clone();
    let mask_project = project.clone();
    let mask_player_state = player_state.clone();
    let mask_runtime = runtime.clone();
    mask_drop.connect_drop(move |_, value, x, y| {
        let Ok(bytes) = value.get::<glib::Bytes>() else {
            return false;
        };
        let Ok(text) = std::str::from_utf8(bytes.as_ref()) else {
            return false;
        };
        let Ok(modifier_id) = Uuid::parse_str(text) else {
            return false;
        };
        assign_mask_source(
            &mask_area,
            &mask_project,
            &mask_player_state,
            &mask_runtime,
            modifier_id,
            x,
            y,
        )
    });
    area.add_controller(mask_drop);

    let mask_text_drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::COPY);
    mask_text_drop.set_preload(true);
    let mask_motion_project = project.clone();
    let mask_motion_runtime = runtime.clone();
    mask_text_drop.connect_motion(move |_, x, y| {
        mask_target_at(
            &mask_motion_project.borrow(),
            mask_motion_runtime.borrow().scene.view(),
            x,
            y,
        )
        .map_or(gdk::DragAction::empty(), |_| gdk::DragAction::COPY)
    });
    let mask_area = area.clone();
    let mask_project = project.clone();
    let mask_player_state = player_state.clone();
    let mask_runtime = runtime.clone();
    mask_text_drop.connect_drop(move |_, value, x, y| {
        let Ok(text) = value.get::<String>() else {
            return false;
        };
        let Ok(modifier_id) = Uuid::parse_str(&text) else {
            return false;
        };
        assign_mask_source(
            &mask_area,
            &mask_project,
            &mask_player_state,
            &mask_runtime,
            modifier_id,
            x,
            y,
        )
    });
    area.add_controller(mask_text_drop);

    let formats = gdk::ContentFormats::for_type(gdk::FileList::static_type())
        .union(&gdk::ContentFormats::for_type(gio::File::static_type()))
        .union(&gdk::ContentFormats::for_type(gdk::Texture::static_type()))
        .union(&gdk::ContentFormats::for_type(String::static_type()))
        .union(&gdk::ContentFormats::new(&[
            "text/uri-list",
            "x-special/gnome-copied-files",
        ]));
    let drop = gtk::DropTarget::builder()
        .formats(&formats)
        .actions(gdk::DragAction::COPY)
        .preload(true)
        .build();
    let enter_area = area.clone();
    let enter_runtime = runtime.clone();
    drop.connect_enter(move |target, x, y| {
        update_preview(target.value().as_ref(), &enter_runtime, x, y);
        enter_area.queue_render();
        gdk::DragAction::COPY
    });
    let motion_area = area.clone();
    let motion_runtime = runtime.clone();
    drop.connect_motion(move |target, x, y| {
        update_preview(target.value().as_ref(), &motion_runtime, x, y);
        motion_area.queue_render();
        gdk::DragAction::COPY
    });
    let leave_area = area.clone();
    let leave_runtime = runtime.clone();
    drop.connect_leave(move |_| {
        let mut runtime = leave_runtime.borrow_mut();
        runtime.scene.clear_drop_preview();
        leave_area.queue_render();
    });
    let drop_area = area.clone();
    drop.connect_drop(move |target, value, x, y| {
        let Some(content) = external_content::from_value(value) else {
            let source_formats = target
                .current_drop()
                .map(|drop| drop.formats().to_str())
                .unwrap_or_else(|| "unavailable".into());
            tracing::warn!(
                "Unsupported timeline drop payload: value_type={} source_formats={} x={x:.1} y={y:.1}",
                value.type_().name(),
                source_formats,
            );
            return false;
        };
        let content_kind = content.label();
        {
            let mut runtime = runtime.borrow_mut();
            runtime.scene.clear_drop_preview();
        }
        let inserted = external_content::insert(
            &drop_area,
            &project,
            &player_state,
            &selection_state,
            &runtime,
            content,
            Origin::Drop,
            Placement::Timeline { x, y },
        );
        if !inserted {
            tracing::warn!(
                "Timeline drop could not be inserted: content={content_kind} x={x:.1} y={y:.1}"
            );
        }
        inserted
    });
    area.add_controller(drop);
}

fn mask_target_at(
    project: &Project,
    view: super::TimelineViewState,
    x: f64,
    y: f64,
) -> Option<super::items::ItemKey> {
    let target = super::items::hit_item_at(project, view, x, y)?;
    (target.kind == super::items::TrackKind::Video).then_some(target)
}

fn assign_mask_source(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    modifier_id: Uuid,
    x: f64,
    y: f64,
) -> bool {
    let source = {
        let project = project.borrow();
        let Some(target) = mask_target_at(&project, runtime.borrow().scene.view(), x, y) else {
            return false;
        };
        let Some(track) = project.video_tracks.get(target.track_index) else {
            return false;
        };
        let Some(item) = track.items.get(target.item_index) else {
            return false;
        };
        shrimply_project::project::ItemAddress::Video {
            sequence_path: Vec::new(),
            track_id: track.id,
            item_id: item.id,
        }
    };
    let mut project = project.borrow_mut();
    let changed = match shrimply_inspector_core::visual_modifiers::set_mask_source(
        &mut project,
        &source,
        modifier_id,
    ) {
        Ok(changed) => changed,
        Err(_) => return false,
    };
    drop(project);
    if !changed {
        return true;
    }
    crate::player_state::refresh_project(
        player_state,
        crate::player_state::ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    area.queue_render();
    true
}

fn update_preview(
    value: Option<&glib::Value>,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    x: f64,
    y: f64,
) {
    let mut runtime = runtime.borrow_mut();
    let point = super::vec2(x as f32, y as f32);
    match value.and_then(external_content::from_value) {
        Some(Content::Text(text)) => runtime.scene.update_text_drop_preview(text, point),
        Some(Content::File(path)) => {
            runtime.scene.update_drop_preview(path, point);
        }
        Some(Content::Texture(_)) | Some(Content::Url(_)) | None => {
            runtime.scene.clear_drop_preview()
        }
    }
}
