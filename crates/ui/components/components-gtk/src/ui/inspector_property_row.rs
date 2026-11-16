use gtk::prelude::*;
use shrimply_components_core::layered::{EXPRESSION_ICON, KEYFRAMES_ICON, REVEAL_DURATION_MS};

pub struct InspectorPropertyRow {
    root: gtk::Box,
    keyframes: gtk::ToggleButton,
    expression: gtk::ToggleButton,
    keyframe_revealer: gtk::Revealer,
    expression_revealer: gtk::Revealer,
}

impl InspectorPropertyRow {
    pub fn new(label: &str, editor: &impl IsA<gtk::Widget>) -> Self {
        Self::build(label, editor, false)
    }

    pub fn new_wide(label: &str, editor: &impl IsA<gtk::Widget>) -> Self {
        Self::build(label, editor, true)
    }

    fn build(label: &str, editor: &impl IsA<gtk::Widget>, wide: bool) -> Self {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let keyframes = gtk::ToggleButton::builder()
            .icon_name(KEYFRAMES_ICON)
            .tooltip_text(crate::tr!("Keyframes").as_ref())
            .css_classes(["flat"])
            .build();
        let expression = gtk::ToggleButton::builder()
            .icon_name(EXPRESSION_ICON)
            .tooltip_text(crate::tr!("Expression").as_ref())
            .css_classes(["flat"])
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        if wide {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            content.append(&spacer);
            content.append(&keyframes);
            content.append(&expression);
            root.append(&super::control_row(label, &content));
            editor.set_hexpand(true);
            root.append(editor);
        } else {
            editor.set_hexpand(true);
            content.append(editor);
            content.append(&keyframes);
            content.append(&expression);
            root.append(&super::control_row(label, &content));
        }
        let keyframe_revealer = gtk::Revealer::builder()
            .reveal_child(false)
            .visible(false)
            .margin_top(6)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(REVEAL_DURATION_MS)
            .build();
        let expression_revealer = gtk::Revealer::builder()
            .reveal_child(false)
            .visible(false)
            .margin_top(6)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(REVEAL_DURATION_MS)
            .build();
        hide_after_collapse(&keyframe_revealer);
        hide_after_collapse(&expression_revealer);
        keyframes.connect_toggled({
            let revealer = keyframe_revealer.clone();
            move |button| {
                if button.is_active() && revealer.child().is_some() {
                    revealer.set_visible(true);
                }
                revealer.set_reveal_child(button.is_active());
            }
        });
        expression.connect_toggled({
            let revealer = expression_revealer.clone();
            move |button| {
                if button.is_active() && revealer.child().is_some() {
                    revealer.set_visible(true);
                }
                revealer.set_reveal_child(button.is_active());
            }
        });
        root.append(&keyframe_revealer);
        root.append(&expression_revealer);
        Self {
            root,
            keyframes,
            expression,
            keyframe_revealer,
            expression_revealer,
        }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.root.upcast_ref()
    }

    pub fn set_keyframe_section(&self, child: &impl IsA<gtk::Widget>) {
        self.keyframe_revealer.set_child(Some(child));
    }

    pub fn set_expression_section(&self, child: &impl IsA<gtk::Widget>) {
        self.expression_revealer.set_child(Some(child));
    }

    pub fn append_body(&self, child: &impl IsA<gtk::Widget>) {
        self.root.append(child);
    }

    pub fn set_keyframes_active(&self, active: bool) {
        self.keyframes.set_active(active);
    }

    pub fn set_expression_active(&self, active: bool) {
        self.expression.set_active(active);
    }

    pub fn connect_keyframes_changed(&self, changed: impl Fn(bool) + 'static) {
        self.keyframes
            .connect_toggled(move |button| changed(button.is_active()));
    }

    pub fn connect_expression_changed(&self, changed: impl Fn(bool) + 'static) {
        self.expression
            .connect_toggled(move |button| changed(button.is_active()));
    }
}

fn hide_after_collapse(revealer: &gtk::Revealer) {
    revealer.connect_child_revealed_notify(|revealer| {
        if !revealer.reveals_child() && !revealer.is_child_revealed() {
            revealer.set_visible(false);
        }
    });
}
