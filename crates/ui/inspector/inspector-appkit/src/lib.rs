#![cfg(target_os = "macos")]

mod control;
mod files;
mod focus;
mod fonts;
mod info;
mod layered;
#[cfg(debug_assertions)]
mod layout_debug;
mod tts;

use objc2::ClassType;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{MainThreadMarker, NSEdgeInsets};
use shrimply_components_appkit::{
    InspectorCard, ScrollingColumn, Switch, Tab, Tabs, ViewHost, column_append_intrinsic,
    column_stack, inset,
};
use shrimply_editor_state::player_state;
use shrimply_inspector_core::{InspectorController, InspectorTarget};
use shrimply_inspector_document::{InspectorDocument, InspectorItem, InspectorListItem};
use shrimply_project_document::project::Project;
use shrimply_timeline_edit::selection_state;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CONTENT_SPACING: f64 = 10.0;
const CONTENT_INSET: f64 = 12.0;

pub struct Inspector {
    _focus_monitor: focus::FocusMonitor,
    state: Rc<State>,
}

struct State {
    #[cfg(debug_assertions)]
    layout_diagnostic_pending: Cell<bool>,
    server_url: Rc<RefCell<String>>,
    preferences: shrimply_editor_state::preferences::SharedPreferences,
    controller: InspectorController,
    host: ViewHost,
    dirty: Rc<Cell<bool>>,
    force_rebuild: Rc<Cell<bool>>,
    polls: control::Polls,
    list: Rc<RefCell<shrimply_inspector_core::list::InspectorListState>>,
    focus: Rc<focus::FocusMap>,
    visible_document: RefCell<Option<InspectorDocument>>,
    visible_scroll: Rc<RefCell<Option<ScrollingColumn>>>,
}

impl Inspector {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player: player_state::SharedPlayerState,
        selection: selection_state::SharedSelectionState,
        clipboard: shrimply_property_transfer::SharedClipboard,
        preview_focus: shrimply_editor_state::preview_focus::SharedPreviewFocus,
        preferences: shrimply_editor_state::preferences::SharedPreferences,
        mtm: MainThreadMarker,
    ) -> Self {
        let dirty = Rc::new(Cell::new(true));
        selection_state::connect_named(&selection, "AppKit inspector selection", {
            let dirty = dirty.clone();
            move || dirty.set(true)
        });
        player_state::connect_named(&player, "AppKit inspector project", {
            let dirty = dirty.clone();
            move |event| {
                if matches!(
                    event,
                    player_state::PlayerEvent::Project(player_state::ProjectChange {
                        inspector: true,
                        ..
                    })
                ) || matches!(
                    event,
                    player_state::PlayerEvent::State(player_state::StateChange {
                        position: Some(player_state::PositionChange::Seek),
                        ..
                    })
                ) {
                    dirty.set(true);
                }
            }
        });
        let server_url =
            shrimply_editor_state::preferences::snapshot(&preferences).compute_server_url;
        let controller = InspectorController::new(project, player, selection)
            .with_property_clipboard(clipboard)
            .with_analysis_backend(
                shrimply_inspector_core::InspectorAnalysisBackend::default()
                    .camera(shrimply_preview_render_metal::camera_reconstruction::analyze)
                    .transparent_fill(
                        shrimply_preview_render_metal::transparent_fill_analysis::analyze,
                    )
                    .visual_cache(shrimply_preview_render_metal::modifier_cache::bake),
            );
        let state = Rc::new(State {
            #[cfg(debug_assertions)]
            layout_diagnostic_pending: Cell::new(
                std::env::var_os("SHRIMPLY_INSPECTOR_LAYOUT_DEBUG").is_some(),
            ),
            server_url: Rc::new(RefCell::new(server_url)),
            preferences: preferences.clone(),
            controller: controller.clone(),
            focus: focus::FocusMap::new(controller, preview_focus),
            host: ViewHost::new(mtm),
            dirty,
            force_rebuild: Rc::new(Cell::new(false)),
            polls: Rc::new(RefCell::new(Vec::new())),
            list: Rc::new(RefCell::new(Default::default())),
            visible_document: RefCell::new(None),
            visible_scroll: Rc::new(RefCell::new(None)),
        });
        shrimply_editor_state::preferences::connect(&preferences, {
            let state = Rc::downgrade(&state);
            move |preferences| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if *state.server_url.borrow() != preferences.compute_server_url {
                    state.server_url.replace(preferences.compute_server_url);
                }
                state.dirty.set(true);
            }
        });
        state.dirty.set(false);
        state.rebuild(mtm);
        let monitor = focus::FocusMonitor::new(state.host.view(), &state.focus, mtm);
        Self {
            _focus_monitor: monitor,
            state,
        }
    }

    pub fn view(&self) -> &NSView {
        self.state.host.view()
    }

    pub fn poll(&self, mtm: MainThreadMarker) -> Vec<String> {
        #[cfg(debug_assertions)]
        if self.state.layout_diagnostic_pending.get()
            && self.view().window().is_some()
            && self.view().frame().size.width > 0.0
            && self.view().frame().size.height > 0.0
        {
            self.state.layout_diagnostic_pending.set(false);
            self.view().layoutSubtreeIfNeeded();
            layout_debug::dump(self.view(), 0);
            layout_debug::expanded_cards(mtm);
        }
        if shrimply_inspector_document::poll(&self.state.controller) {
            self.state.dirty.set(true);
        }
        let errors = shrimply_inspector_core::manim_parameters::take_scene_errors();
        for poll in self.state.polls.borrow().iter() {
            poll();
        }
        // Keep the active control alive throughout AppKit's mouse-tracking loop.
        // Async results may mark the document dirty while preview rendering continues.
        if self.state.dirty.get()
            && unsafe {
                objc2_foundation::NSRunLoop::currentRunLoop()
                    .currentMode()
                    .as_deref()
                    == Some(objc2_app_kit::NSEventTrackingRunLoopMode)
            }
        {
            return errors;
        }
        if self.state.dirty.replace(false) {
            self.state.rebuild(mtm);
        }
        errors
    }
}

impl State {
    fn rebuild(self: &Rc<Self>, mtm: MainThreadMarker) {
        let document = shrimply_inspector_document::document(
            &self.controller,
            info::format_date,
            &self.server_url.borrow(),
            &shrimply_editor_state::preferences::snapshot(&self.preferences).last_tts_model,
        );
        if !self.force_rebuild.replace(false)
            && self.visible_document.borrow().as_ref() == Some(&document)
        {
            return;
        }
        if let (Some(previous), Some(scroll)) = (
            self.visible_document.borrow().as_ref(),
            self.visible_scroll.borrow().as_ref(),
        ) {
            self.list
                .borrow_mut()
                .set_scroll_position(&previous.target, scroll.position());
        }
        self.polls.borrow_mut().clear();
        self.focus.clear(&document.target);
        let scroll_position = self.list.borrow().scroll_position(&document.target);
        let view = self.document_view(&document, mtm);
        self.visible_document.replace(Some(document));
        self.host.set_content(view);
        self.host.view().layoutSubtreeIfNeeded();
        if let Some(scroll) = self.visible_scroll.borrow().as_ref() {
            scroll.set_position(scroll_position);
        }
    }

    fn document_view(
        self: &Rc<Self>,
        document: &InspectorDocument,
        mtm: MainThreadMarker,
    ) -> Retained<NSView> {
        let selected = self
            .list
            .borrow()
            .active_category(&document.target)
            .and_then(|key| {
                document
                    .categories
                    .iter()
                    .position(|category| category.key == key)
            })
            .unwrap_or_default();
        let context = control::Context {
            focus: self.focus.clone(),
            preferences: self.preferences.clone(),
            server_url: self.server_url.clone(),
            controller: self.controller.clone(),
            target: document.target.clone(),
            dirty: self.dirty.clone(),
            force_rebuild: self.force_rebuild.clone(),
            polls: self.polls.clone(),
        };
        let mut tabs = Vec::with_capacity(document.categories.len());
        let mut scrolls = Vec::with_capacity(document.categories.len());
        for category in &document.categories {
            let column = column_stack(CONTENT_SPACING, mtm);
            column.setHuggingPriority_forOrientation(
                objc2_app_kit::NSLayoutPriorityRequired,
                objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
            );
            column.setClippingResistancePriority_forOrientation(
                objc2_app_kit::NSLayoutPriorityRequired,
                objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
            );
            for item in &category.items {
                match item {
                    InspectorListItem::Item(item) => column_append_intrinsic(
                        &column,
                        &self.card(&document.target, item, &context, mtm),
                    ),
                    InspectorListItem::Flat(section) => {
                        control::append_section(&column, section, &context, mtm);
                    }
                }
            }
            let padded = inset(
                &column,
                NSEdgeInsets {
                    top: CONTENT_INSET,
                    left: CONTENT_INSET,
                    bottom: CONTENT_INSET,
                    right: CONTENT_INSET,
                },
                mtm,
            );
            let scroll = ScrollingColumn::new(&padded, mtm);
            tabs.push(
                Tab::new(category.label, scroll.view().as_super().into())
                    .symbol(category_symbol(category.icon)),
            );
            scrolls.push(scroll);
        }
        let scrolls = Rc::new(scrolls);
        self.visible_scroll.replace(Some(scrolls[selected].clone()));
        let target = document.target.clone();
        let list = self.list.clone();
        let visible_scroll = self.visible_scroll.clone();
        let selected_scrolls = scrolls.clone();
        let category_keys = Rc::new(
            document
                .categories
                .iter()
                .map(|category| category.key)
                .collect::<Vec<_>>(),
        );
        let tabs = Tabs::with_selection(
            tabs,
            selected,
            move |index| {
                list.borrow_mut()
                    .set_active_category(&target, category_keys[index]);
                visible_scroll.replace(Some(selected_scrolls[index].clone()));
            },
            mtm,
        );
        tabs.view().into()
    }

    fn card(
        &self,
        target: &InspectorTarget,
        item: &InspectorItem,
        context: &control::Context,
        mtm: MainThreadMarker,
    ) -> Retained<NSView> {
        let expanded = self.list.borrow().expanded(target, &item.presentation.key);
        let card = if let Some(action) = item.reset.clone() {
            let context = context.clone();
            InspectorCard::new(
                &item.presentation.title,
                expanded,
                move || context.apply(&action),
                mtm,
            )
        } else {
            InspectorCard::without_reset(&item.presentation.title, expanded, mtm)
        };
        self.focus.register(
            card.view().as_super(),
            target,
            shrimply_inspector_core::item::ControlPreviewFocus::new(
                &item.presentation.key,
                item.presentation.preview_target,
            ),
        );
        if let Some(toggle) = &item.toggle {
            let context = context.clone();
            let action = toggle.activate.clone();
            let toggle = Switch::new(
                toggle.active,
                Some(toggle.tooltip),
                move |_| context.apply(&action),
                mtm,
            );
            card.append_before_reset(toggle.view().as_super().as_super());
        }
        if let Some(toggle) = &item.button_toggle {
            let context = context.clone();
            let action = toggle.activate.clone();
            let button = shrimply_components_appkit::ActionButton::symbol_toggle(
                action_symbol(toggle.icon),
                toggle.tooltip,
                toggle.active,
                move |requested| {
                    let result = context
                        .controller
                        .apply_basic_action(&context.target, &action.action(requested));
                    let accepted = if result.is_ok() {
                        requested
                    } else {
                        !requested
                    };
                    if result.is_ok() {
                        context.focus.toggle(&context.target, &action, requested);
                    }
                    context.refresh(result);
                    accepted
                },
                mtm,
            );
            card.append_before_reset(button.view());
        }
        for action in &item.actions {
            let context = context.clone();
            let activate = action.activate.clone();
            let button = shrimply_components_appkit::ActionButton::symbol(
                action_symbol(action.icon),
                action.tooltip,
                move || context.apply(&activate),
                mtm,
            );
            button.view().setEnabled(action.sensitive);
            card.append_after_reset(button.view());
        }
        for control in item
            .section
            .controls
            .iter()
            .filter(|control| control.visible)
        {
            card.append(&control::view(control, context, mtm));
        }
        let target = target.clone();
        let key = item.presentation.key.clone();
        let list = self.list.clone();
        card.connect_expansion(move |expanded, _| {
            list.borrow_mut().set_expanded(&target, &key, expanded);
        });
        card.view().as_super().into()
    }
}

fn category_symbol(icon: shrimply_inspector_document::CategoryIcon) -> &'static str {
    use shrimply_inspector_document::CategoryIcon;
    match icon {
        CategoryIcon::Project | CategoryIcon::Track => "slider.horizontal.3",
        CategoryIcon::Text => "textformat",
        CategoryIcon::Visual => "circle.lefthalf.filled",
        CategoryIcon::Audio => "speaker.wave.2",
        CategoryIcon::Playback => "play",
        CategoryIcon::Info => "info.circle",
        CategoryIcon::Performance => "speedometer",
        CategoryIcon::Transition => "arrow.left.and.right",
    }
}

fn action_symbol(icon: &str) -> &'static str {
    match icon {
        "edit-copy-symbolic" => "doc.on.doc",
        "go-up-symbolic" => "chevron.up",
        "go-down-symbolic" => "chevron.down",
        "user-trash-symbolic" => "trash",
        "select-symbolic" => "square.dashed",
        "view-refresh-symbolic" => "arrow.clockwise",
        _ => panic!("unsupported inspector header icon: {icon}"),
    }
}
