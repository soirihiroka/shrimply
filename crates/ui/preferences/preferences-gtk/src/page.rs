use crate::{server, store};
use adw::prelude::*;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::ColorPicker;
use store::{PreferenceId, PreferencePage, PreferenceValue, SharedPreferences};

pub fn show_settings(
    window: &adw::ApplicationWindow,
    preferences: SharedPreferences,
) -> adw::Window {
    let settings = adw::Window::builder()
        .title(tr!(store::SETTINGS_TITLE).as_ref())
        .default_width(940)
        .default_height(630)
        .transient_for(window)
        .destroy_with_parent(true)
        .modal(true)
        .build();
    let categories = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let pages = gtk::Stack::new();
    let content_header = adw::HeaderBar::new();
    for id in PreferencePage::ALL {
        let row = adw::ActionRow::builder()
            .title(tr!(id.title()).as_ref())
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(id.icon()));
        categories.append(&row);
        let page = match id {
            PreferencePage::Trust => crate::trust::page(),
            PreferencePage::External => {
                let blender = adw::PreferencesGroup::builder()
                    .title(tr!("Blender").as_ref())
                    .build();
                for option in id.options("Blender") {
                    blender.add(&preference_row(option, &preferences));
                }
                server::page(preferences.clone(), &blender)
            }
            _ => {
                let page = adw::PreferencesPage::new();
                for group in id.groups() {
                    let section = adw::PreferencesGroup::builder()
                        .title(tr!(*group).as_ref())
                        .build();
                    #[cfg(target_os = "linux")]
                    if id == PreferencePage::Performance {
                        section.add(&crate::acceleration::row(preferences.clone()));
                    }
                    for option in id.options(group) {
                        section.add(&preference_row(option, &preferences));
                    }
                    page.add(&section);
                }
                page
            }
        };
        pages.add_named(&page, Some(id.id()));
    }

    let sidebar = adw::ToolbarView::new();
    sidebar.add_top_bar(&adw::HeaderBar::new());
    sidebar.set_content(Some(&categories));
    let sidebar = adw::NavigationPage::builder()
        .title(tr!(store::SETTINGS_TITLE).as_ref())
        .child(&sidebar)
        .build();
    let content = adw::ToolbarView::new();
    content.add_top_bar(&content_header);
    content.set_content(Some(&pages));
    let page = adw::NavigationPage::builder()
        .title(tr!(PreferencePage::Appearance.title()).as_ref())
        .child(&content)
        .build();
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&page)
        .sidebar_width_unit(adw::LengthUnit::Px)
        .min_sidebar_width(f64::from(store::SETTINGS_SIDEBAR_WIDTH))
        .max_sidebar_width(f64::from(store::SETTINGS_SIDEBAR_WIDTH))
        .build();
    categories.select_row(categories.row_at_index(0).as_ref());
    categories.connect_row_activated({
        let split = split.downgrade();
        move |_, row| {
            let id = PreferencePage::ALL[row.index() as usize];
            pages.set_visible_child_name(id.id());
            page.set_title(tr!(id.title()).as_ref());
            if let Some(split) = split.upgrade() {
                split.set_show_content(true);
            }
        }
    });
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        f64::from(store::SETTINGS_WIDE_WIDTH),
        adw::LengthUnit::Px,
    ));
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    settings.add_breakpoint(narrow);
    settings.set_content(Some(&split));
    settings.present();
    settings
}

fn preference_row(id: PreferenceId, preferences: &SharedPreferences) -> gtk::Widget {
    let presentation = id.presentation();
    let title = tr!(presentation.label);
    let subtitle = tr!(presentation.subtitle);
    let current = store::value(preferences, id);
    let set = {
        let preferences = preferences.clone();
        move |value| store::set_value(&preferences, id, value).expect("settings control value")
    };
    match current {
        PreferenceValue::Boolean(active) => {
            let row = adw::SwitchRow::builder()
                .title(title.as_ref())
                .subtitle(subtitle.as_ref())
                .active(active)
                .build();
            row.connect_active_notify(move |row| set(PreferenceValue::Boolean(row.is_active())));
            row.upcast()
        }
        PreferenceValue::Integer(value) if !presentation.choices.is_empty() => {
            let labels = presentation
                .choices
                .iter()
                .map(|label| tr!(*label))
                .collect::<Vec<_>>();
            let labels = labels.iter().map(AsRef::as_ref).collect::<Vec<_>>();
            let row = adw::ComboRow::builder()
                .title(title.as_ref())
                .subtitle(subtitle.as_ref())
                .model(&gtk::StringList::new(&labels))
                .selected(value as u32)
                .build();
            shrimply_components_gtk::ui::adaptive_choice(&row);
            row.connect_selected_notify(move |row| {
                set(PreferenceValue::Integer(i64::from(row.selected())))
            });
            row.upcast()
        }
        PreferenceValue::Integer(value) => {
            let range = store::integer_range(id).expect("numeric settings range");
            let scale = range.scale as f64;
            let row = adw::SpinRow::with_range(
                range.minimum as f64 / scale,
                range.maximum as f64 / scale,
                range.step as f64 / scale,
            );
            row.set_title(title.as_ref());
            row.set_subtitle(subtitle.as_ref());
            row.set_digits(match range.scale {
                1 => 0,
                10 => 1,
                _ => 2,
            });
            row.set_value(value as f64 / scale);
            row.connect_value_notify(move |row| {
                set(PreferenceValue::Integer(
                    (row.value() * scale).round() as i64
                ))
            });
            row.upcast()
        }
        PreferenceValue::Color(color) => {
            let row = adw::ActionRow::builder().title(title.as_ref()).build();
            let picker = ColorPicker::builder(color)
                .title(title.as_ref())
                .on_change(move |color| set(PreferenceValue::Color(color)))
                .build();
            picker.set_valign(gtk::Align::Center);
            row.add_suffix(&picker);
            row.upcast()
        }
        PreferenceValue::FontFamily(family) => {
            let row = adw::ActionRow::builder().title(title.as_ref()).build();
            let picker = shrimply_inspector_gtk::font_family_selector(&family, move |family| {
                set(PreferenceValue::FontFamily(family))
            });
            picker.set_valign(gtk::Align::Center);
            row.add_suffix(&picker);
            row.upcast()
        }
        PreferenceValue::Text(_) => blender_row(preferences.clone(), &title).upcast(),
    }
}

fn blender_row(preferences: SharedPreferences, title: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(shrimply_blender_core::binary_label(
            store::snapshot(&preferences).blender_binary.as_deref(),
        ))
        .build();
    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(tr!("Clear Blender binary").as_ref())
        .valign(gtk::Align::Center)
        .build();
    let choose = gtk::Button::builder()
        .label(tr!("Choose…").as_ref())
        .valign(gtk::Align::Center)
        .build();
    clear.connect_clicked({
        let preferences = preferences.clone();
        let row = row.downgrade();
        move |_| {
            store::apply_blender_binary(&preferences, None);
            if let Some(row) = row.upgrade() {
                row.set_subtitle(tr!("Not configured").as_ref());
            }
        }
    });
    choose.connect_clicked({
        let row = row.downgrade();
        move |button| {
            let picker = gtk::FileDialog::builder()
                .title(tr!("Choose Blender Binary").as_ref())
                .build();
            let preferences = preferences.clone();
            let row = row.clone();
            shrimply_components_gtk::file_picker::open(
                "Choose Blender Binary",
                &picker,
                button.root().and_downcast::<gtk::Window>().as_ref(),
                move |result| {
                    if let Some(path) = result.ok().and_then(|file| file.path()) {
                        gtk::glib::spawn_future_local(async move {
                            let result = gtk::gio::spawn_blocking(move || {
                                store::validate_blender_binary(&path)
                            })
                            .await
                            .expect("Blender probe");
                            let Some(row) = row.upgrade() else {
                                return;
                            };
                            match result {
                                Ok(path) => {
                                    row.set_subtitle(&path.display().to_string());
                                    store::apply_blender_binary(&preferences, Some(path));
                                }
                                Err(error) => {
                                    let alert = adw::AlertDialog::new(
                                        Some(tr!("Incompatible Blender Binary").as_ref()),
                                        Some(&error),
                                    );
                                    alert.add_response("close", tr!("Close").as_ref());
                                    alert.present(Some(&row));
                                }
                            }
                        });
                    }
                },
            );
        }
    });
    row.add_suffix(&clear);
    row.add_suffix(&choose);
    row
}
