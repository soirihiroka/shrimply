mod action_controller;
mod audio;
mod audio_generator;
mod audio_modifiers;
mod backend;
mod caption;
mod graph_backend;
mod info;
mod item;
mod list;
mod modifiers;
mod project;
mod section;
mod selector;
mod timeline_controller;
mod track;
mod transition;
mod value_backend;
mod video;

use serde_json::Value;
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::{preferences::SharedPreferences, preview_focus::SharedPreviewFocus};
use shrimply_inspector_core::{
    ControlKind, InspectorController, InspectorSnapshot, InspectorTarget,
};
use shrimply_project_document::project::{ItemAddress, Time};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use crate::item::InspectorAction;
use crate::list::{InspectorDocument, PreviewItem};
use action_controller::*;
use timeline_controller::*;

const VOICE_MODEL_RETRY_INTERVAL: Duration = Duration::from_secs(5);

thread_local! {
    static CONTROLLER: RefCell<Option<InspectorController>> = const { RefCell::new(None) };
    static PROPERTY_CLIPBOARD: RefCell<Option<shrimply_property_transfer::SharedClipboard>> = const { RefCell::new(None) };
    static PREFERENCES: RefCell<Option<SharedPreferences>> = const { RefCell::new(None) };
    static PREVIEW_FOCUS: RefCell<Option<SharedPreviewFocus>> = const { RefCell::new(None) };
    static PROJECT_FONTS: RefCell<Option<shrimply_inspector_core::font_cache::ProjectFontActivation>> = const { RefCell::new(None) };
    static FONT_BROWSER: RefCell<shrimply_inspector_core::font_selector::Browser> = RefCell::new(shrimply_inspector_core::font_selector::Browser::default());
    static PENDING_FONT_EDIT: RefCell<Option<PendingFontEdit>> = const { RefCell::new(None) };
    static MEDIA_METADATA: RefCell<MediaMetadata> = RefCell::new(MediaMetadata::default());
    static VOICE_MODELS: RefCell<VoiceModelCache> = RefCell::new(VoiceModelCache::default());
    static CAMERA_MODELS: RefCell<CameraModelRequests> = RefCell::new(CameraModelRequests::default());
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

struct MediaMetadata {
    sender: mpsc::Sender<(MediaMetadataKey, Result<Arc<CachedMediaInfo>, String>)>,
    receiver: mpsc::Receiver<(MediaMetadataKey, Result<Arc<CachedMediaInfo>, String>)>,
    active: Option<MediaMetadataKey>,
    pending: Vec<MediaMetadataKey>,
    result: Option<(MediaMetadataKey, Result<Arc<CachedMediaInfo>, String>)>,
}

#[derive(Clone)]
pub(crate) enum MediaMetadataState {
    Loading,
    Ready(Arc<CachedMediaInfo>),
    Failed(String),
}

pub(crate) struct CachedMediaInfo {
    presentation: shrimply_inspector_core::info::MediaInfoPresentation,
    artwork_url: Option<String>,
    audio_stream_count: u32,
    video_stream_count: u32,
}

pub(crate) struct VoiceModels {
    pub(crate) values: Vec<String>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
}

#[derive(Clone, Eq, PartialEq)]
enum CacheKind {
    Audio,
    Visual(shrimply_project_document::project::ItemAddress),
}

#[derive(Clone, Eq, PartialEq)]
struct VoiceModelKey {
    server_url: String,
}

struct VoiceModelCache {
    sender: mpsc::Sender<(VoiceModelKey, Result<Vec<String>, String>)>,
    receiver: mpsc::Receiver<(VoiceModelKey, Result<Vec<String>, String>)>,
    pending: Vec<VoiceModelKey>,
    results: Vec<(VoiceModelKey, Result<Vec<String>, String>, Instant)>,
}

struct CameraModelRequests {
    sender: mpsc::Sender<String>,
    receiver: mpsc::Receiver<String>,
    pending: Vec<String>,
}

impl Default for CameraModelRequests {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            pending: Vec::new(),
        }
    }
}

impl Default for VoiceModelCache {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            pending: Vec::new(),
            results: Vec::new(),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct MediaMetadataKey {
    path: PathBuf,
    revision: Option<u64>,
}

impl Default for MediaMetadata {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active: None,
            pending: Vec::new(),
            result: None,
        }
    }
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
            .with_default_text_font(default_font.clone()),
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
    audio::poll_tts_runtime();
    receive_pdf_pages();
    receive_project_fonts();
    receive_media_metadata();
    if shrimply_inspector_core::video::blender::poll_metadata() {
        mark_dirty();
    }
    if shrimply_inspector_core::manim_parameters::poll_scenes() {
        mark_dirty();
    }
    if CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .is_some_and(InspectorController::poll_sanim_parameters)
    }) {
        mark_dirty();
    }
    receive_voice_models();
    receive_camera_models();
    receive_cache_statuses();
    if !DIRTY.replace(false) {
        return None;
    }
    CONTROLLER.with_borrow(|controller| {
        let controller = controller
            .as_ref()
            .expect("Qt inspector snapshot requested before installation");
        controller.retain_analysis_transitions();
        let (server_url, camera_models) = PREFERENCES.with_borrow(|preferences| {
            let server_url = shrimply_editor_state::preferences::snapshot(
                preferences
                    .as_ref()
                    .expect("Qt inspector preferences requested before installation"),
            )
            .compute_server_url;
            let models =
                shrimply_inspector_core::camera_source::cached_tracking_models(&server_url);
            (server_url, models)
        });
        let mut snapshot = controller.snapshot_with_camera_models(camera_models.as_ref());
        if camera_models.is_none()
            && snapshot.video.as_ref().is_some_and(|video| {
                video.visual.iter().any(|card| {
                    card.section.controls.iter().any(|control| {
                        control.path == shrimply_inspector_core::camera_source::SOURCE_PATH
                    })
                })
            })
        {
            request_camera_models(&server_url);
        }
        let blender_metadata = blender_metadata(snapshot.video.as_ref());
        if let Some(shrimply_inspector_core::video::blender::MetadataState::Ready(metadata)) =
            blender_metadata.as_ref()
            && controller
                .sync_blender_metadata(&snapshot.target, metadata)
                .unwrap_or_else(|error| panic!("could not synchronize Blender metadata: {error}"))
        {
            snapshot = controller.snapshot_with_camera_models(camera_models.as_ref());
        }
        if let Some(manim) = snapshot
            .video
            .as_ref()
            .and_then(|video| video.manim.as_ref())
            && let Some(scene) = manim.main.section.controls.iter().find(|control| {
                control.path == shrimply_inspector_core::manim_parameters::SCENE_PATH
            })
            && scene.sensitive
            && scene.value != manim.current_scene
        {
            controller
                .set_manim_scene(&snapshot.target, scene.value.clone(), "select-manim-scene")
                .unwrap_or_else(|error| panic!("could not select default Manim scene: {error}"));
            snapshot = controller.snapshot_with_camera_models(camera_models.as_ref());
        }
        let document = document(snapshot, blender_metadata);
        retain_cache_statuses(&document);
        Some(document)
    })
}

fn receive_pdf_pages() {
    if !shrimply_inspector_core::video::pdf::poll_pages() {
        return;
    }
    CONTROLLER.with_borrow(|controller| {
        let controller = controller
            .as_ref()
            .expect("Qt PDF pages received before inspector installation");
        controller
            .normalize_pdf_page(&controller.target())
            .unwrap_or_else(|error| panic!("could not normalize Qt PDF page: {error}"));
    });
    mark_dirty();
}

fn blender_metadata(
    video: Option<&shrimply_inspector_core::VideoPresentation>,
) -> Option<shrimply_inspector_core::video::blender::MetadataState> {
    let source = video?.blender.as_ref()?;
    let asset =
        shrimply_project_document::project::Asset::from(std::path::Path::new(&source.asset));
    let binary = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt inspector preferences requested before installation"),
        )
        .blender_binary
    });
    Some(shrimply_inspector_core::video::blender::metadata(
        &asset,
        binary.as_deref(),
    ))
}

fn receive_camera_models() {
    CAMERA_MODELS.with_borrow_mut(|requests| {
        while let Ok(server_url) = requests.receiver.try_recv() {
            requests.pending.retain(|pending| pending != &server_url);
            mark_dirty();
        }
    });
}

fn request_camera_models(server_url: &str) {
    if shrimply_inspector_core::camera_source::cached_tracking_models(server_url).is_some() {
        return;
    }
    CAMERA_MODELS.with_borrow_mut(|requests| {
        if requests.pending.iter().any(|pending| pending == server_url) {
            return;
        }
        requests.pending.push(server_url.to_string());
        let sender = requests.sender.clone();
        let request_url = server_url.to_string();
        thread::spawn(move || {
            let _ = shrimply_inspector_core::camera_source::tracking_models(&request_url);
            let _ = sender.send(request_url);
        });
    });
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

fn document(
    snapshot: InspectorSnapshot,
    blender_metadata: Option<shrimply_inspector_core::video::blender::MetadataState>,
) -> InspectorDocument {
    let preview_item = match &snapshot.target {
        InspectorTarget::Item(address @ ItemAddress::Video { .. }) => Some(PreviewItem {
            address: address.clone(),
            id: snapshot
                .value
                .get("id")
                .and_then(Value::as_str)
                .expect("video inspector ID must be text")
                .parse()
                .expect("video inspector ID must be a UUID"),
        }),
        _ => None,
    };
    let metadata = media_metadata(snapshot.media.as_ref());
    let categories = match &snapshot.target {
        InspectorTarget::Project => project::categories(
            snapshot
                .project
                .as_ref()
                .expect("project inspector snapshot must include project presentation"),
        ),
        InspectorTarget::Track(_) => track::categories(
            snapshot
                .track
                .as_ref()
                .expect("track inspector snapshot must include track presentation"),
        ),
        InspectorTarget::Transition { .. } => transition::categories(
            snapshot
                .transition
                .as_ref()
                .expect("transition inspector snapshot must include transition presentation"),
        ),
        InspectorTarget::Item(address) => match address {
            ItemAddress::Comment { .. } => vec![list::InspectorCategory {
                key: "comment",
                label: "Comment",
                icon: "rich-text-symbolic",
                items: vec![
                    item::InspectorItem::new(
                        "comment",
                        "Comment",
                        shrimply_inspector_core::comment::section(
                            &serde_json::from_value(snapshot.value.clone())
                                .expect("comment must be valid"),
                        ),
                    )
                    .boxed(),
                ],
            }],
            ItemAddress::Caption { .. } => caption::categories(&snapshot.value, &snapshot.details),
            ItemAddress::Audio { .. } => audio::categories(
                &snapshot.value,
                &snapshot.details,
                metadata,
                audio_gain(&snapshot),
                snapshot.runtime,
                can_paste_audio_modifiers(&snapshot.target),
            ),
            ItemAddress::Video { .. } => video::categories(
                snapshot
                    .video
                    .as_ref()
                    .expect("video inspector snapshot must include video presentation"),
                &snapshot.details,
                metadata,
                blender_metadata,
                can_paste_visual_modifiers(&snapshot.target),
            ),
        },
    };
    let document = InspectorDocument {
        target: snapshot.target,
        title: snapshot.title,
        categories,
        preview_item,
    };
    validate_preview_focus(&document);
    document
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

fn receive_media_metadata() {
    let changed = MEDIA_METADATA.with_borrow_mut(|metadata| {
        let results = metadata.receiver.try_iter().collect::<Vec<_>>();
        let mut changed = false;
        for (key, result) in results {
            metadata.pending.retain(|pending| pending != &key);
            if metadata.active.as_ref() == Some(&key) {
                metadata.result = Some((key, result));
                changed = true;
            }
        }
        changed
    });
    if changed {
        mark_dirty();
    }
}

fn receive_voice_models() {
    let changed = VOICE_MODELS.with_borrow_mut(|cache| {
        let results = cache.receiver.try_iter().collect::<Vec<_>>();
        for (key, result) in &results {
            cache.pending.retain(|pending| pending != key);
            if let Some((_, cached, completed)) = cache
                .results
                .iter_mut()
                .find(|(cached, _, _)| cached == key)
            {
                *cached = result.clone();
                *completed = Instant::now();
            } else {
                cache
                    .results
                    .push((key.clone(), result.clone(), Instant::now()));
            }
        }
        let mut changed = !results.is_empty();
        cache.results.retain(|(_, result, completed)| {
            let retry = result.is_err() && completed.elapsed() >= VOICE_MODEL_RETRY_INTERVAL;
            changed |= retry;
            !retry
        });
        changed
    });
    if changed {
        mark_dirty();
    }
}

pub(crate) fn voice_models(current: &str) -> VoiceModels {
    let server_url = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt inspector voice models requested before installation"),
        )
        .compute_server_url
    });
    let key = VoiceModelKey { server_url };
    VOICE_MODELS.with_borrow_mut(|cache| {
        if let Some(index) = cache
            .results
            .iter()
            .position(|(cached, _, _)| cached == &key)
        {
            let retry = cache.results[index].1.is_err()
                && cache.results[index].2.elapsed() >= VOICE_MODEL_RETRY_INTERVAL;
            if retry {
                drop(cache.results.remove(index));
            } else {
                return match &cache.results[index].1 {
                    Ok(values) => VoiceModels {
                        values: voice_models_with_current(values.clone(), current),
                        error: None,
                        loading: false,
                    },
                    Err(error) => VoiceModels {
                        values: vec![current.to_string()],
                        error: Some(error.clone()),
                        loading: false,
                    },
                };
            }
        }
        if cache.pending.iter().all(|pending| pending != &key) {
            cache.pending.push(key.clone());
            let sender = cache.sender.clone();
            let request = key.clone();
            thread::spawn(move || {
                let result =
                    shrimply_inspector_core::voice_change_model_catalog(&request.server_url);
                let _ = sender.send((request, result));
            });
        }
        VoiceModels {
            values: vec![current.to_string()],
            error: None,
            loading: true,
        }
    })
}

fn voice_models_with_current(mut values: Vec<String>, current: &str) -> Vec<String> {
    if !values.iter().any(|value| value == current) {
        values.insert(0, current.to_string());
    }
    values
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

fn media_metadata(
    media: Option<&shrimply_inspector_core::InspectorMedia>,
) -> Option<MediaMetadataState> {
    MEDIA_METADATA.with_borrow_mut(|metadata| {
        let Some(media) = media else {
            metadata.active = None;
            metadata.result = None;
            return None;
        };
        let key = MediaMetadataKey {
            path: media.path.clone(),
            revision: media.revision,
        };
        if metadata.active.as_ref() != Some(&key) {
            metadata.active = Some(key.clone());
            metadata.result = None;
        }
        if let Some((_, result)) = metadata
            .result
            .as_ref()
            .filter(|(cached, _)| cached == &key)
        {
            return Some(match result {
                Ok(info) => MediaMetadataState::Ready(info.clone()),
                Err(error) => MediaMetadataState::Failed(error.clone()),
            });
        }
        if metadata.pending.iter().all(|pending| pending != &key) {
            metadata.pending.push(key.clone());
            let sender = metadata.sender.clone();
            thread::spawn(move || {
                let result = shrimply_media_info::inspect(&key.path, key.revision)
                    .map(info::cache_media_info);
                let _ = sender.send((key, result));
            });
        }
        Some(MediaMetadataState::Loading)
    })
}

fn audio_gain(snapshot: &InspectorSnapshot) -> f32 {
    let item: shrimply_project_document::project::AudioItem =
        serde_json::from_value(snapshot.value.clone())
            .expect("audio inspector value must be valid");
    item.gain.decibels.value_at(
        snapshot
            .runtime
            .local_time
            .unwrap_or(shrimply_project_document::project::Time::ZERO),
    )
}

fn set_field(target: &InspectorTarget, path: &str, value: &str) -> Result<(), String> {
    with_controller(|controller| controller.set_field(target, path, value))
}

fn set_transition_field(
    target: &InspectorTarget,
    path: &str,
    value: &str,
    commit_name: &str,
    commit_immediately: bool,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_transition_field(target, path, value, commit_name, commit_immediately)
    })
}

fn commit_transition_field(target: &InspectorTarget, commit_name: &str) -> Result<(), String> {
    with_controller(|controller| controller.commit_transition_field(target, commit_name))
}

fn set_video_field(
    target: &InspectorTarget,
    path: &str,
    value: &str,
    commit_name: &str,
    commit_immediately: bool,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_video_field(target, path, value, commit_name, commit_immediately)
    })
}

fn set_video_fraction(
    target: &InspectorTarget,
    path: &str,
    numerator: i64,
    denominator: i64,
    commit_name: &str,
) -> Result<(), String> {
    if denominator <= 0 {
        return Err("video fraction denominator must be positive".to_string());
    }
    with_controller(|controller| {
        controller.set_video_fraction(
            target,
            path,
            shrimply_math_core::fraction_new(numerator, denominator),
            commit_name,
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

fn commit_control_edit(
    target: &InspectorTarget,
    control: &section::InspectorControl,
) -> Result<(), String> {
    if crate::graph_backend::background_integer(control) {
        let timeline_id = control
            .timeline_id
            .ok_or_else(|| "background integer timeline ID is unavailable".to_string())?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        commit_background_integer_value(target, path, timeline_id)
    } else if matches!(
        control.kind,
        section::ControlKind::LayeredNumber
            | section::ControlKind::LayeredSelector
            | section::ControlKind::LayeredVector2
            | section::ControlKind::LayeredVector3
            | section::ControlKind::LayeredText
            | section::ControlKind::LayeredDrawing
    ) {
        with_controller(|controller| controller.finish_live_inspector_edit(target))
    } else if matches!(target, InspectorTarget::Transition { .. })
        && !control.commit_name.is_empty()
        && !control.commit_immediately
    {
        commit_transition_field(target, &control.commit_name)
    } else if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
        && !control.commit_name.is_empty()
        && !control.commit_immediately
    {
        with_controller(|controller| controller.commit_video_field(target, &control.commit_name))
    } else {
        finish_live_edit();
        Ok(())
    }
}

fn named_control_edit(
    target: &InspectorTarget,
    control: &section::InspectorControl,
    value: String,
) -> Option<Result<(), String>> {
    if control.commit_name.is_empty()
        || matches!(
            control.kind,
            section::ControlKind::LayeredNumber
                | section::ControlKind::LayeredBoolean
                | section::ControlKind::LayeredSelector
        )
    {
        return None;
    }
    if let Some(result) = alpha_mask_control_edit(target, control, &value) {
        return Some(result);
    }
    let value = if control.kind == section::ControlKind::Number
        && (control.store_multiplier != 1.0
            || control.number_mapping != shrimply_inspector_core::NumberMapping::Linear)
    {
        value
            .parse::<f64>()
            .map(|value| control.store_number(value).to_string())
            .map_err(|_| format!("invalid numeric inspector value: {value}"))
    } else {
        Ok(value)
    };
    if matches!(target, InspectorTarget::Transition { .. }) {
        Some(value.and_then(|value| {
            set_transition_field(
                target,
                &control.path,
                &value,
                &control.commit_name,
                control.commit_immediately,
            )
        }))
    } else if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. })) {
        Some(value.and_then(|value| {
            set_video_field(
                target,
                &control.path,
                &value,
                &control.commit_name,
                control.commit_immediately,
            )
        }))
    } else {
        None
    }
}

fn alpha_mask_control_edit(
    target: &InspectorTarget,
    control: &section::InspectorControl,
    value: &str,
) -> Option<Result<(), String>> {
    let field = control.path.rsplit_once("/alpha_mask/")?.1;
    let mask_target = if control.path.starts_with("/compositing/alpha_mask/") {
        shrimply_project_document::project::VisualAlphaMaskTarget::Compositing
    } else if control.path.starts_with("/modifiers/") {
        let Some(id) = control.target_id else {
            return Some(Err("alpha-mask modifier target is unavailable".to_string()));
        };
        shrimply_project_document::project::VisualAlphaMaskTarget::Modifier(id)
    } else {
        return None;
    };
    match field {
        "shape" => {
            let Some(shape) = shrimply_inspector_core::alpha_mask::SHAPE_CHOICES
                .iter()
                .find_map(|(shape, key, _)| (*key == value).then_some(*shape))
            else {
                return Some(Err(format!("invalid alpha-mask shape: {value}")));
            };
            Some(with_controller(|controller| {
                controller.set_alpha_mask_shape(target, mask_target, shape)
            }))
        }
        "invert" => Some(
            value
                .parse::<bool>()
                .map_err(|_| format!("invalid alpha-mask inversion: {value}"))
                .and_then(|invert| {
                    with_controller(|controller| {
                        controller.set_alpha_mask_inverted(target, mask_target, invert)
                    })
                }),
        ),
        _ => None,
    }
}

fn set_control_fraction(
    target: &InspectorTarget,
    control: &section::InspectorControl,
    numerator: i64,
    denominator: i64,
) -> Result<(), String> {
    if denominator <= 0 {
        return Err("fraction denominator must be positive".to_string());
    }
    if let Some(result) = with_controller(|controller| {
        Ok(controller.set_manim_fraction(
            target,
            &control.path,
            shrimply_math_core::fraction_new(numerator, denominator),
            &control.commit_name,
        ))
    })? {
        return result;
    }
    if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
        && !control.commit_name.is_empty()
    {
        set_video_fraction(
            target,
            &control.path,
            numerator,
            denominator,
            &control.commit_name,
        )
    } else {
        set_fraction(target, &control.path, numerator, denominator)
    }
}

fn set_manim_text_field(
    target: &InspectorTarget,
    path: &str,
    value: &str,
    commit_name: &str,
) -> Option<Result<(), String>> {
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt Manim edit requested before installation")
            .set_manim_text_field(target, path, value, commit_name)
    })
}

fn set_optional_field(
    target: &InspectorTarget,
    path: &str,
    value: Option<&str>,
) -> Result<(), String> {
    with_controller(|controller| controller.set_optional_field(target, path, value))
}

fn set_optional_number_field(
    target: &InspectorTarget,
    path: &str,
    value: Option<&str>,
) -> Result<(), String> {
    with_controller(|controller| controller.set_optional_number_field(target, path, value))
}

fn set_components(
    target: &InspectorTarget,
    path: &str,
    values: &[(usize, String)],
) -> Result<(), String> {
    with_controller(|controller| controller.set_components(target, path, values))
}

fn ensure_control_timeline(
    target: &InspectorTarget,
    control: &section::InspectorControl,
) -> Result<(), String> {
    let path = control.timeline_path.as_deref().unwrap_or(&control.path);
    let layered = matches!(
        control.kind,
        section::ControlKind::LayeredNumber
            | section::ControlKind::LayeredSelector
            | section::ControlKind::LayeredVector2
            | section::ControlKind::LayeredVector3
            | section::ControlKind::LayeredColor
            | section::ControlKind::LayeredText
            | section::ControlKind::LayeredDrawing
    );
    if path.starts_with("/content/generator/") {
        if path == "/content/generator/kind" && !layered {
            return Ok(());
        }
        return control
            .timeline_id
            .ok_or_else(|| "background timeline ID is unavailable".to_string())
            .and_then(|timeline_id| {
                with_controller(|controller| controller.ensure_timeline(target, path, timeline_id))
            });
    }
    if path.contains("/alpha_mask/") && layered {
        if let Some(modifier_id) = control.target_id {
            with_controller(|controller| {
                controller.ensure_visual_modifier(target, path, modifier_id)
            })?;
        }
        return control
            .timeline_id
            .ok_or_else(|| "alpha-mask timeline ID is unavailable".to_string())
            .and_then(|timeline_id| {
                with_controller(|controller| controller.ensure_timeline(target, path, timeline_id))
            });
    }
    if !path.starts_with("/modifiers/") {
        if shrimply_inspector_core::paint::is_timeline_path(path) && layered {
            return control
                .timeline_id
                .ok_or_else(|| "paint timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    with_controller(|controller| {
                        controller.ensure_timeline(target, path, timeline_id)
                    })
                });
        }
        if shrimply_inspector_core::generated::is_timeline_control(control) {
            return control
                .timeline_id
                .ok_or_else(|| "generated timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    with_controller(|controller| {
                        controller.ensure_timeline(target, path, timeline_id)
                    })
                });
        }
        if (path.starts_with("/content/model/") || path.starts_with("/content/camera/")) && layered
        {
            return control
                .timeline_id
                .ok_or_else(|| "Gaussian timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    with_controller(|controller| {
                        controller.ensure_timeline(target, path, timeline_id)
                    })
                });
        }
        if shrimply_inspector_core::scene_3d::is_timeline_path(path) && layered {
            return control
                .timeline_id
                .ok_or_else(|| "scene 3D timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    with_controller(|controller| {
                        controller.ensure_timeline(target, path, timeline_id)
                    })
                });
        }
        if matches!(
            (
                shrimply_inspector_core::transform::TransformField::from_path(path),
                control.kind,
            ),
            (
                Some(shrimply_inspector_core::transform::TransformField::Vec2(_)),
                section::ControlKind::LayeredVector2,
            ) | (
                Some(shrimply_inspector_core::transform::TransformField::Scalar(
                    _
                )),
                section::ControlKind::LayeredNumber,
            )
        ) {
            return control
                .timeline_id
                .ok_or_else(|| "transform timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    with_controller(|controller| {
                        controller.ensure_timeline(target, path, timeline_id)
                    })
                });
        }
        return Ok(());
    }
    if let Some(modifier_id) = control.target_id {
        with_controller(|controller| controller.ensure_visual_modifier(target, path, modifier_id))?;
    }
    if !layered {
        return Ok(());
    }
    let timeline_id = control
        .timeline_id
        .ok_or_else(|| "visual modifier timeline ID is unavailable".to_string())?;
    with_controller(|controller| match control.kind {
        section::ControlKind::LayeredNumber => {
            controller.ensure_visual_modifier_number(target, path, timeline_id)
        }
        section::ControlKind::LayeredSelector => {
            controller.ensure_timeline(target, path, timeline_id)
        }
        section::ControlKind::LayeredColor => controller.ensure_timeline(target, path, timeline_id),
        section::ControlKind::LayeredText => {
            controller.ensure_visual_modifier_text(target, path, timeline_id)
        }
        section::ControlKind::LayeredVector2 => {
            controller.ensure_visual_modifier_vector2(target, path, timeline_id)
        }
        section::ControlKind::LayeredVector3 => {
            controller.ensure_visual_modifier_vector3(target, path, timeline_id)
        }
        _ => Ok(()),
    })
}

fn set_vector2_value(
    target: &InspectorTarget,
    path: &str,
    first: f64,
    second: f64,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_vector2_value(
            target,
            path,
            first,
            second,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

fn set_vector3_value(
    target: &InspectorTarget,
    path: &str,
    first: f64,
    second: f64,
    third: f64,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_vector3_value(
            target,
            path,
            first,
            second,
            third,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
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

fn set_paint_drawing_keyframes_enabled(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    enabled: bool,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_paint_drawing_keyframes_enabled(target, timeline_id, enabled)
    })
}

fn move_paint_drawing_keyframes(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    moves: &[(Time, Time)],
) -> Result<Vec<Time>, String> {
    with_controller(|controller| {
        controller.move_paint_drawing_keyframes(target, timeline_id, moves)
    })
}

fn delete_paint_drawing_keyframe(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    time: Time,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.delete_paint_drawing_keyframe(target, timeline_id, time)
    })
}

fn add_paint_drawing_keyframe(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    time: Time,
) -> Result<(), String> {
    with_controller(|controller| controller.add_paint_drawing_keyframe(target, timeline_id, time))
}

fn copy_paint_drawing_keyframes(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    times: &[Time],
) -> Result<usize, String> {
    with_controller(|controller| {
        controller.copy_paint_drawing_keyframes(target, timeline_id, times)
    })
}

fn paste_paint_drawing_keyframes(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    time: Time,
) -> Result<usize, String> {
    with_controller(|controller| {
        controller.paste_paint_drawing_keyframes(target, timeline_id, time)
    })
}

fn set_paint_drawing_interpolation(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    interpolation: usize,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_paint_drawing_interpolation(target, timeline_id, owner_id, interpolation)
    })
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
