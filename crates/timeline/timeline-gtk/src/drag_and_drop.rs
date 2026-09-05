use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::player_state::SharedPlayerState;
use crate::project::{Project, Time};
use crate::selection_state::SharedSelectionState;
use uuid::Uuid;

use super::external_content::{self, Content, Origin, Placement};
use super::{RULER_HEIGHT, TimelineImportPreview, TimelineRuntime, import, x_to_time};

const IMPORT_INSPECTION_DELIVERY_INTERVAL: Duration = Duration::from_millis(16);

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
            mask_motion_runtime.borrow().view,
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
            mask_motion_runtime.borrow().view,
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
    let enter_project = project.clone();
    let enter_runtime = runtime.clone();
    drop.connect_enter(move |target, x, y| {
        update_preview(
            &enter_area,
            target.value().as_ref(),
            &enter_project,
            &enter_runtime,
            x,
            y,
        );
        enter_area.queue_render();
        gdk::DragAction::COPY
    });
    let motion_area = area.clone();
    let motion_project = project.clone();
    let motion_runtime = runtime.clone();
    drop.connect_motion(move |target, x, y| {
        update_preview(
            &motion_area,
            target.value().as_ref(),
            &motion_project,
            &motion_runtime,
            x,
            y,
        );
        motion_area.queue_render();
        gdk::DragAction::COPY
    });
    let leave_area = area.clone();
    let leave_runtime = runtime.clone();
    drop.connect_leave(move |_| {
        let mut runtime = leave_runtime.borrow_mut();
        runtime.import_preview = None;
        runtime.text_drop_preview = None;
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
            runtime.import_preview = None;
            runtime.text_drop_preview = None;
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
        let Some(target) = mask_target_at(&project, runtime.borrow().view, x, y) else {
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
    area: &gtk::GLArea,
    value: Option<&glib::Value>,
    project: &Rc<RefCell<Project>>,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    x: f64,
    y: f64,
) {
    let path = match value.and_then(external_content::from_value) {
        Some(Content::Text(text)) => {
            let preview = {
                let project = project.borrow();
                let runtime = runtime.borrow();
                external_content::text_preview(&project, &runtime, text, x, y)
            };
            let mut runtime = runtime.borrow_mut();
            runtime.import_preview = None;
            runtime.text_drop_preview = preview;
            return;
        }
        Some(Content::File(path)) => path,
        Some(Content::Texture(_)) | Some(Content::Url(_)) | None => {
            let mut runtime = runtime.borrow_mut();
            runtime.import_preview = None;
            runtime.text_drop_preview = None;
            return;
        }
    };
    runtime.borrow_mut().text_drop_preview = None;
    let Some(file_kind) = import::file_kind(&path) else {
        runtime.borrow_mut().import_preview = None;
        return;
    };
    let (visual_kind, video_streams, audio_streams) = match file_kind {
        import::FileKind::Mp4
        | import::FileKind::Mov
        | import::FileKind::Mkv
        | import::FileKind::WebM => (Some(import::VisualMediaKind::Video), 1, 1),
        import::FileKind::Image => (Some(import::VisualMediaKind::Image), 1, 0),
        import::FileKind::Gif => (Some(import::VisualMediaKind::Gif), 1, 0),
        import::FileKind::Svg => (Some(import::VisualMediaKind::Svg), 1, 0),
        import::FileKind::Pdf => (Some(import::VisualMediaKind::Pdf), 1, 0),
        import::FileKind::Python => (Some(import::VisualMediaKind::Manim), 1, 0),
        import::FileKind::Blender => (Some(import::VisualMediaKind::Blender), 1, 0),
        import::FileKind::LayeredImage => (Some(import::VisualMediaKind::LayeredImage), 1, 0),
        import::FileKind::Obj => (Some(import::VisualMediaKind::Obj), 1, 0),
        import::FileKind::Ply => (Some(import::VisualMediaKind::Gaussian), 1, 0),
        import::FileKind::Audio => (None, 0, 1),
        import::FileKind::Vtt => {
            runtime.borrow_mut().import_preview = None;
            return;
        }
    };

    let (canvas_size, default_visual_duration, new_source) = {
        let project = project.borrow();
        let mut runtime = runtime.borrow_mut();
        let new_source = runtime
            .import_preview
            .as_ref()
            .is_none_or(|preview| preview.source.path() != path);
        let (source, duration, visual_kind, video_streams, audio_streams) = runtime
            .import_preview
            .as_ref()
            .filter(|preview| preview.source.path() == path)
            .map(|preview| {
                (
                    preview.source.clone(),
                    preview.duration,
                    preview.visual_kind,
                    preview.preview.video_streams,
                    preview.preview.audio_streams,
                )
            })
            .unwrap_or_else(|| {
                (
                    crate::project::Asset::new(path.clone()),
                    runtime.default_visual_duration,
                    visual_kind,
                    video_streams,
                    audio_streams,
                )
            });
        let view = runtime.view;
        let start =
            Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));
        let start = runtime.snap_repository.snap(start).unwrap_or(start);
        let y = y.max(RULER_HEIGHT) + view.scroll_y;
        let preview = import::preview(
            &project,
            duration,
            video_streams,
            audio_streams,
            start,
            super::items::NewItemTarget::AtY(y),
            runtime.drag_collision_mode,
        );
        runtime.import_preview = Some(TimelineImportPreview {
            source,
            duration,
            visual_kind,
            preview,
            y,
        });
        (
            project.canvas_size,
            runtime.default_visual_duration,
            new_source,
        )
    };

    if !new_source || file_kind == import::FileKind::Python {
        return;
    }

    let subscription =
        import::request_inspection(path.clone(), canvas_size, default_visual_duration);
    let project = project.clone();
    let runtime_for_result = runtime.clone();
    let handle = shrimply_gtk_components::resource_pipeline::deliver(
        area.downgrade(),
        subscription,
        IMPORT_INSPECTION_DELIVERY_INTERVAL,
        move |area, event| match event {
            shrimply_resource_pipeline::Event::Finished(info) => {
                let project = project.borrow();
                let mut runtime = runtime_for_result.borrow_mut();
                let Some(current) = runtime
                    .import_preview
                    .as_ref()
                    .filter(|preview| preview.source.path() == path)
                else {
                    return;
                };
                let start = current.preview.start;
                let y = current.y;
                let preview = import::preview(
                    &project,
                    info.duration,
                    info.video_streams,
                    info.audio_streams,
                    start,
                    super::items::NewItemTarget::AtY(y),
                    runtime.drag_collision_mode,
                );
                runtime.import_preview = Some(TimelineImportPreview {
                    source: info.source.clone(),
                    duration: info.duration,
                    visual_kind: info.visual_kind,
                    preview,
                    y,
                });
                area.queue_render();
            }
            shrimply_resource_pipeline::Event::Failed(error) => {
                tracing::warn!("Could not inspect dropped media: {error}");
            }
            shrimply_resource_pipeline::Event::Progress(_)
            | shrimply_resource_pipeline::Event::Cancelled => {}
        },
    );
    let mut runtime = runtime.borrow_mut();
    runtime.resource_jobs.retain(|job| job.is_active());
    runtime.resource_jobs.push(handle);
}
