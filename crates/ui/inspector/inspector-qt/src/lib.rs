mod action_controller;
mod audio;
mod backend;
mod graph_backend;
mod info;
mod item;
mod list;
mod section;
mod timeline_controller;
mod value_backend;

use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::{preferences::SharedPreferences, preview_focus::SharedPreviewFocus};
use shrimply_inspector_core::{ControlKind, InspectorController, InspectorTarget};
use shrimply_project_document::project::ItemAddress;
use std::cell::{Cell, RefCell};

use crate::item::InspectorAction;
use crate::list::{InspectorDocument, PreviewItem};
use action_controller::*;
use timeline_controller::*;

thread_local! {
    static CONTROLLER: RefCell<Option<InspectorController>> = const { RefCell::new(None) };
    static PROPERTY_CLIPBOARD: RefCell<Option<shrimply_property_transfer::SharedClipboard>> = const { RefCell::new(None) };
    static PREFERENCES: RefCell<Option<SharedPreferences>> = const { RefCell::new(None) };
    static PREVIEW_FOCUS: RefCell<Option<SharedPreviewFocus>> = const { RefCell::new(None) };
    static PROJECT_FONTS: RefCell<Option<shrimply_inspector_core::font_cache::ProjectFontActivation>> = const { RefCell::new(None) };
    static FONT_BROWSER: RefCell<shrimply_inspector_core::font_selector::Browser> = RefCell::new(shrimply_inspector_core::font_selector::Browser::default());
    static PENDING_FONT_EDIT: RefCell<Option<PendingFontEdit>> = const { RefCell::new(None) };
    static CACHE_STATUSES: RefCell<shrimply_inspector_core::CacheStatusTracker<CacheKind>> = RefCell::new(shrimply_inspector_core::CacheStatusTracker::default());
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    static CACHE_DIRTY: Cell<bool> = const { Cell::new(false) };
    static EXPRESSION_DIRTY: Cell<bool> = const { Cell::new(false) };
    static GRAPH_DIRTY: Cell<bool> = const { Cell::new(false) };
    static PLAYHEAD_DIRTY: Cell<bool> = const { Cell::new(false) };
    static TRANSFORM_DIRTY: Cell<bool> = const { Cell::new(false) };
    static FOCUS_DIRTY: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone)]
struct PendingFontEdit {
    target: InspectorTarget,
    modifier_id: Option<uuid::Uuid>,
    path: String,
    commit_name: String,
    source_value: String,
    value: String,
}

type FontEditActivation = (PendingFontEdit, Result<(), String>);
type FontBrowserPoll = (bool, Vec<FontEditActivation>);

#[derive(Clone, Eq, PartialEq)]
enum CacheKind {
    Audio,
    Visual(shrimply_project_document::project::ItemAddress),
}

pub fn init() {
    cxx_qt::init_crate!(shrimply_inspector_qt);
    cxx_qt::init_qml_module!("dev.shrimply.inspector");
}

pub fn install(session: &EditorSession) {
    let default_font =
        shrimply_editor_state::preferences::snapshot(&session.preferences).default_text_font_family;
    CONTROLLER.with_borrow_mut(|controller| {
        assert!(controller.is_none(), "Qt inspector is already installed");
        *controller = Some(
            InspectorController::new(
                session.project.clone(),
                session.player_state.clone(),
                session.selection_state.clone(),
            )
            .with_default_text_font(default_font.clone())
            .with_property_clipboard(session.property_clipboard.clone()),
        );
    });
    PROPERTY_CLIPBOARD.with_borrow_mut(|clipboard| {
        assert!(
            clipboard.is_none(),
            "Qt inspector clipboard is already installed"
        );
        *clipboard = Some(session.property_clipboard.clone());
    });
    PREFERENCES.with_borrow_mut(|preferences| {
        assert!(
            preferences.is_none(),
            "Qt inspector preferences are already installed"
        );
        *preferences = Some(session.preferences.clone());
    });
    PREVIEW_FOCUS.with_borrow_mut(|preview_focus| {
        assert!(
            preview_focus.is_none(),
            "Qt inspector preview focus is already installed"
        );
        *preview_focus = Some(session.preview_focus.clone());
    });
    PROJECT_FONTS.with_borrow_mut(|activation| {
        *activation = shrimply_inspector_core::font_cache::activate_project_google_fonts(
            &session.project.borrow(),
            &default_font,
        );
    });
    shrimply_timeline_edit::selection_state::connect_named(
        &session.selection_state,
        "Qt inspector selection",
        mark_dirty,
    );
    shrimply_editor_state::player_state::connect_named(
        &session.player_state,
        "Qt inspector project",
        |event| match event {
            shrimply_editor_state::player_state::PlayerEvent::State(_) => PLAYHEAD_DIRTY.set(true),
            shrimply_editor_state::player_state::PlayerEvent::Project(change)
                if change.inspector =>
            {
                mark_dirty()
            }
            shrimply_editor_state::player_state::PlayerEvent::Project(change)
                if change.video && change.live_preview =>
            {
                TRANSFORM_DIRTY.set(true)
            }
            shrimply_editor_state::player_state::PlayerEvent::Project(change) if change.audio => {
                TRANSFORM_DIRTY.set(true);
                EXPRESSION_DIRTY.set(true);
            }
            _ => {}
        },
    );
    shrimply_editor_state::preview_focus::connect_named(
        &session.preview_focus,
        "Qt inspector preview focus",
        || FOCUS_DIRTY.set(true),
    );
    mark_dirty();
}

fn mark_dirty() {
    DIRTY.set(true);
}

fn target_change_pending(current: &InspectorTarget) -> bool {
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector target requested before installation")
            .target()
            != *current
    })
}

fn take_document() -> Option<InspectorDocument> {
    receive_project_fonts();
    receive_cache_statuses();
    if CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .is_some_and(shrimply_inspector_document::poll)
    }) {
        mark_dirty();
    }
    if !DIRTY.replace(false) {
        return None;
    }
    CONTROLLER.with_borrow(|controller| {
        let controller = controller.as_ref()?;
        let preferences = PREFERENCES.with_borrow(|preferences| {
            shrimply_editor_state::preferences::snapshot(
                preferences
                    .as_ref()
                    .expect("Qt inspector preferences are installed"),
            )
        });
        let shared = shrimply_inspector_document::document(
            controller,
            info::format_date,
            &preferences.compute_server_url,
            &preferences.last_tts_model,
        );
        let preview_item = match &shared.target {
            InspectorTarget::Item(address @ ItemAddress::Video { .. }) => Some(PreviewItem {
                address: address.clone(),
                id: address.item_id(),
            }),
            _ => None,
        };
        let document = InspectorDocument {
            target: shared.target,
            title: shared.title,
            categories: shared.categories,
            preview_item,
        };
        retain_cache_statuses(&document);
        validate_preview_focus(&document);
        Some(document)
    })
}

fn receive_project_fonts() {
    let finished = PROJECT_FONTS.with_borrow(|activation| {
        activation
            .as_ref()
            .is_some_and(shrimply_inspector_core::font_cache::ProjectFontActivation::finished)
    });
    if !finished {
        return;
    }
    PROJECT_FONTS.with_borrow_mut(|activation| drop(activation.take()));
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector font activation completed before installation")
            .refresh_video();
    });
}

fn open_font_browser() -> bool {
    FONT_BROWSER.with_borrow_mut(|browser| browser.open())
}

fn search_font_browser(query: String) -> bool {
    FONT_BROWSER.with_borrow_mut(|browser| browser.search(query))
}

fn request_font_browser_previews(range: std::ops::Range<usize>) {
    FONT_BROWSER.with_borrow_mut(|browser| browser.request_previews(range));
}

fn font_browser_count() -> usize {
    FONT_BROWSER.with_borrow(|browser| browser.visible().len())
}

fn font_browser_choice(index: usize) -> Option<shrimply_inspector_core::font_cache::FontFamily> {
    FONT_BROWSER.with_borrow(|browser| browser.visible().get(index).cloned())
}

fn find_font_browser_choice(
    name: &str,
    source: shrimply_inspector_core::font_cache::FontSource,
) -> Option<shrimply_inspector_core::font_cache::FontFamily> {
    FONT_BROWSER.with_borrow(|browser| {
        browser
            .visible()
            .iter()
            .find(|family| family.source == source && family.name.eq_ignore_ascii_case(name))
            .cloned()
    })
}

fn font_browser_lookup() -> Option<shrimply_inspector_core::font_cache::GoogleFamily> {
    FONT_BROWSER.with_borrow(|browser| browser.lookup().cloned())
}

fn font_browser_status() -> String {
    FONT_BROWSER.with_borrow(|browser| browser.status().to_string())
}

fn font_browser_busy() -> bool {
    FONT_BROWSER.with_borrow(shrimply_inspector_core::font_selector::Browser::busy)
}

fn activate_font_browser_family(
    family: shrimply_inspector_core::font_cache::FontFamily,
    edit: PendingFontEdit,
) -> Result<(), String> {
    if PENDING_FONT_EDIT.with_borrow(Option::is_some) {
        return Err("A font is already being activated".to_string());
    }
    FONT_BROWSER.with_borrow_mut(|browser| browser.activate(family))?;
    PENDING_FONT_EDIT.with_borrow_mut(|pending| *pending = Some(edit));
    Ok(())
}

fn ensure_font_control(
    controller: &InspectorController,
    target: &InspectorTarget,
    path: &str,
    modifier_id: Option<uuid::Uuid>,
) -> Result<(), String> {
    let available = if let Some(modifier_id) = modifier_id {
        controller
            .text_3d_presentation(target, modifier_id)?
            .controls
            .iter()
            .any(|control| {
                control.kind == ControlKind::FontFamilies
                    && control.target_id == Some(modifier_id)
                    && control.path == path
            })
    } else {
        let snapshot = controller.snapshot();
        (&snapshot.target == target)
            && snapshot.video.is_some_and(|video| {
                video.visual.iter().any(|card| {
                    card.section.controls.iter().any(|control| {
                        control.kind == ControlKind::FontFamilies
                            && control.target_id.is_none()
                            && control.path == path
                    })
                })
            })
    };
    available
        .then_some(())
        .ok_or_else(|| "text font control is no longer available".to_string())
}

fn apply_font_browser_edit(edit: &PendingFontEdit) -> Result<(), String> {
    with_controller(|controller| {
        if controller.target() != edit.target {
            return Ok(());
        }
        ensure_font_control(controller, &edit.target, &edit.path, edit.modifier_id)?;
        let unchanged = if let Some(modifier_id) = edit.modifier_id {
            controller
                .text_3d_presentation(&edit.target, modifier_id)?
                .controls
                .iter()
                .any(|control| control.path == edit.path && control.value == edit.source_value)
        } else {
            controller.snapshot().video.is_some_and(|video| {
                video.visual.iter().any(|card| {
                    card.section.controls.iter().any(|control| {
                        control.path == edit.path && control.value == edit.source_value
                    })
                })
            })
        };
        if !unchanged {
            return Ok(());
        }
        controller.set_video_field(
            &edit.target,
            &edit.path,
            &edit.value,
            &edit.commit_name,
            true,
        )
    })
}

fn receive_font_browser() -> FontBrowserPoll {
    let poll = FONT_BROWSER.with_borrow_mut(|browser| browser.poll());
    let edits = poll
        .activations
        .into_iter()
        .filter_map(|result| {
            PENDING_FONT_EDIT
                .with_borrow_mut(Option::take)
                .map(|edit| (edit, result.map(drop)))
        })
        .collect();
    (poll.changed, edits)
}

fn cancel_font_browser_edit() {
    PENDING_FONT_EDIT.with_borrow_mut(|pending| drop(pending.take()));
}

fn validate_preview_focus(document: &InspectorDocument) {
    let focused = PREVIEW_FOCUS.with_borrow(|preview_focus| {
        preview_focus
            .as_ref()
            .and_then(shrimply_editor_state::preview_focus::snapshot)
    });
    let valid = focused.as_ref().is_none_or(|focused| {
        document.preview_item.as_ref().is_some_and(|preview| {
            CONTROLLER.with_borrow(|controller| {
                controller
                    .as_ref()
                    .expect("Qt inspector preview validation requested before installation")
                    .valid_preview_focus(&focused.item, focused.target, &preview.address)
            })
        })
    });
    if !valid {
        PREVIEW_FOCUS.with_borrow(|preview_focus| {
            shrimply_editor_state::preview_focus::clear(
                preview_focus
                    .as_ref()
                    .expect("Qt inspector preview focus requested before installation"),
            );
        });
    }
}

fn cache_status(kind: CacheKind, id: uuid::Uuid) -> shrimply_inspector_core::CacheStatus {
    let status = current_cache_status(kind.clone(), id);
    CACHE_STATUSES.with_borrow_mut(|statuses| statuses.observe(kind, id, status))
}

fn current_cache_status(kind: CacheKind, id: uuid::Uuid) -> shrimply_inspector_core::CacheStatus {
    match kind {
        CacheKind::Audio => shrimply_inspector_core::audio_cache_status(id),
        CacheKind::Visual(address) => shrimply_inspector_core::visual_cache_status(&address, id),
    }
}

fn cache_kind(
    control: crate::section::ControlKind,
    target: Option<&InspectorTarget>,
) -> Option<CacheKind> {
    match control {
        crate::section::ControlKind::AudioCache | crate::section::ControlKind::AudioCachePreset => {
            Some(CacheKind::Audio)
        }
        crate::section::ControlKind::VisualCache
        | crate::section::ControlKind::VisualCacheQuality => match target {
            Some(InspectorTarget::Item(
                address @ shrimply_project_document::project::ItemAddress::Video { .. },
            )) => Some(CacheKind::Visual(address.clone())),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn tracked_cache_control(
    control: crate::section::ControlKind,
    id: uuid::Uuid,
    target: Option<&InspectorTarget>,
) -> Option<shrimply_inspector_core::CacheControlPresentation> {
    let kind = cache_kind(control, target)?;
    let tracked =
        CACHE_STATUSES.with_borrow(|statuses| statuses.tracked(kind.clone(), id).cloned());
    let status = tracked.unwrap_or_else(|| cache_status(kind.clone(), id));
    Some(match kind {
        CacheKind::Audio => shrimply_inspector_core::audio_cache_control(status),
        CacheKind::Visual(_) => shrimply_inspector_core::cache_control_presentation(status, ""),
    })
}

fn receive_cache_statuses() {
    let poll = CACHE_STATUSES.with_borrow_mut(|statuses| statuses.poll(current_cache_status));
    let audio_terminal = poll.finished.contains(&CacheKind::Audio);
    let visual_terminal = poll
        .finished
        .iter()
        .any(|kind| matches!(kind, CacheKind::Visual(_)));
    if audio_terminal || visual_terminal {
        with_controller(|controller| {
            if audio_terminal {
                controller.refresh_audio_cache();
            }
            if visual_terminal {
                controller.refresh_visual_cache();
            }
            Ok(())
        })
        .expect("Qt inspector cache refresh requested before installation");
    } else if poll.changed {
        CACHE_DIRTY.set(true);
    }
}

fn retain_cache_statuses(document: &InspectorDocument) {
    let mut ids = document
        .categories
        .iter()
        .flat_map(|category| &category.items)
        .flat_map(|item| match item {
            crate::item::InspectorListItem::Item(item) => item.section.controls.as_slice(),
            crate::item::InspectorListItem::Flat(section) => section.controls.as_slice(),
        })
        .filter_map(|control| {
            Some((
                cache_kind(control.kind, Some(&document.target))?,
                control.target_id?,
            ))
        })
        .fold(Vec::new(), |mut ids, entry| {
            if !ids.contains(&entry) {
                ids.push(entry);
            }
            ids
        });
    CACHE_STATUSES.with_borrow_mut(|statuses| {
        statuses.retain(|kind, id| ids.contains(&(kind, id)));
        for (kind, id) in ids.drain(..) {
            let status = current_cache_status(kind.clone(), id);
            statuses.observe(kind, id, status);
        }
    });
}

fn take_cache_dirty() -> bool {
    CACHE_DIRTY.replace(false)
}

fn take_expression_dirty() -> bool {
    EXPRESSION_DIRTY.replace(false)
}

fn take_graph_dirty() -> bool {
    GRAPH_DIRTY.replace(false)
}

fn take_playhead_dirty() -> bool {
    PLAYHEAD_DIRTY.replace(false)
}

fn take_transform_dirty() -> bool {
    TRANSFORM_DIRTY.replace(false)
}

fn mark_transform_dirty() {
    TRANSFORM_DIRTY.set(true);
}

fn take_focus_dirty() -> bool {
    FOCUS_DIRTY.replace(false)
}

fn item_focused(document: &InspectorDocument, item: &crate::item::InspectorItem) -> bool {
    let Some(preview) = &document.preview_item else {
        return false;
    };
    PREVIEW_FOCUS.with_borrow(|preview_focus| {
        preview_focus
            .as_ref()
            .and_then(shrimply_editor_state::preview_focus::snapshot)
            .is_some_and(|focused| {
                focused.item == preview.address && focused.card_key == item.presentation.key
            })
    })
}

fn focus_item(document: &InspectorDocument, item: &crate::item::InspectorItem) {
    let Some(preview) = &document.preview_item else {
        return;
    };
    PREVIEW_FOCUS.with_borrow(|preview_focus| {
        shrimply_editor_state::preview_focus::set(
            preview_focus
                .as_ref()
                .expect("Qt inspector preview focus requested before installation"),
            shrimply_editor_state::preview_focus::FocusedPreview {
                item: preview.address.clone(),
                card_key: item.presentation.key.clone(),
                target: item.presentation.preview_target.resolve(preview.id),
            },
        );
    });
}

fn focus_control(
    document: &InspectorDocument,
    item: &crate::item::InspectorItem,
    control: &section::InspectorControl,
) {
    let Some(focus) = &control.preview_focus else {
        focus_item(document, item);
        return;
    };
    let Some(preview) = &document.preview_item else {
        return;
    };
    PREVIEW_FOCUS.with_borrow(|preview_focus| {
        shrimply_editor_state::preview_focus::set(
            preview_focus
                .as_ref()
                .expect("Qt inspector preview focus requested before installation"),
            shrimply_editor_state::preview_focus::FocusedPreview {
                item: preview.address.clone(),
                card_key: focus.card_key.clone(),
                target: focus.target.resolve(preview.id),
            },
        );
    });
}

fn focus_alpha_mask(
    target: &InspectorTarget,
    mask_target: shrimply_project_document::project::VisualAlphaMaskTarget,
    enabled: bool,
) {
    let InspectorTarget::Item(
        address @ shrimply_project_document::project::ItemAddress::Video { .. },
    ) = target
    else {
        return;
    };
    let item_id = CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector alpha-mask focus requested before installation")
            .snapshot()
            .video
            .expect("video inspector snapshot must include video presentation")
            .item_id
    });
    let focus = shrimply_inspector_core::alpha_mask::preview_focus(item_id, mask_target, enabled);
    PREVIEW_FOCUS.with_borrow(|preview_focus| {
        shrimply_editor_state::preview_focus::set(
            preview_focus
                .as_ref()
                .expect("Qt inspector preview focus requested before installation"),
            shrimply_editor_state::preview_focus::FocusedPreview {
                item: address.clone(),
                card_key: focus.card_key,
                target: focus.target.resolve(item_id),
            },
        );
    });
}

fn keyframe_snapping() -> (bool, f64) {
    PREFERENCES.with_borrow(|preferences| {
        shrimply_inspector_core::keyframe_model::graph_snapping(
            preferences
                .as_ref()
                .expect("Qt inspector keyframe preferences requested before installation"),
        )
    })
}

fn trigger_video_control_action(
    target: &InspectorTarget,
    action: shrimply_inspector_core::InspectorControlAction,
) -> Result<(), String> {
    with_controller(|controller| controller.trigger_video_control_action(target, action))
}

fn select_object_3d_model(target: &InspectorTarget, modifier_id: uuid::Uuid) -> Result<(), String> {
    let Some(path) = shrimply_components_qt::file_picker::open(
        "select-3d-object-model",
        "Select 3D model",
        "3D models (*.obj *.glb)",
    ) else {
        return Ok(());
    };
    with_controller(|controller| controller.set_object_3d_model(target, modifier_id, &path))
}

fn select_scene_3d_environment(target: &InspectorTarget) -> Result<(), String> {
    let Some(path) = shrimply_components_qt::file_picker::open(
        "select-scene-3d-environment",
        "Select environment image",
        "Environment images (*.png *.jpg *.jpeg *.webp *.avif *.hdr *.exr)",
    ) else {
        return Ok(());
    };
    with_controller(|controller| controller.set_scene_3d_environment(target, &path))
}

fn select_paint_texture(target: &InspectorTarget, color_id: uuid::Uuid) -> Result<(), String> {
    let Some(path) = shrimply_components_qt::file_picker::open(
        "select-paint-texture",
        "Select paint texture",
        "Images (*.png *.jpg *.jpeg *.webp *.gif *.bmp *.tif *.tiff)",
    ) else {
        return Ok(());
    };
    with_controller(|controller| controller.set_paint_texture(target, color_id, &path))
}

fn paint_drawing_expression_output(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<String>, String> {
    with_controller(|controller| controller.paint_drawing_expression_output(target, timeline_id))
}

fn video_stabilization_generating(target: &InspectorTarget) -> Option<bool> {
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector stabilization state requested before installation")
            .video_stabilization_generating(target)
    })
}

fn control_graph(
    target: &InspectorTarget,
    control: &shrimply_inspector_core::InspectorControl,
) -> Result<Option<shrimply_inspector_core::ScalarGraph>, String> {
    with_controller(|controller| controller.control_graph(target, control))
}

fn control_graph_source(
    target: &InspectorTarget,
) -> Result<(serde_json::Value, shrimply_inspector_core::InspectorRuntime), String> {
    with_controller(|controller| controller.control_graph_source(target))
}

fn finish_live_edit() {
    with_controller(|controller| {
        controller.finish_live_edit();
        Ok(())
    })
    .expect("Qt inspector live edit finished before installation");
}

fn with_controller<T>(
    operation: impl FnOnce(&InspectorController) -> Result<T, String>,
) -> Result<T, String> {
    CONTROLLER.with_borrow(|controller| {
        operation(
            controller
                .as_ref()
                .expect("Qt inspector edit requested before installation"),
        )
    })
}
