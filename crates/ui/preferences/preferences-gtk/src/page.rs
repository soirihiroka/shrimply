use crate::{server, store as preferences_store};
use adw::prelude::*;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::ColorPicker;

pub fn show_preferences_dialog(
    window: &adw::ApplicationWindow,
    preferences: preferences_store::SharedPreferences,
) {
    let snapshot = preferences_store::snapshot(&preferences);
    let caption_size_range =
        preferences_store::integer_range(preferences_store::PreferenceId::CaptionFontSize)
            .expect("caption font size must be numeric");
    let duration_range =
        preferences_store::integer_range(preferences_store::PreferenceId::DefaultVisualDuration)
            .expect("visual duration must be numeric");
    let snap_range =
        preferences_store::integer_range(preferences_store::PreferenceId::TimelineSnapRadius)
            .expect("timeline snap radius must be numeric");
    let padding_range =
        preferences_store::integer_range(preferences_store::PreferenceId::PreviewPadding)
            .expect("preview padding must be numeric");
    let shadow_range =
        preferences_store::integer_range(preferences_store::PreferenceId::PreviewShadowSize)
            .expect("preview shadow size must be numeric");
    let decoder_range =
        preferences_store::integer_range(preferences_store::PreferenceId::TemporalDecoderPoolSize)
            .expect("decoder pool size must be numeric");
    let memory_range =
        preferences_store::integer_range(preferences_store::PreferenceId::GpuHostMemory)
            .expect("GPU host memory must be numeric");

    let default_font_row = adw::ActionRow::builder()
        .title(tr!("Default Text Font").as_ref())
        .build();
    let default_font =
        shrimply_inspector_gtk::font_family_selector(&snapshot.default_text_font_family, {
            let preferences = preferences.clone();
            move |family| preferences_store::set_default_text_font_family(&preferences, family)
        });
    default_font.set_valign(gtk::Align::Center);
    default_font.set_width_request(260);
    default_font_row.add_suffix(&default_font);
    let text_group = adw::PreferencesGroup::new();
    text_group.set_title(tr!("Text").as_ref());
    text_group.add(&default_font_row);

    let caption_font_size = adw::SpinRow::with_range(
        caption_size_range.minimum as f64,
        caption_size_range.maximum as f64,
        caption_size_range.step as f64,
    );
    caption_font_size.set_title(tr!("Font Size").as_ref());
    caption_font_size.set_digits(0);

    let caption_background_color_row = adw::ActionRow::builder()
        .title(tr!("Background Color").as_ref())
        .build();

    let caption_group = adw::PreferencesGroup::new();
    caption_group.set_title(tr!("Captions").as_ref());
    caption_group.add(&caption_font_size);
    caption_group.add(&caption_background_color_row);

    let default_visual_duration = adw::SpinRow::with_range(
        duration_range.minimum as f64 / duration_range.scale as f64,
        duration_range.maximum as f64 / duration_range.scale as f64,
        duration_range.step as f64 / duration_range.scale as f64,
    );
    default_visual_duration.set_title(tr!("Default Visual Duration").as_ref());
    default_visual_duration.set_digits(1);

    let snap_radius = adw::SpinRow::with_range(
        snap_range.minimum as f64,
        snap_range.maximum as f64,
        snap_range.step as f64,
    );
    snap_radius.set_title(tr!("Snap Attraction Radius").as_ref());
    snap_radius
        .set_subtitle(tr!("Distance in pixels for timeline, beat, and preview snapping").as_ref());
    snap_radius.set_digits(0);

    let timeline_group = adw::PreferencesGroup::new();
    timeline_group.set_title(tr!("Timeline").as_ref());
    timeline_group.add(&default_visual_duration);
    timeline_group.add(&snap_radius);

    let preview_padding = adw::SpinRow::with_range(
        padding_range.minimum as f64,
        padding_range.maximum as f64,
        padding_range.step as f64,
    );
    preview_padding.set_title(tr!("Padding").as_ref());
    preview_padding.set_subtitle(tr!("Space around the video frame in pixels").as_ref());
    preview_padding.set_digits(0);

    let preview_shadow_size = adw::SpinRow::with_range(
        shadow_range.minimum as f64,
        shadow_range.maximum as f64,
        shadow_range.step as f64,
    );
    preview_shadow_size.set_title(tr!("Shadow Size").as_ref());
    preview_shadow_size
        .set_subtitle(tr!("Drop shadow extent around the video frame in pixels").as_ref());
    preview_shadow_size.set_digits(0);

    let preview_upsample_method = adw::ComboRow::new();
    preview_upsample_method.set_title(tr!("Preview Upsample Method").as_ref());
    preview_upsample_method
        .set_subtitle(tr!("Filter used when the preview is larger than the video").as_ref());
    let upsample_labels = [tr!("Nearest"), tr!("Bilinear")];
    preview_upsample_method.set_model(Some(&gtk::StringList::new(
        &upsample_labels
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
    )));

    let preview_downsample_method = adw::ComboRow::new();
    preview_downsample_method.set_title(tr!("Preview Downsample Method").as_ref());
    preview_downsample_method
        .set_subtitle(tr!("Filter used when the preview is smaller than the video").as_ref());
    let downsample_labels = [tr!("Nearest"), tr!("Bilinear"), tr!("Trilinear")];
    preview_downsample_method.set_model(Some(&gtk::StringList::new(
        &downsample_labels
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
    )));

    let preview_group = adw::PreferencesGroup::new();
    preview_group.set_title(tr!("Preview").as_ref());
    let preview_zoom_pan = adw::SwitchRow::builder()
        .title(tr!("Preview zoom and pan").as_ref())
        .subtitle(tr!("Scroll to zoom; middle-drag to pan; middle-click to fit.").as_ref())
        .active(snapshot.preview_zoom_pan_enabled)
        .build();
    let zoom_preferences = preferences.clone();
    preview_zoom_pan.connect_active_notify(move |row| {
        preferences_store::set_preview_zoom_pan_enabled(&zoom_preferences, row.is_active());
    });
    preview_group.add(&preview_zoom_pan);
    preview_group.add(&preview_padding);
    preview_group.add(&preview_shadow_size);
    preview_group.add(&preview_upsample_method);
    preview_group.add(&preview_downsample_method);

    let temporal_decoder_pool_size = adw::SpinRow::with_range(
        decoder_range.minimum as f64,
        decoder_range.maximum as f64,
        decoder_range.step as f64,
    );
    temporal_decoder_pool_size.set_title(tr!("Temporal Decoder Pool Size").as_ref());
    temporal_decoder_pool_size
        .set_subtitle(tr!("Maximum number of active video decoder sessions").as_ref());
    temporal_decoder_pool_size.set_digits(0);

    let gpu_host_memory = adw::SpinRow::with_range(
        memory_range.minimum as f64 / memory_range.scale as f64,
        memory_range.maximum as f64 / memory_range.scale as f64,
        memory_range.step as f64 / memory_range.scale as f64,
    );
    gpu_host_memory.set_title(tr!("GPU Host Memory Budget").as_ref());
    gpu_host_memory.set_subtitle(
        tr!("Maximum system RAM available for CUDA spill and reconstructible GPU resources")
            .as_ref(),
    );
    gpu_host_memory.set_digits(2);

    let performance_group = adw::PreferencesGroup::new();
    performance_group.set_title(tr!("Performance").as_ref());
    #[cfg(target_os = "linux")]
    performance_group.add(&crate::acceleration::row(preferences.clone()));
    performance_group.add(&temporal_decoder_pool_size);
    performance_group.add(&gpu_host_memory);

    let appearance_page = adw::PreferencesPage::builder()
        .title(tr!("Appearance").as_ref())
        .icon_name("appearance-symbolic")
        .name("appearance")
        .build();
    appearance_page.add(&caption_group);
    appearance_page.add(&text_group);
    appearance_page.add(&preview_group);
    appearance_page.add(&timeline_group);

    let performance_page = adw::PreferencesPage::builder()
        .title(tr!("Performance").as_ref())
        .icon_name("speedometer-symbolic")
        .name("performance")
        .build();
    performance_page.add(&performance_group);

    let blender_row = adw::ActionRow::builder()
        .title(tr!("Blender Binary").as_ref())
        .subtitle(shrimply_blender_core::binary_label(
            snapshot.blender_binary.as_deref(),
        ))
        .build();
    let clear_blender = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(tr!("Clear Blender binary").as_ref())
        .valign(gtk::Align::Center)
        .build();
    let choose_blender = gtk::Button::builder()
        .label(tr!("Choose…").as_ref())
        .valign(gtk::Align::Center)
        .build();
    blender_row.add_suffix(&clear_blender);
    blender_row.add_suffix(&choose_blender);
    let blender_group = adw::PreferencesGroup::new();
    blender_group.set_title(tr!("Blender").as_ref());
    blender_group.add(&blender_row);

    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Preferences").as_ref())
        .search_enabled(false)
        .build();
    dialog.add(&appearance_page);
    dialog.add(&performance_page);
    dialog.add(&crate::trust::page());
    dialog.add(&server::page(preferences.clone(), &blender_group));

    let color_store = preferences.clone();
    let color_button = ColorPicker::builder(snapshot.caption_background_color)
        .title(tr!("Caption background color").as_ref())
        .on_change(move |color| {
            preferences_store::set_value(
                &color_store,
                preferences_store::PreferenceId::CaptionBackgroundColor,
                preferences_store::PreferenceValue::Color(color),
            )
            .expect("caption background color has a fixed color type")
        })
        .build();
    color_button.set_valign(gtk::Align::Center);
    color_button.set_hexpand(false);
    color_button.set_vexpand(false);
    caption_background_color_row.add_suffix(&color_button);

    use preferences_store::{PreferenceId as Id, PreferenceValue as Value};
    for (row, id) in [
        (caption_font_size, Id::CaptionFontSize),
        (default_visual_duration, Id::DefaultVisualDuration),
        (snap_radius, Id::TimelineSnapRadius),
        (preview_padding, Id::PreviewPadding),
        (preview_shadow_size, Id::PreviewShadowSize),
        (temporal_decoder_pool_size, Id::TemporalDecoderPoolSize),
        (gpu_host_memory, Id::GpuHostMemory),
    ] {
        let store = preferences.clone();
        let range = preferences_store::integer_range(id).expect("numeric preference range");
        let Value::Integer(current) = preferences_store::value(&store, id) else {
            unreachable!("numeric preference value");
        };
        row.set_value(current as f64 / range.scale as f64);
        row.connect_value_notify(move |row| {
            preferences_store::set_value(
                &store,
                id,
                Value::Integer((row.value() * range.scale as f64).round() as i64),
            )
            .expect("numeric preference has an integer UI value");
        });
    }
    for (row, id) in [
        (preview_upsample_method, Id::PreviewUpsampleMethod),
        (preview_downsample_method, Id::PreviewDownsampleMethod),
    ] {
        let store = preferences.clone();
        let Value::Integer(current) = preferences_store::value(&store, id) else {
            unreachable!("preference choice value");
        };
        row.set_selected(current.try_into().expect("nonnegative preference choice"));
        row.connect_selected_notify(move |row| {
            preferences_store::set_value(&store, id, Value::Integer(i64::from(row.selected())))
                .expect("preference choice has an integer UI value");
        });
    }

    let clear_store = preferences.clone();
    let clear_row = blender_row.clone();
    clear_blender.connect_clicked(move |_| {
        preferences_store::apply_blender_binary(&clear_store, None);
        clear_row.set_subtitle(tr!("Not configured").as_ref());
    });

    let choose_store = preferences.clone();
    let choose_row = blender_row.clone();
    let choose_dialog = dialog.clone();
    choose_blender.connect_clicked(move |_| {
        let picker = gtk::FileDialog::builder()
            .title(tr!("Choose Blender Binary").as_ref())
            .build();
        let store = choose_store.clone();
        let row = choose_row.clone();
        let parent = choose_dialog.clone();
        shrimply_components_gtk::file_picker::open(
            "Choose Blender Binary",
            &picker,
            parent.root().and_downcast::<gtk::Window>().as_ref(),
            move |result| {
                let Some(path) = result.ok().and_then(|file| file.path()) else {
                    return;
                };
                let (sender, receiver) = async_channel::bounded(1);
                let probe_path = path.clone();
                std::thread::spawn(move || {
                    let result = preferences_store::validate_blender_binary(&probe_path);
                    let _ = sender.send_blocking(result);
                });
                let store = store.clone();
                let parent = parent.clone();
                gtk::glib::spawn_future_local(async move {
                    let Ok(result) = receiver.recv().await else {
                        return;
                    };
                    match result {
                        Ok(path) => {
                            preferences_store::apply_blender_binary(&store, Some(path.clone()));
                            row.set_subtitle(&path.display().to_string());
                        }
                        Err(error) => {
                            let alert = adw::AlertDialog::new(
                                Some("Incompatible Blender Binary"),
                                Some(&error),
                            );
                            alert.add_response("close", tr!("Close").as_ref());
                            alert.present(Some(parent.upcast_ref::<gtk::Widget>()));
                        }
                    }
                });
            },
        );
    });

    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
}
