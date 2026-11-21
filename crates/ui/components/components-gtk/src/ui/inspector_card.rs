use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

thread_local! {
    static EXPANDER_CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

pub struct InspectorCard {
    root: gtk::Box,
    controls: gtk::Box,
    before_reset: gtk::Box,
    after_reset: gtk::Box,
}

impl InspectorCard {
    pub fn new(title: &str, expanded: bool, on_reset: impl Fn() + 'static) -> Self {
        Self::with_expansion(title, expanded, Some(Rc::new(on_reset)), |_| {})
    }

    pub fn with_expansion(
        title: &str,
        expanded: bool,
        on_reset: Option<Rc<dyn Fn()>>,
        on_expanded: impl Fn(bool) + 'static,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("card");

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.set_margin_top(6);
        header.set_margin_bottom(6);
        header.set_margin_start(8);
        header.set_margin_end(8);
        install_expander_css(&header.display());

        let expander_icon = gtk::Image::from_icon_name("pan-end-symbolic");
        expander_icon.add_css_class("inspector-expander-icon");
        if expanded {
            expander_icon.add_css_class("expanded");
        }
        let expander = gtk::Button::builder()
            .child(&expander_icon)
            .tooltip_text(if expanded { "Collapse" } else { "Expand" })
            .css_classes(["flat"])
            .build();
        header.append(&expander);
        header.append(
            &gtk::Label::builder()
                .label(title)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .css_classes(["heading"])
                .build(),
        );
        let before_reset = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.append(&before_reset);
        if let Some(on_reset) = on_reset {
            let reset = gtk::Button::builder()
                .icon_name("edit-undo-symbolic")
                .tooltip_text("Reset")
                .css_classes(["flat"])
                .build();
            reset.connect_clicked(move |_| on_reset());
            header.append(&reset);
        }
        let after_reset = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.append(&after_reset);

        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.set_margin_top(4);
        controls.set_margin_bottom(12);
        controls.set_margin_start(12);
        controls.set_margin_end(12);
        let revealer = gtk::Revealer::builder()
            .child(&controls)
            .reveal_child(expanded)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(180)
            .build();
        expander.connect_clicked({
            let expander_icon = expander_icon.clone();
            let revealer = revealer.clone();
            move |button| {
                let expanded = !revealer.reveals_child();
                revealer.set_reveal_child(expanded);
                if expanded {
                    expander_icon.add_css_class("expanded");
                } else {
                    expander_icon.remove_css_class("expanded");
                }
                button.set_tooltip_text(Some(if expanded { "Collapse" } else { "Expand" }));
                on_expanded(expanded);
            }
        });
        root.append(&header);
        root.append(&revealer);
        Self {
            root,
            controls,
            before_reset,
            after_reset,
        }
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        self.controls.append(child);
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.root.upcast_ref()
    }

    pub fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub fn append_before_reset(&self, child: &impl IsA<gtk::Widget>) {
        self.before_reset.append(child);
    }

    pub fn append_after_reset(&self, child: &impl IsA<gtk::Widget>) {
        self.after_reset.append(child);
    }
}

fn install_expander_css(display: &gtk::gdk::Display) {
    EXPANDER_CSS_INSTALLED.with(|installed| {
        if installed.replace(true) {
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(
            ".inspector-expander-icon { \
                 -gtk-icon-transform: rotate(0deg); \
                 transition: 180ms ease; \
             } \
             .inspector-expander-icon.expanded { \
                 -gtk-icon-transform: rotate(90deg); \
             }",
        );
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}
