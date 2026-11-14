use std::rc::Rc;

use gtk::prelude::*;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::InspectorCard;
use shrimply_project_document::project::ItemAddress;

use super::InspectorContext;
use super::item::{HeaderAction, HeaderButtonToggle, HeaderToggle, InspectorListItem};
use crate::item::PreviewFocusTarget;
use crate::preview_focus::{self, FocusedPreview};

pub(super) struct InspectorCategory {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) icon: &'static str,
    pub(super) items: Vec<InspectorListItem>,
}

pub(super) fn render_document(
    document: shrimply_inspector_document::InspectorDocument,
    context: &InspectorContext,
    controls: super::item::RenderControls,
) -> Vec<gtk::Widget> {
    assert_eq!(
        document.target, context.list_target,
        "inspector target changed during rendering"
    );
    let categories = document
        .categories
        .into_iter()
        .map(|category| InspectorCategory {
            key: category.key,
            label: category.label,
            icon: category.icon.symbolic_name(),
            items: category
                .items
                .into_iter()
                .map(|item| match item {
                    shrimply_inspector_document::InspectorListItem::Item(item) => {
                        InspectorListItem::Item(Box::new(super::item::DocumentItem::new(
                            *item, context, controls,
                        )))
                    }
                    shrimply_inspector_document::InspectorListItem::Flat(section) => {
                        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
                        for widget in controls(&section, context) {
                            content.append(&widget);
                        }
                        InspectorListItem::Flat(content.upcast())
                    }
                })
                .collect(),
        })
        .collect();
    render_categories(categories, context)
}

pub(super) fn render_categories(
    categories: Vec<InspectorCategory>,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let selector = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    selector.set_hexpand(true);
    selector.set_homogeneous(true);
    selector.set_margin_top(12);
    selector.set_margin_bottom(8);
    selector.set_margin_start(16);
    selector.set_margin_end(16);
    selector.add_css_class("linked");
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_vhomogeneous(false);

    let active = {
        let state = context.list_state.borrow();
        let remembered = state.active_category(&context.list_target);
        categories
            .iter()
            .position(|category| remembered == Some(category.key))
            .unwrap_or_default() as u32
    };

    let mut first_button = None;
    for (index, category) in categories.into_iter().enumerate() {
        let toggle_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toggle_content.set_halign(gtk::Align::Center);
        toggle_content.append(&gtk::Image::from_icon_name(category.icon));
        toggle_content.append(&gtk::Label::new(Some(tr!(category.label).as_ref())));
        stack.add_named(&render_list(category.items, context), Some(category.key));
        if index as u32 == active {
            stack.set_visible_child_name(category.key);
        }

        let mut button = gtk::ToggleButton::builder()
            .hexpand(true)
            .tooltip_text(tr!(category.label).as_ref())
            .child(&toggle_content);
        if let Some(first_button) = &first_button {
            button = button.group(first_button);
        }
        let button = button.active(index as u32 == active).build();
        let target = context.list_target.clone();
        let list_state = context.list_state.clone();
        let category_stack = stack.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            category_stack.set_visible_child_name(category.key);
            list_state
                .borrow_mut()
                .set_active_category(&target, category.key);
        });
        first_button.get_or_insert_with(|| button.clone());
        selector.append(&button);
    }

    context.category_bar.append(&selector);
    context.category_bar.set_visible(true);
    vec![stack.upcast()]
}

fn render_list(items: Vec<InspectorListItem>, context: &InspectorContext) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    list.set_margin_top(4);
    list.set_margin_bottom(16);
    list.set_margin_start(16);
    list.set_margin_end(16);

    for item in items {
        match item {
            InspectorListItem::Item(item) => {
                let expanded = context
                    .list_state
                    .borrow()
                    .expanded(&context.list_target, item.key());

                let list_state = context.list_state.clone();
                let target = context.list_target.clone();
                let key = item.key().to_string();
                let reset = item.reset(context);
                let card = InspectorCard::with_expansion(
                    tr!(item.title()).as_ref(),
                    expanded,
                    reset,
                    move |expanded| {
                        list_state
                            .borrow_mut()
                            .set_expanded(&target, &key, expanded);
                    },
                );
                add_preview_focus(card.root(), item.key(), item.preview_target(), context);

                if let Some(toggle) = item.toggle() {
                    card.append_before_reset(&toggle_widget(toggle));
                }
                if let Some(toggle) = item.button_toggle() {
                    card.append_before_reset(&button_toggle_widget(toggle));
                }
                for action in item.actions() {
                    card.append_after_reset(&action_widget(action));
                }
                for control in item.controls(context) {
                    card.append(&control);
                }
                list.append(card.widget());
            }
            InspectorListItem::Flat(widget) => list.append(&widget),
        }
    }

    list
}

fn add_preview_focus(
    row: &gtk::Box,
    card_key: &str,
    target: PreviewFocusTarget,
    context: &InspectorContext,
) {
    let Some(item) = context.preview_item.clone() else {
        return;
    };
    let Some(item_id) = context
        .project
        .borrow()
        .video_item(&item)
        .map(|item| item.id)
    else {
        return;
    };
    let target = target.resolve(item_id);
    let card_key = card_key.to_string();
    sync_preview_focus_class(
        row,
        preview_focus::snapshot(&context.preview_focus).as_ref(),
        &item,
        &card_key,
    );

    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let click_focus = context.preview_focus.clone();
    let click_card_key = card_key.clone();
    let click_item = item.clone();
    click.connect_pressed(move |_, _, _, _| {
        preview_focus::set(
            &click_focus,
            FocusedPreview {
                item: click_item.clone(),
                card_key: click_card_key.clone(),
                target,
            },
        );
    });
    row.add_controller(click);

    let focus = gtk::EventControllerFocus::new();
    let keyboard_focus = context.preview_focus.clone();
    let keyboard_card_key = card_key.clone();
    let keyboard_item = item.clone();
    focus.connect_enter(move |_| {
        preview_focus::set(
            &keyboard_focus,
            FocusedPreview {
                item: keyboard_item.clone(),
                card_key: keyboard_card_key.clone(),
                target,
            },
        );
    });
    row.add_controller(focus);

    let weak_row = row.downgrade();
    let listener_scope = Rc::downgrade(&context.listener_scope);
    let listener_focus = context.preview_focus.clone();
    preview_focus::connect_while_alive_named(
        &context.preview_focus,
        "inspector preview focus style",
        move || listener_scope.upgrade().is_some(),
        move || {
            let Some(row) = weak_row.upgrade() else {
                return;
            };
            sync_preview_focus_class(
                &row,
                preview_focus::snapshot(&listener_focus).as_ref(),
                &item,
                &card_key,
            );
        },
    );
}

fn sync_preview_focus_class(
    row: &gtk::Box,
    focused: Option<&FocusedPreview>,
    item: &ItemAddress,
    card_key: &str,
) {
    if focused.is_some_and(|focused| &focused.item == item && focused.card_key == card_key) {
        row.add_css_class("accent");
    } else {
        row.remove_css_class("accent");
    }
}

fn toggle_widget(toggle: &HeaderToggle) -> gtk::Switch {
    let switch = gtk::Switch::builder()
        .active(toggle.active)
        .tooltip_text(tr!(toggle.tooltip).as_ref())
        .valign(gtk::Align::Center)
        .build();
    let activate = toggle.activate.clone();
    switch.connect_active_notify(move |switch| activate(switch.is_active()));
    switch
}

fn button_toggle_widget(toggle: &HeaderButtonToggle) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .icon_name(toggle.icon)
        .active(toggle.active)
        .tooltip_text(tr!(toggle.tooltip).as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let activate = toggle.activate.clone();
    button.connect_toggled(move |button| activate(button.is_active()));
    button
}

fn action_widget(action: &HeaderAction) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(action.icon)
        .tooltip_text(tr!(action.tooltip).as_ref())
        .sensitive(action.sensitive)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let activate = action.activate.clone();
    button.connect_clicked(move |_| activate());
    button
}
