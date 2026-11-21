use adw::prelude::*;

pub fn read_only_field(value: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(value)
        .selectable(true)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .wrap(true)
        .build()
}

pub struct ReadOnlyField;

impl ReadOnlyField {
    pub fn builder(value: impl Into<String>) -> ReadOnlyFieldBuilder {
        ReadOnlyFieldBuilder {
            value: value.into(),
            right_aligned: false,
            action: None,
        }
    }
}

type Action = (String, String, Box<dyn Fn(&gtk::Button)>);

pub struct ReadOnlyFieldBuilder {
    value: String,
    right_aligned: bool,
    action: Option<Action>,
}

impl ReadOnlyFieldBuilder {
    pub fn right_aligned(mut self) -> Self {
        self.right_aligned = true;
        self
    }

    pub fn action(
        mut self,
        icon_name: impl Into<String>,
        tooltip: impl Into<String>,
        callback: impl Fn(&gtk::Button) + 'static,
    ) -> Self {
        self.action = Some((icon_name.into(), tooltip.into(), Box::new(callback)));
        self
    }

    pub fn build(self) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let value = read_only_field(&self.value);
        value.set_hexpand(true);
        if self.right_aligned {
            value.set_xalign(1.0);
        }
        row.append(&value);
        if let Some((icon_name, tooltip, callback)) = self.action {
            let button = gtk::Button::builder()
                .icon_name(icon_name)
                .tooltip_text(tooltip)
                .css_classes(["flat"])
                .build();
            button.connect_clicked(callback);
            row.append(&button);
        }
        row.upcast()
    }
}

const INLINE_VALUE_MAX_CHARS: usize = 24;

pub fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(crate::i18n::text(title).as_ref())
        .use_markup(false)
        .build();
    if value.chars().count() <= INLINE_VALUE_MAX_CHARS && !value.contains('\n') {
        let value = gtk::Label::builder()
            .label(value)
            .selectable(true)
            .xalign(1.0)
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&value);
    } else {
        row.set_subtitle(value);
        row.set_subtitle_selectable(true);
        row.set_subtitle_lines(0);
    }
    row
}
