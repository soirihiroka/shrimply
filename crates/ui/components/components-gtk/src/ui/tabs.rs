use adw::prelude::*;

pub fn stack_tabs(
    stack: &gtk::Stack,
    toggles: impl IntoIterator<Item = adw::Toggle>,
) -> adw::ToggleGroup {
    let selector = adw::ToggleGroup::builder()
        .hexpand(true)
        .homogeneous(true)
        .can_shrink(true)
        .margin_top(12)
        .margin_bottom(8)
        .margin_start(16)
        .margin_end(16)
        .build();
    for toggle in toggles {
        selector.add(toggle);
    }
    stack
        .bind_property("visible-child-name", &selector, "active-name")
        .bidirectional()
        .sync_create()
        .build();
    selector
}

pub fn tabs(pages: impl IntoIterator<Item = (String, String, gtk::Widget)>) -> gtk::Widget {
    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .vhomogeneous(false)
        .build();
    let toggles = pages
        .into_iter()
        .enumerate()
        .map(|(index, (title, icon, child))| {
            let name = format!("tab-{index}");
            stack.add_named(&child, Some(&name));
            adw::Toggle::builder()
                .name(name)
                .label(&title)
                .tooltip(title)
                .icon_name(icon)
                .build()
        })
        .collect::<Vec<_>>();
    let selector = stack_tabs(&stack, toggles);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&selector);
    content.append(&stack);
    content.upcast()
}
