use std::rc::Rc;

use gtk::prelude::*;
pub(super) use shrimply_inspector_core::item::PreviewFocusTarget;
use shrimply_inspector_core::item::{
    HeaderAction as SharedHeaderAction, HeaderButtonToggle as SharedHeaderButtonToggle,
    HeaderToggle as SharedHeaderToggle,
};

use super::InspectorContext;

pub(super) type HeaderAction = SharedHeaderAction<Rc<dyn Fn()>>;
pub(super) type HeaderToggle = SharedHeaderToggle<Rc<dyn Fn(bool)>>;
pub(super) type HeaderButtonToggle = SharedHeaderButtonToggle<Rc<dyn Fn(bool)>>;

pub(super) trait InspectorItem {
    fn key(&self) -> &str;
    fn title(&self) -> &str;
    fn controls(&self, context: &InspectorContext) -> Vec<gtk::Widget>;
    fn reset(&self, context: &InspectorContext) -> Option<Rc<dyn Fn()>>;
    fn actions(&self) -> &[HeaderAction];
    fn toggle(&self) -> Option<&HeaderToggle>;
    fn button_toggle(&self) -> Option<&HeaderButtonToggle>;
    fn preview_target(&self) -> PreviewFocusTarget;
}

pub(super) enum InspectorListItem {
    Item(Box<dyn InspectorItem>),
    Flat(gtk::Widget),
}

pub(super) type RenderControls =
    fn(&shrimply_inspector_core::InspectorSection, &InspectorContext) -> Vec<gtk::Widget>;

pub(super) struct DocumentItem {
    model: shrimply_inspector_document::InspectorItem,
    render: RenderControls,
    reset: Option<Rc<dyn Fn()>>,
    actions: Vec<HeaderAction>,
    toggle: Option<HeaderToggle>,
    button_toggle: Option<HeaderButtonToggle>,
}

impl DocumentItem {
    pub fn new(
        model: shrimply_inspector_document::InspectorItem,
        context: &InspectorContext,
        render: RenderControls,
    ) -> Self {
        let action = |action: shrimply_inspector_core::BasicInspectorAction| -> Rc<dyn Fn()> {
            let controller = context.inspector_core.clone();
            let target = context.list_target.clone();
            let anchor = context.category_bar.downgrade();
            Rc::new(move || {
                let message = match controller.apply_basic_action(&target, &action) {
                    Ok(message) => message,
                    Err(error) => Some(error),
                };
                if let Some(message) = message
                    && let Some(anchor) = anchor.upgrade()
                {
                    shrimply_components_gtk::toast::show_confirmation_text_for_widget(
                        &anchor, &message,
                    );
                }
            })
        };
        let reset = model.reset.clone().map(action);
        let actions = model
            .actions
            .iter()
            .map(|header| HeaderAction {
                icon: header.icon,
                tooltip: header.tooltip,
                sensitive: header.sensitive,
                activate: action(header.activate.clone()),
            })
            .collect();
        let toggle = model.toggle.as_ref().map(|header| {
            let activate = action(header.activate.clone());
            HeaderToggle {
                active: header.active,
                tooltip: header.tooltip,
                activate: Rc::new(move |_| activate()),
            }
        });
        let button_toggle = model.button_toggle.as_ref().map(|header| {
            let controller = context.inspector_core.clone();
            let target = context.list_target.clone();
            let anchor = context.category_bar.downgrade();
            let action = header.activate.clone();
            let preview_focus = context.preview_focus.clone();
            HeaderButtonToggle {
                icon: header.icon,
                active: header.active,
                tooltip: header.tooltip,
                activate: Rc::new(move |active| {
                    if let Err(error) =
                        controller.apply_basic_action(&target, &action.action(active))
                    {
                        if let Some(anchor) = anchor.upgrade() {
                            shrimply_components_gtk::toast::show_confirmation_text_for_widget(
                                &anchor, &error,
                            );
                        }
                        return;
                    }
                    if let Some(focus) = controller.toggle_preview_focus(&target, &action, active) {
                        crate::preview_focus::set(&preview_focus, focus);
                    }
                }),
            }
        });
        Self {
            model,
            render,
            reset,
            actions,
            toggle,
            button_toggle,
        }
    }
}

impl InspectorItem for DocumentItem {
    fn key(&self) -> &str {
        &self.model.presentation.key
    }
    fn title(&self) -> &str {
        &self.model.presentation.title
    }
    fn controls(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        (self.render)(&self.model.section, context)
    }
    fn reset(&self, _context: &InspectorContext) -> Option<Rc<dyn Fn()>> {
        self.reset.clone()
    }
    fn actions(&self) -> &[HeaderAction] {
        &self.actions
    }
    fn toggle(&self) -> Option<&HeaderToggle> {
        self.toggle.as_ref()
    }
    fn button_toggle(&self) -> Option<&HeaderButtonToggle> {
        self.button_toggle.as_ref()
    }
    fn preview_target(&self) -> PreviewFocusTarget {
        self.model.presentation.preview_target
    }
}
