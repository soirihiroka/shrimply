mod alpha_mask;
mod audio;
mod audio_cache;
mod background;
mod benchmarking;
mod camera_source;
mod caption;
mod comment;
mod control;
pub(crate) use shrimply_inspector_core::font_cache;
pub(crate) mod font_selector;
mod gaussian_3d;
mod generated;
mod info;
mod item;
mod keyframe_editor;
mod keyframe_graph;
pub(crate) use shrimply_inspector_core::keyframe_model;
mod list;
pub(crate) mod modifiers;
mod paint;
mod project;
mod rhai_editor;
mod scene_3d;
mod section;
mod selector;
mod timeline_value;
mod track;
mod transform;
mod transition;
mod video;

pub use shrimply_inspector_core::INSPECTOR_MIN_WIDTH;

pub fn font_family_selector(
    selected: &shrimply_project_document::project::FontFamily,
    on_change: impl Fn(shrimply_project_document::project::FontFamily) + 'static,
) -> gtk::Widget {
    font_selector::project_font_selector(selected, on_change)
}

use std::cell::{Cell, RefCell};

pub use shrimply_components_gtk::{desktop_open, skia_font, ui};
pub use shrimply_editor_state::player_state;
pub use shrimply_editor_state::preview_focus;
pub use shrimply_math_color::Color;
pub use shrimply_project_document::time_format;
pub use shrimply_project_evaluation as transform_eval;
pub use shrimply_render_core::math;
pub use shrimply_timeline_edit::selection_state;

pub mod timeline {
    pub use shrimply_components_gtk::{canvas as renderer, cursor};
}

use hashbrown::HashMap;
use std::rc::Rc;

use crate::player_state::SharedPlayerState;
use crate::preview_focus::SharedPreviewFocus;
use crate::selection_state::SharedSelectionState;
use gtk::glib;
use gtk::prelude::*;
use shrimply_editor_state::preferences::SharedPreferences;
use shrimply_inspector_core::InspectorTarget;
use shrimply_project_document::project::{ItemAddress, ItemRef, Project, Time, VideoItem};

use section::InspectorSection;

const DOCUMENT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

type KeyframeGraphViewStates =
    Rc<RefCell<HashMap<String, shrimply_components_gtk::ui::SharedFrameGraphState>>>;
pub(crate) type InspectedItem = ItemAddress;

#[derive(Clone)]
struct InspectorState {
    core: shrimply_inspector_core::InspectorController,
    listener_scope: Rc<RefCell<Rc<()>>>,
    keyframe_graph_views: KeyframeGraphViewStates,
    fallback_focus: gtk::Widget,
    rebuild_pending: Rc<Cell<bool>>,
    list_state: Rc<RefCell<shrimply_inspector_core::list::InspectorListState>>,
    volume: Rc<RefCell<shrimply_audio_engine::streaming::FrameAudioSampler>>,
    preferences: SharedPreferences,
    preview_focus: SharedPreviewFocus,
    property_clipboard: shrimply_property_transfer::SharedClipboard,
    active_inspector: Rc<RefCell<Option<InspectorTarget>>>,
}

#[derive(Clone)]
pub(crate) struct InspectorContext {
    pub(crate) inspector_core: shrimply_inspector_core::InspectorController,
    pub(crate) project: Rc<RefCell<Project>>,
    pub(crate) player_state: SharedPlayerState,
    pub(crate) selected_item: Option<InspectedItem>,
    pub(crate) volume: Rc<RefCell<shrimply_audio_engine::streaming::FrameAudioSampler>>,
    pub(crate) preferences: SharedPreferences,
    pub(crate) preview_focus: SharedPreviewFocus,
    pub(crate) preview_item: Option<ItemAddress>,
    pub(crate) property_clipboard: shrimply_property_transfer::SharedClipboard,
    keyframe_graph_views: KeyframeGraphViewStates,
    // Per-row inspector listeners must use this scope instead of GTK weak refs.
    // Removed widgets can stay alive through signal/controller cycles after rebuild.
    pub(crate) listener_scope: Rc<()>,
    pub(crate) refresh: Rc<dyn Fn()>,
    list_target: InspectorTarget,
    list_state: Rc<RefCell<shrimply_inspector_core::list::InspectorListState>>,
    category_bar: gtk::Box,
}

impl InspectorContext {
    pub(crate) fn audio_analysis_at(
        &self,
        position: Time,
    ) -> shrimply_project_evaluation::FrameAudioAnalysis {
        self.inspector_core.audio_analysis_at(position)
    }

    fn detached(&self) -> Self {
        let mut context = self.clone();
        context.listener_scope = Rc::new(());
        context
    }

    pub(crate) fn keyframe_graph_state(
        &self,
        scope: impl Into<String>,
        initial: shrimply_inspector_core::keyframe_graph::FrameGraphState,
    ) -> shrimply_components_gtk::ui::SharedFrameGraphState {
        let key =
            crate::keyframe_graph::view_state_scope(self.selected_item.as_ref(), &scope.into());
        self.keyframe_graph_views
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                shrimply_components_gtk::ui::SharedFrameGraphState::new(
                    shrimply_inspector_core::keyframe_graph::FrameGraphComponents::single(initial),
                )
            })
            .clone()
    }
}

pub(crate) trait Inspectable {
    fn title(&self) -> &'static str;
    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext);

    fn default_action(&self, _context: &InspectorContext) -> Option<Box<dyn Fn() + 'static>> {
        None
    }

    fn controls(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let section = InspectorSection::controls();
        self.add_rows(&section, context);
        vec![section.into_widget()]
    }

    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let section = InspectorSection::new(self.title(), self.default_action(context));
        self.add_rows(&section, context);
        vec![section.into_widget()]
    }
}

pub fn new(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    preferences: SharedPreferences,
    fallback_focus: gtk::Widget,
    property_clipboard: shrimply_property_transfer::SharedClipboard,
) -> gtk::Widget {
    let default_font =
        shrimply_editor_state::preferences::snapshot(&preferences).default_text_font_family;
    let core = shrimply_inspector_core::InspectorController::new(
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
    )
    .with_default_text_font(default_font.clone())
    .with_property_clipboard(property_clipboard.clone());
    activate_project_google_fonts(&project.borrow(), &default_font, &core);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let category_bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    category_bar.set_visible(false);
    let volume = core.audio_sampler();
    let state = InspectorState {
        core,
        listener_scope: Rc::new(RefCell::new(Rc::new(()))),
        keyframe_graph_views: Rc::new(RefCell::new(HashMap::new())),
        fallback_focus,
        rebuild_pending: Rc::new(Cell::new(false)),
        list_state: Default::default(),
        volume,
        preferences,
        preview_focus,
        property_clipboard,
        active_inspector: Default::default(),
    };
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&content)
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(INSPECTOR_MIN_WIDTH)
        .build();
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layout.append(&category_bar);
    layout.append(&scrolled);

    let selection_content = content.clone();
    let selection_category_bar = category_bar.clone();
    let selection_scrolled = scrolled.clone();
    let selection_project = project.clone();
    let selection_player_state = player_state.clone();
    let selection_inspector_state = state.clone();
    selection_state::connect_named(&selection_state, "inspector selection rebuild", move || {
        queue_rebuild(
            &selection_content,
            &selection_category_bar,
            &selection_scrolled,
            &selection_project,
            &selection_player_state,
            &selection_inspector_state,
        );
    });

    let project_content = content.clone();
    let project_category_bar = category_bar.clone();
    let project_scrolled = scrolled.clone();
    let project_for_listener = project.clone();
    let project_player_state = player_state.clone();
    let project_inspector_state = state.clone();
    player_state::connect_named(&player_state, "inspector project rebuild", move |event| {
        let crate::player_state::PlayerEvent::Project(change) = event else {
            return;
        };
        if !change.inspector {
            return;
        }
        queue_rebuild(
            &project_content,
            &project_category_bar,
            &project_scrolled,
            &project_for_listener,
            &project_player_state,
            &project_inspector_state,
        );
    });

    let poll_layout = layout.downgrade();
    let poll_content = content.downgrade();
    let poll_category_bar = category_bar.downgrade();
    let poll_scrolled = scrolled.downgrade();
    let poll_project = project.clone();
    let poll_player_state = player_state.clone();
    let poll_state = state.clone();
    glib::timeout_add_local(DOCUMENT_POLL_INTERVAL, move || {
        let (Some(layout), Some(content), Some(category_bar), Some(scrolled)) = (
            poll_layout.upgrade(),
            poll_content.upgrade(),
            poll_category_bar.upgrade(),
            poll_scrolled.upgrade(),
        ) else {
            return glib::ControlFlow::Break;
        };
        if layout.is_mapped() && shrimply_inspector_document::poll(&poll_state.core) {
            queue_rebuild(
                &content,
                &category_bar,
                &scrolled,
                &poll_project,
                &poll_player_state,
                &poll_state,
            );
        }
        glib::ControlFlow::Continue
    });

    rebuild(
        &content,
        &category_bar,
        &scrolled,
        &project,
        &player_state,
        &state,
    );

    layout.upcast()
}

fn activate_project_google_fonts(
    project: &Project,
    default_font: &shrimply_project_document::project::FontFamily,
    controller: &shrimply_inspector_core::InspectorController,
) {
    let Some(activation) = font_cache::activate_project_google_fonts(project, default_font) else {
        return;
    };
    let controller = controller.clone();
    glib::spawn_future_local(async move {
        if activation.wait().await {
            controller.refresh_video();
        }
    });
}

fn queue_rebuild(
    content: &gtk::Box,
    category_bar: &gtk::Box,
    scrolled: &gtk::ScrolledWindow,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    state: &InspectorState,
) {
    if state.rebuild_pending.replace(true) {
        return;
    }
    // Retire the old tree's listeners before the current event continues dispatching.
    *state.listener_scope.borrow_mut() = Rc::new(());
    let content = content.clone();
    let category_bar = category_bar.clone();
    let scrolled = scrolled.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let state = state.clone();
    glib::idle_add_local_once(move || {
        if content.root().is_some() {
            rebuild(
                &content,
                &category_bar,
                &scrolled,
                &project,
                &player_state,
                &state,
            );
        }
        state.rebuild_pending.set(false);
    });
}

fn rebuild(
    content: &gtk::Box,
    category_bar: &gtk::Box,
    scrolled: &gtk::ScrolledWindow,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    state: &InspectorState,
) {
    let scoped_listeners = state.listener_scope.borrow().clone();
    if let Some(active) = state.active_inspector.borrow().as_ref() {
        let value = scrolled.vadjustment().value();
        state
            .list_state
            .borrow_mut()
            .set_scroll_position(active, value);
    }
    if content
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| GtkWindowExt::focus(&window))
        .is_some_and(|focus| focus.is_ancestor(content) || focus.is_ancestor(category_bar))
    {
        state.fallback_focus.grab_focus();
    }
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    while let Some(child) = category_bar.first_child() {
        category_bar.remove(&child);
    }
    category_bar.set_visible(false);

    state.core.retain_analysis_transitions();
    let target = state.core.target();
    let scroll_value = state.list_state.borrow().scroll_position(&target);
    *state.active_inspector.borrow_mut() = Some(target.clone());
    let (inspectable, selected_item, preview_item) =
        resolve_target(project, &target, &state.preview_focus);

    let context = InspectorContext {
        inspector_core: state.core.clone(),
        project: project.clone(),
        player_state: player_state.clone(),
        selected_item,
        volume: state.volume.clone(),
        preferences: state.preferences.clone(),
        preview_focus: state.preview_focus.clone(),
        preview_item,
        property_clipboard: state.property_clipboard.clone(),
        keyframe_graph_views: state.keyframe_graph_views.clone(),
        listener_scope: scoped_listeners,
        refresh: {
            let content = content.downgrade();
            let category_bar = category_bar.downgrade();
            let scrolled = scrolled.downgrade();
            let project = project.clone();
            let player_state = player_state.clone();
            let state = state.clone();
            Rc::new(move || {
                let (Some(content), Some(category_bar), Some(scrolled)) = (
                    content.upgrade(),
                    category_bar.upgrade(),
                    scrolled.upgrade(),
                ) else {
                    return;
                };
                queue_rebuild(
                    &content,
                    &category_bar,
                    &scrolled,
                    &project,
                    &player_state,
                    &state,
                )
            })
        },
        list_target: target,
        list_state: state.list_state.clone(),
        category_bar: category_bar.clone(),
    };
    for widget in inspectable.inspect(&context) {
        content.append(&widget);
    }
    let scrolled = scrolled.clone();
    glib::idle_add_local_once(move || {
        let adjustment = scrolled.vadjustment();
        let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value(scroll_value.clamp(adjustment.lower(), max));
    });
}

fn validate_preview_focus(
    preview_focus: &SharedPreviewFocus,
    address: &ItemAddress,
    item: &VideoItem,
) {
    let Some(focused) = preview_focus::snapshot(preview_focus) else {
        return;
    };
    if !shrimply_inspector_core::item::valid_preview_focus(
        &focused.item,
        focused.target,
        address,
        item,
    ) {
        preview_focus::clear(preview_focus);
    }
}

fn resolve_target(
    project: &Rc<RefCell<Project>>,
    target: &InspectorTarget,
    preview_focus: &SharedPreviewFocus,
) -> (
    Box<dyn Inspectable>,
    Option<ItemAddress>,
    Option<ItemAddress>,
) {
    let mut preview_video = None;
    let resolved = {
        let project = project.borrow();
        match target {
            InspectorTarget::Transition { item, side } => {
                shrimply_inspector_core::transition::presentation(&project, item, *side)
                    .map(|transition| {
                        (
                            Box::new(transition) as Box<dyn Inspectable>,
                            Some(item.clone()),
                            None,
                        )
                    })
                    .unwrap_or_else(|| project_target(&project))
            }
            InspectorTarget::Item(address) => match project.item(address) {
                Some(ItemRef::Comment(item)) => (
                    Box::new(item.clone()) as Box<dyn Inspectable>,
                    Some(address.clone()),
                    None,
                ),
                Some(ItemRef::Caption(item)) => (
                    Box::new(item.clone()) as Box<dyn Inspectable>,
                    Some(address.clone()),
                    None,
                ),
                Some(ItemRef::Video(item)) => {
                    preview_video = Some((address.clone(), item.clone()));
                    (
                        Box::new(item.clone()) as Box<dyn Inspectable>,
                        Some(address.clone()),
                        Some(address.clone()),
                    )
                }
                Some(ItemRef::Audio(item)) => (
                    Box::new(item.clone()) as Box<dyn Inspectable>,
                    Some(address.clone()),
                    None,
                ),
                None => project_target(&project),
            },
            InspectorTarget::Track(address) => {
                shrimply_inspector_core::track::presentation(&project, address.clone())
                    .map(|track| (Box::new(track) as Box<dyn Inspectable>, None, None))
                    .unwrap_or_else(|| project_target(&project))
            }
            InspectorTarget::Project => project_target(&project),
        }
    };
    if let Some((address, item)) = preview_video {
        validate_preview_focus(preview_focus, &address, &item);
    } else {
        preview_focus::clear(preview_focus);
    }
    resolved
}

fn project_target(
    project: &Project,
) -> (
    Box<dyn Inspectable>,
    Option<ItemAddress>,
    Option<ItemAddress>,
) {
    (Box::new(project.clone()), None, None)
}
