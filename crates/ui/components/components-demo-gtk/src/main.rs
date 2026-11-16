use adw::prelude::*;
use shrimply_components_core::layered::LayeredPropertyController;
use shrimply_components_gtk::playback_shortcuts::attach_space_play_toggle;
use shrimply_components_gtk::ui::{
    ColorPicker, InspectorCard, MultilineTextInput, Number2Picker, Number3Picker, NumberPicker,
    ProgressButton, ProgressButtonState, SingleLineTextInput, StringChoice, control_row,
    labeled_string_selector, live_performance, modifier_menu, property_row, split_button,
    switch_row, tabs,
};
use shrimply_math_color::Color;
use std::{cell::Cell, rc::Rc};

mod transform_property;

fn main() {
    shrimply_components_gtk::i18n::init_system_locale();
    let app = adw::Application::new(
        Some("dev.shrimply.ComponentsShowcase"),
        gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    shrimply_cross_ui_theme::set_dark(adw::StyleManager::default().is_dark());
    shrimply_components_gtk::icons::register_bundled();
    let general = gtk::Box::new(gtk::Orientation::Vertical, 10);
    general.set_margin_top(16);
    general.set_margin_bottom(16);
    general.set_margin_start(16);
    general.set_margin_end(16);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(500), || {
        let measurement = shrimply_profiling::measure("Demo / UI refresh");
        shrimply_profiling::increment("Demo / Refresh count");
        drop(measurement);
        gtk::glib::ControlFlow::Continue
    });

    let events = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .height_request(140)
        .build();
    let log: Rc<dyn Fn(String)> = Rc::new({
        let buffer = events.buffer();
        move |message: String| {
            let (start, end) = buffer.bounds();
            let previous = buffer.text(&start, &end, true);
            buffer.set_text(&format!("{message}\n{previous}"));
        }
    });

    let number = NumberPicker::builder(12.5)
        .accepted_range(-100.0, 100.0)
        .drag_step(0.25)
        .digits(2)
        .unit_name("px")
        .on_change({
            let log = log.clone();
            move |value| log(format!("number changed {value}"))
        })
        .on_commit({
            let log = log.clone();
            move |value| log(format!("number committed {value}"))
        })
        .build();
    general.append(&control_row("Number", &number));

    let pair = Number2Picker::builder(1920.0, 1080.0)
        .minimum(1.0)
        .maximum(16_384.0)
        .digits(0)
        .first_prefix("W")
        .second_prefix("H")
        .unit_name("px")
        .enable_lock()
        .on_first_change({
            let log = log.clone();
            move |value| log(format!("pair first {value}"))
        })
        .on_second_change({
            let log = log.clone();
            move |value| log(format!("pair second {value}"))
        })
        .build_with_handles();
    general.append(&control_row("Pair", &pair.widget));

    let vector = Number3Picker::builder(1.0, 2.0, 3.0)
        .prefixes(["X", "Y", "Z"])
        .enable_lock()
        .on_change(0, {
            let log = log.clone();
            move |value| log(format!("vector 0 {value}"))
        })
        .on_change(1, {
            let log = log.clone();
            move |value| log(format!("vector 1 {value}"))
        })
        .on_change(2, {
            let log = log.clone();
            move |value| log(format!("vector 2 {value}"))
        })
        .build_with_handles();
    general.append(&control_row("Vector", &vector.widget));

    let single = SingleLineTextInput::builder("Editable text")
        .placeholder("Type here")
        .max_length(40)
        .on_commit({
            let log = log.clone();
            move |value| log(format!("text committed {value}"))
        })
        .build();
    general.append(&control_row("Single line", &single));

    let multiline = MultilineTextInput::builder("Try a typo such as teh.")
        .min_content_height(110)
        .max_length(240)
        .on_change(|_| true)
        .on_commit({
            let log = log.clone();
            move || log("multiline committed".to_string())
        })
        .build();
    general.append(&control_row("Multiline", multiline.widget()));

    let selector = labeled_string_selector(
        "Searchable dropdown",
        "two",
        [
            "one", "two", "three", "four", "five", "six", "seven", "eight",
        ]
        .into_iter()
        .map(|value| StringChoice {
            value: value.to_string(),
            label: value.to_uppercase(),
        })
        .collect(),
        {
            let log = log.clone();
            move |value| log(format!("selected {value}"))
        },
    );
    general.append(selector.widget());

    let position_controller = LayeredPropertyController::default();
    let position_graph = transform_property::pair_graph("Position", [960.0, 540.0], log.clone());
    let position_value = Rc::new(Cell::new([960.0, 540.0]));
    let position = Number2Picker::builder(960.0, 540.0)
        .first_prefix("X")
        .second_prefix("Y")
        .unit_name("px")
        .digits(0)
        .on_change(transform_property::pair_edit_handler(
            position_controller.clone(),
            position_graph.clone(),
            position_value.clone(),
        ))
        .build_with_handles();
    let position_row = transform_property::pair_property(
        transform_property::PairPropertyConfig {
            label: "Position",
            initial_values: [960.0, 540.0],
            modes: (true, true),
        },
        &position.widget,
        [&position.first, &position.second],
        position_graph.clone(),
        position_controller.clone(),
        position_value.clone(),
        log.clone(),
    );

    let anchor_controller = LayeredPropertyController::default();
    let anchor_graph = transform_property::pair_graph("Anchor", [960.0, 540.0], log.clone());
    let anchor_value = Rc::new(Cell::new([960.0, 540.0]));
    let anchor = Number2Picker::builder(960.0, 540.0)
        .first_prefix("X")
        .second_prefix("Y")
        .unit_name("px")
        .digits(0)
        .on_change(transform_property::pair_edit_handler(
            anchor_controller.clone(),
            anchor_graph.clone(),
            anchor_value.clone(),
        ))
        .build_with_handles();
    let anchor_row = transform_property::pair_property(
        transform_property::PairPropertyConfig {
            label: "Anchor",
            initial_values: [960.0, 540.0],
            modes: (false, false),
        },
        &anchor.widget,
        [&anchor.first, &anchor.second],
        anchor_graph.clone(),
        anchor_controller.clone(),
        anchor_value.clone(),
        log.clone(),
    );

    let scale_controller = LayeredPropertyController::default();
    let scale_graph = transform_property::pair_graph("Scale", [1.0, 1.0], log.clone());
    let scale_value = Rc::new(Cell::new([1.0, 1.0]));
    let scale = Number2Picker::builder(1.0, 1.0)
        .first_prefix("X")
        .second_prefix("Y")
        .unit_name("x")
        .digits(2)
        .minimum(0.0)
        .enable_lock()
        .on_change(transform_property::pair_edit_handler(
            scale_controller.clone(),
            scale_graph.clone(),
            scale_value.clone(),
        ))
        .build_with_handles();
    let scale_row = transform_property::pair_property(
        transform_property::PairPropertyConfig {
            label: "Scale",
            initial_values: [1.0, 1.0],
            modes: (false, false),
        },
        &scale.widget,
        [&scale.first, &scale.second],
        scale_graph.clone(),
        scale_controller.clone(),
        scale_value.clone(),
        log.clone(),
    );

    let shear_controller = LayeredPropertyController::default();
    let shear_graph = transform_property::pair_graph("Shear", [0.0, 0.0], log.clone());
    let shear_value = Rc::new(Cell::new([0.0, 0.0]));
    let shear = Number2Picker::builder(0.0, 0.0)
        .first_prefix("X")
        .second_prefix("Y")
        .digits(2)
        .on_change(transform_property::pair_edit_handler(
            shear_controller.clone(),
            shear_graph.clone(),
            shear_value.clone(),
        ))
        .build_with_handles();
    let shear_row = transform_property::pair_property(
        transform_property::PairPropertyConfig {
            label: "Shear",
            initial_values: [0.0, 0.0],
            modes: (false, false),
        },
        &shear.widget,
        [&shear.first, &shear.second],
        shear_graph.clone(),
        shear_controller.clone(),
        shear_value.clone(),
        log.clone(),
    );

    let rotation_controller = LayeredPropertyController::default();
    let rotation_graph = transform_property::graph("Rotation", 0.0, log.clone());
    let rotation = NumberPicker::builder(0.0)
        .drag_step(0.1)
        .digits(1)
        .unit_name("°")
        .rotating_prefix_icon_name("arrow3-up-symbolic")
        .on_change(transform_property::edit_handler(
            rotation_controller.clone(),
            rotation_graph.clone(),
        ))
        .build_with_handle();
    let rotation_row = transform_property::scalar_property(
        transform_property::PropertyConfig {
            label: "Rotation",
            initial_value: 0.0,
            modes: (false, false),
        },
        &rotation.widget,
        &rotation.handle,
        rotation_graph.clone(),
        rotation_controller,
        log.clone(),
    );
    let transform = InspectorCard::new("Transform", true, {
        let position_graph = position_graph.clone();
        let anchor_graph = anchor_graph.clone();
        let scale_graph = scale_graph.clone();
        let shear_graph = shear_graph.clone();
        let rotation_graph = rotation_graph.clone();
        let position_first = position.first.clone();
        let position_second = position.second.clone();
        let anchor_first = anchor.first.clone();
        let anchor_second = anchor.second.clone();
        let scale_first = scale.first.clone();
        let scale_second = scale.second.clone();
        let shear_first = shear.first.clone();
        let shear_second = shear.second.clone();
        let rotation = rotation.handle.clone();
        let position_value = position_value.clone();
        let anchor_value = anchor_value.clone();
        let scale_value = scale_value.clone();
        let shear_value = shear_value.clone();
        let position_controller = position_controller.clone();
        let anchor_controller = anchor_controller.clone();
        let scale_controller = scale_controller.clone();
        let shear_controller = shear_controller.clone();
        let log = log.clone();
        move || {
            transform_property::reset_pair(
                &position_graph,
                &position_controller,
                [&position_first, &position_second],
                &position_value,
                [960.0, 540.0],
            );
            transform_property::reset_pair(
                &anchor_graph,
                &anchor_controller,
                [&anchor_first, &anchor_second],
                &anchor_value,
                [960.0, 540.0],
            );
            transform_property::reset_pair(
                &scale_graph,
                &scale_controller,
                [&scale_first, &scale_second],
                &scale_value,
                [1.0, 1.0],
            );
            transform_property::reset_pair(
                &shear_graph,
                &shear_controller,
                [&shear_first, &shear_second],
                &shear_value,
                [0.0, 0.0],
            );
            rotation_graph.replace_state(shrimply_components_demo_core::property_graph_state(0.0));
            rotation.set_f64(0.0);
            log("transform reset".to_string());
        }
    });
    transform.append(position_row.widget());
    transform.append(anchor_row.widget());
    transform.append(scale_row.widget());
    transform.append(shear_row.widget());
    transform.append(rotation_row.widget());
    let transform_group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    transform_group.append(transform.widget());
    transform_group.append(&modifier_menu(
        shrimply_components_demo_core::modifier_names()
            .into_iter()
            .map(|name| StringChoice {
                value: name.to_string(),
                label: name.to_string(),
            })
            .collect(),
        {
            let log = log.clone();
            move |value| log(format!("add modifier {value}"))
        },
    ));
    general.append(&transform_group);
    general.append(&live_performance());
    general.append(&switch_row("Enabled", Some("Toggle this option"), true, {
        let log = log.clone();
        move |value| log(format!("switch {value}"))
    }));

    let color = ColorPicker::builder(Color::new(0x35, 0x84, 0xe4, 0xcc))
        .on_change({
            let log = log.clone();
            move |value| {
                log(format!(
                    "color #{:02X}{:02X}{:02X}{:02X}",
                    value.r, value.g, value.b, value.a
                ))
            }
        })
        .build();
    general.append(&control_row("Color", &color));
    general.append(&control_row(
        "Split",
        &split_button(
            "Primary",
            "Secondary",
            {
                let log = log.clone();
                move |_| log("primary".to_string())
            },
            {
                let log = log.clone();
                move || log("secondary".to_string())
            },
        ),
    ));

    let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let idle = ProgressButton::new("Idle");
    let indeterminate = ProgressButton::new("Working");
    indeterminate.set_state(ProgressButtonState::Indeterminate);
    let half = ProgressButton::new("Half");
    half.set_state(ProgressButtonState::Progress(0.5));
    progress_row.append(idle.widget());
    progress_row.append(indeterminate.widget());
    progress_row.append(half.widget());
    general.append(&control_row("Progress", &progress_row));

    let playback = gtk::Frame::builder()
        .child(&gtk::Label::new(Some("Click, then press Space or L")))
        .height_request(44)
        .build();
    attach_space_play_toggle(
        &playback,
        {
            let log = log.clone();
            move || log("toggle playback".to_string())
        },
        {
            let log = log.clone();
            move || log("step playback speed".to_string())
        },
    );
    general.append(&control_row("Playback keys", &playback));

    let general_scroller = gtk::ScrolledWindow::builder()
        .child(&general)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let info = adw::PreferencesGroup::builder()
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    info.add(&property_row("Selected item", "Example clip · 00:00:02:00"));
    info.add(&property_row(
        "Component package",
        "shrimply-components-gtk",
    ));
    info.add(&property_row("Frame graph", "Shared Rust renderer"));
    let home = gtk::glib::home_dir();
    let folder = property_row("Home folder", &home.display().to_string());
    let reveal = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Show in Folder")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    reveal.connect_clicked({
        let log = log.clone();
        move |button| {
            if let Err(error) = shrimply_components_gtk::desktop_open::show_path_in_folder(
                button.upcast_ref(),
                home.clone(),
            ) {
                log(format!("show in folder failed: {error}"));
            }
        }
    });
    folder.add_suffix(&reveal);
    folder.set_activatable_widget(Some(&reveal));
    info.add(&folder);
    let info_scroller = gtk::ScrolledWindow::builder()
        .child(&info)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let log_page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    log_page.set_margin_top(16);
    log_page.set_margin_bottom(16);
    log_page.set_margin_start(16);
    log_page.set_margin_end(16);
    let log_scroller = gtk::ScrolledWindow::builder()
        .child(&events)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    log_page.set_vexpand(true);
    log_page.append(&log_scroller);

    let pages = tabs([
        (
            "General".to_string(),
            "preferences-system-symbolic".to_string(),
            general_scroller.upcast(),
        ),
        (
            "Info".to_string(),
            "dialog-information-symbolic".to_string(),
            info_scroller.upcast(),
        ),
        (
            "Log".to_string(),
            "utilities-terminal-symbolic".to_string(),
            log_page.upcast(),
        ),
    ]);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(true);
    header.set_title_widget(Some(
        &gtk::Label::builder()
            .label("Shrimply GTK Components")
            .css_classes(["title"])
            .build(),
    ));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&pages));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Shrimply GTK Components")
        .default_width(920)
        .default_height(900)
        .content(&toolbar)
        .build();
    window.present();
}
