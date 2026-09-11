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
    caption_font_size.set_value(snapshot.caption_font_size as f64);
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
    default_visual_duration.set_value(snapshot.default_visual_duration.as_secs_f64());
    default_visual_duration.set_digits(1);

    let snap_radius = adw::SpinRow::with_range(
        snap_range.minimum as f64,
        snap_range.maximum as f64,
        snap_range.step as f64,
    );
    snap_radius.set_title(tr!("Snap Attraction Radius").as_ref());
    snap_radius
        .set_subtitle(tr!("Distance in pixels for timeline, beat, and preview snapping").as_ref());
    snap_radius.set_value(f64::from(snapshot.timeline_snap_radius_px));
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
    preview_padding.set_value(f64::from(snapshot.preview_padding_px));
    preview_padding.set_digits(0);

    let preview_shadow_size = adw::SpinRow::with_range(
        shadow_range.minimum as f64,
        shadow_range.maximum as f64,
        shadow_range.step as f64,
    );
    preview_shadow_size.set_title(tr!("Shadow Size").as_ref());
    preview_shadow_size
        .set_subtitle(tr!("Drop shadow extent around the video frame in pixels").as_ref());
    preview_shadow_size.set_value(f64::from(snapshot.preview_shadow_size_px));
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
    preview_upsample_method.set_selected(match snapshot.preview_upsample_method {
        preferences_store::PreviewUpsampleMethod::Nearest => 0,
        preferences_store::PreviewUpsampleMethod::Bilinear => 1,
    });

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
    preview_downsample_method.set_selected(match snapshot.preview_downsample_method {
        preferences_store::PreviewDownsampleMethod::Nearest => 0,
        preferences_store::PreviewDownsampleMethod::Bilinear => 1,
        preferences_store::PreviewDownsampleMethod::Trilinear => 2,
    });

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
    temporal_decoder_pool_size.set_value(f64::from(snapshot.temporal_decoder_pool_size));
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
    let preferences_store::PreferenceValue::Integer(memory_quarters) =
        preferences_store::value(&preferences, preferences_store::PreferenceId::GpuHostMemory)
    else {
        unreachable!("GPU host memory must have an integer UI representation")
    };
    gpu_host_memory.set_value(memory_quarters as f64 / memory_range.scale as f64);
    gpu_host_memory.set_digits(2);

    let performance_group = adw::PreferencesGroup::new();
    performance_group.set_title(tr!("Performance").as_ref());
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
    dialog.add(&server::page(preferences.clone(), &blender_group));

    let font_preferences = preferences.clone();
    caption_font_size.connect_value_notify(move |row| {
        preferences_store::set_value(
            &font_preferences,
            preferences_store::PreferenceId::CaptionFontSize,
            preferences_store::PreferenceValue::Integer(row.value().round() as i64),
        )
        .expect("caption font size has a fixed integer type");
    });

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

    let duration_store = preferences.clone();
    default_visual_duration.connect_value_notify(move |row| {
        preferences_store::set_value(
            &duration_store,
            preferences_store::PreferenceId::DefaultVisualDuration,
            preferences_store::PreferenceValue::Integer(
                (row.value() * duration_range.scale as f64).round() as i64,
            ),
        )
        .expect("visual duration has a fixed integer UI type");
    });

    let snap_radius_store = preferences.clone();
    snap_radius.connect_value_notify(move |row| {
        preferences_store::set_value(
            &snap_radius_store,
            preferences_store::PreferenceId::TimelineSnapRadius,
            preferences_store::PreferenceValue::Integer(row.value().round() as i64),
        )
        .expect("timeline snap radius has a fixed integer type");
    });

    let preview_padding_store = preferences.clone();
    preview_padding.connect_value_notify(move |row| {
        preferences_store::set_value(
            &preview_padding_store,
            preferences_store::PreferenceId::PreviewPadding,
            preferences_store::PreferenceValue::Integer(row.value().round() as i64),
        )
        .expect("preview padding has a fixed integer type");
    });

    let preview_shadow_store = preferences.clone();
    preview_shadow_size.connect_value_notify(move |row| {
        preferences_store::set_value(
            &preview_shadow_store,
            preferences_store::PreferenceId::PreviewShadowSize,
            preferences_store::PreferenceValue::Integer(row.value().round() as i64),
        )
        .expect("preview shadow size has a fixed integer type");
    });

    let preview_upsample_store = preferences.clone();
    preview_upsample_method.connect_selected_notify(move |row| {
        preferences_store::set_value(
            &preview_upsample_store,
            preferences_store::PreferenceId::PreviewUpsampleMethod,
            preferences_store::PreferenceValue::Integer(i64::from(row.selected())),
        )
        .expect("preview upsample method has a fixed integer UI type");
    });

    let preview_downsample_store = preferences.clone();
    preview_downsample_method.connect_selected_notify(move |row| {
        preferences_store::set_value(
            &preview_downsample_store,
            preferences_store::PreferenceId::PreviewDownsampleMethod,
            preferences_store::PreferenceValue::Integer(i64::from(row.selected())),
        )
        .expect("preview downsample method has a fixed integer UI type");
    });

    let temporal_decoder_pool_store = preferences.clone();
    temporal_decoder_pool_size.connect_value_notify(move |row| {
        preferences_store::set_value(
            &temporal_decoder_pool_store,
            preferences_store::PreferenceId::TemporalDecoderPoolSize,
            preferences_store::PreferenceValue::Integer(row.value().round() as i64),
        )
        .expect("decoder pool size has a fixed integer type");
    });

    let gpu_host_memory_store = preferences.clone();
    gpu_host_memory.connect_value_notify(move |row| {
        preferences_store::set_value(
            &gpu_host_memory_store,
            preferences_store::PreferenceId::GpuHostMemory,
            preferences_store::PreferenceValue::Integer(
                (row.value() * memory_range.scale as f64).round() as i64,
            ),
        )
        .expect("GPU host memory has a fixed integer UI type");
    });

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
