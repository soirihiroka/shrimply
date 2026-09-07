pub use shrimply_export_core::{audio, video};
use shrimply_export_core::{json, output};
use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nMenuExt;

pub use shrimply_export_core::{caption, project, time_format};
pub use shrimply_gtk_components::desktop_open;

use shrimply_math_media as math;

use crate::caption::ytt;
use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_math_core::Fraction;
use shrimply_state::preferences;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::{Duration, Instant};

pub fn build_export_button(
    window: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    project: Rc<RefCell<project::Project>>,
    preferences: preferences::SharedPreferences,
) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append_i18n("Export video", "export.video");
    menu.append_i18n("Export captions (YTT)", "export.ytt");
    menu.append_i18n("Export JSON", "export.json");

    let actions = gio::SimpleActionGroup::new();
    add_menu_action(&actions, "video", {
        let window = window.clone();
        let toasts = toasts.clone();
        let project = project.clone();
        let preferences = preferences.clone();
        move || open_export_page(&window, &toasts, project.clone(), preferences.clone())
    });
    add_menu_action(&actions, "json", {
        let window = window.clone();
        let project = project.clone();
        let toasts = toasts.clone();
        move || open_project_json_dialog(&window, &toasts, project.clone())
    });
    add_menu_action(&actions, "ytt", {
        let window = window.clone();
        let toasts = toasts.clone();
        let project = project.clone();
        move || open_ytt_dialog(&window, &toasts, project.clone())
    });

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.insert_action_group("export", Some(&actions));

    let button = gtk::MenuButton::builder()
        .icon_name("share-symbolic")
        .tooltip_text(tr!("Export").as_ref())
        .has_frame(false)
        .popover(&popover)
        .build();
    button.add_css_class("flat");
    button
}

fn open_ytt_dialog(
    parent: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    project: Rc<RefCell<project::Project>>,
) {
    let merge = gtk::CheckButton::new();
    merge.set_active(true);
    let merge_row = adw::ActionRow::builder()
        .title(tr!("Merge into one file").as_ref())
        .activatable(true)
        .build();
    merge_row.add_prefix(&merge);
    merge_row.set_activatable_widget(Some(&merge));

    let separate = gtk::CheckButton::new();
    separate.set_group(Some(&merge));
    let separate_row = adw::ActionRow::builder()
        .title(tr!("Export each track separately").as_ref())
        .activatable(true)
        .build();
    separate_row.add_prefix(&separate);
    separate_row.set_activatable_widget(Some(&separate));

    let modes = gtk::ListBox::new();
    modes.add_css_class("boxed-list");
    modes.set_selection_mode(gtk::SelectionMode::None);
    modes.append(&merge_row);
    modes.append(&separate_row);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.append(&modes);

    let export = gtk::Button::with_label(tr!("Export YTT").as_ref());
    export.add_css_class("suggested-action");
    export.add_css_class("pill");
    export.set_halign(gtk::Align::End);
    content.append(&export);

    let dialog = adw::Dialog::builder()
        .title(tr!("Export Captions").as_ref())
        .content_width(400)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let export_parent = parent.clone();
    let toasts = toasts.clone();
    let close = dialog.clone();
    export.connect_clicked(move |_| {
        let export_mode = if merge.is_active() {
            ytt::ExportMode::Merge
        } else {
            ytt::ExportMode::Separate
        };
        let label = "Export YouTube Captions";
        let file_dialog = gtk::FileDialog::builder()
            .title(tr!(label).as_ref())
            .initial_name(output::default_filename(&project.borrow(), "ytt"))
            .build();
        let parent_for_result = export_parent.clone();
        let toasts = toasts.clone();
        let project = project.clone();
        shrimply_gtk_components::file_picker::save(
            label,
            &file_dialog,
            Some(export_parent.upcast_ref::<gtk::Window>()),
            move |result| {
                let Some(file) = result.ok() else {
                    return;
                };
                let Some(path) = file.path() else {
                    show_export_error(
                        &parent_for_result,
                        "Could not export captions",
                        "Could not resolve the selected file path.",
                    );
                    return;
                };
                let path = output::ensure_extension(path, "ytt");
                match ytt::export(&project.borrow(), &path, export_mode) {
                    Ok(paths) => {
                        if let Some(path) = paths.first() {
                            let title = if paths.len() == 1 {
                                tr!("Captions exported").into_owned()
                            } else {
                                shrimply_gtk_components::i18n::text_args(
                                    "%{count} caption files exported",
                                    &[("count", paths.len().to_string())],
                                )
                            };
                            shrimply_gtk_components::export_feedback::show_export_finished_text(
                                &toasts,
                                &parent_for_result,
                                &title,
                                path,
                            );
                        }
                    }
                    Err(error) => {
                        show_export_error(&parent_for_result, "Could not export captions", &error)
                    }
                }
            },
        );
        close.close();
    });
    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
}

fn add_menu_action<F>(group: &gio::SimpleActionGroup, name: &str, activate: F)
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    group.add_action(&action);
}

#[derive(Clone)]
struct TypedComboRow<T> {
    row: adw::ComboRow,
    values: Rc<[T]>,
}

impl<T: Copy + PartialEq + 'static> TypedComboRow<T> {
    fn new<S: AsRef<str>>(
        title: &str,
        choices: impl IntoIterator<Item = (T, S)>,
        selected: T,
    ) -> Self {
        let choices = choices
            .into_iter()
            .map(|(value, label)| (value, tr!(label.as_ref()).into_owned()))
            .collect::<Vec<_>>();
        let selected = choices
            .iter()
            .position(|(value, _)| *value == selected)
            .expect("combo box default must be one of its choices") as u32;
        let labels = choices
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>();
        let row = adw::ComboRow::new();
        row.set_title(tr!(title).as_ref());
        row.set_model(Some(&gtk::StringList::new(&labels)));
        row.set_selected(selected);
        Self {
            row,
            values: choices.into_iter().map(|(value, _)| value).collect(),
        }
    }

    fn selected(&self) -> T {
        *self
            .values
            .get(self.row.selected() as usize)
            .expect("combo box selection must be one of its choices")
    }

    fn widget(&self) -> &adw::ComboRow {
        &self.row
    }

    fn connect_selected<F: Fn(T) + 'static>(&self, callback: F) {
        let values = self.values.clone();
        self.row.connect_selected_notify(move |row| {
            let selected = *values
                .get(row.selected() as usize)
                .expect("combo box selection must be one of its choices");
            callback(selected);
        });
    }
}

impl<T> Deref for TypedComboRow<T> {
    type Target = adw::ComboRow;

    fn deref(&self) -> &Self::Target {
        &self.row
    }
}

fn open_export_page(
    parent: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    project: Rc<RefCell<project::Project>>,
    preferences: preferences::SharedPreferences,
) {
    let project_fps = project.borrow().fps;
    let format_row = TypedComboRow::new(
        "Export format",
        [
            (video::ExportVideoCodec::H264, "H.264"),
            (video::ExportVideoCodec::H265, "H.265"),
            (video::ExportVideoCodec::Gif, "GIF"),
        ],
        video::ExportVideoCodec::H265,
    );

    let container_row = TypedComboRow::new(
        "Container",
        [
            (video::ExportContainer::Mp4, "MP4"),
            (video::ExportContainer::Mkv, "MKV"),
        ],
        video::ExportContainer::Mp4,
    );

    let mut fps_rates = project::COMMON_FRAME_RATES
        .iter()
        .map(|rate| (rate.value, rate.label.to_string()))
        .collect::<Vec<_>>();
    if !fps_rates.iter().any(|(fps, _)| *fps == project_fps) {
        fps_rates.push((project_fps, project::fraction_as_label(project_fps)));
    }
    let fps_row = TypedComboRow::new("Frame rate", fps_rates, project_fps);

    let background_alpha_row = adw::SpinRow::with_range(0.0, f64::from(u8::MAX), 1.0);
    background_alpha_row.set_title(tr!("Background Alpha").as_ref());
    background_alpha_row
        .set_subtitle(tr!("Values below 128 are transparent in GIF output").as_ref());
    background_alpha_row.set_value(0.0);
    background_alpha_row.set_digits(0);
    background_alpha_row.set_visible(false);

    let rate_control_row = TypedComboRow::new(
        "Rate Control",
        [
            (video::ExportRateControl::ConstantQp, "Constant QP"),
            (
                video::ExportRateControl::ConstantBitrate,
                "Constant Bitrate",
            ),
            (
                video::ExportRateControl::VariableBitrate,
                "Variable Bitrate",
            ),
            (
                video::ExportRateControl::VariableBitrateTargetQuality,
                "Variable Bitrate with Target Quality",
            ),
            (video::ExportRateControl::Lossless, "Lossless"),
        ],
        video::ExportRateControl::ConstantQp,
    );

    let bitrate_row = adw::SpinRow::with_range(50.0, 250_000.0, 50.0);
    bitrate_row.set_title(tr!("Video Bitrate").as_ref());
    bitrate_row.set_subtitle(tr!("Kbps for CBR/VBR").as_ref());
    bitrate_row.set_value(10_000.0);
    bitrate_row.set_digits(0);

    let max_bitrate_row = adw::SpinRow::with_range(50.0, 250_000.0, 50.0);
    max_bitrate_row.set_title(tr!("Max Video Bitrate").as_ref());
    max_bitrate_row.set_subtitle(tr!("Kbps for VBR").as_ref());
    max_bitrate_row.set_value(10_000.0);
    max_bitrate_row.set_digits(0);

    let target_quality_row = adw::SpinRow::with_range(1.0, 51.0, 1.0);
    target_quality_row.set_title(tr!("Target Quality").as_ref());
    target_quality_row.set_subtitle(tr!("Lower values are higher quality").as_ref());
    target_quality_row.set_value(20.0);
    target_quality_row.set_digits(0);

    let default_constant_qp_row = adw::SpinRow::with_range(0.0, 51.0, 1.0);
    default_constant_qp_row.set_title(tr!("Constant QP").as_ref());
    default_constant_qp_row.set_subtitle(tr!("Lower values are higher quality").as_ref());
    default_constant_qp_row.set_value(20.0);
    default_constant_qp_row.set_digits(0);

    let keyframe_interval_row = adw::SpinRow::with_range(0.0, 10.0, 1.0);
    keyframe_interval_row.set_title(tr!("Keyframe Interval").as_ref());
    keyframe_interval_row.set_subtitle(tr!("Seconds; 0 lets NVENC choose").as_ref());
    keyframe_interval_row.set_value(0.0);
    keyframe_interval_row.set_digits(0);

    let preset_row = TypedComboRow::new(
        "Preset",
        [
            (video::ExportPreset::P1, "P1: Fastest (Lowest Quality)"),
            (video::ExportPreset::P2, "P2: Faster (Lower Quality)"),
            (video::ExportPreset::P3, "P3: Fast (Low Quality)"),
            (video::ExportPreset::P4, "P4: Medium (Medium Quality)"),
            (video::ExportPreset::P5, "P5: Slow (Good Quality)"),
            (video::ExportPreset::P6, "P6: Slower (Better Quality)"),
            (video::ExportPreset::P7, "P7: Slowest (Best Quality)"),
        ],
        video::ExportPreset::P6,
    );

    let tuning_row = TypedComboRow::new(
        "Tuning",
        [
            (video::ExportTuning::UltraHighQuality, "Ultra High Quality"),
            (video::ExportTuning::HighQuality, "High Quality"),
            (video::ExportTuning::LowLatency, "Low Latency"),
            (video::ExportTuning::UltraLowLatency, "Ultra Low Latency"),
        ],
        video::ExportTuning::HighQuality,
    );

    let multipass_row = TypedComboRow::new(
        "Multi Pass",
        [
            (video::ExportMultipass::SinglePass, "Single Pass"),
            (
                video::ExportMultipass::QuarterResolution,
                "Two Passes (Quarter Resolution)",
            ),
            (
                video::ExportMultipass::FullResolution,
                "Two Passes (Full Resolution)",
            ),
        ],
        video::ExportMultipass::QuarterResolution,
    );

    let profile_row = TypedComboRow::new(
        "Profile",
        [
            (video::ExportProfile::Main, "Main"),
            (video::ExportProfile::Main10, "Main10"),
        ],
        video::ExportProfile::Main,
    );

    let look_ahead_row = adw::SwitchRow::new();
    look_ahead_row.set_title(tr!("Look-ahead").as_ref());
    look_ahead_row.set_active(true);

    let adaptive_quantization_row = adw::SwitchRow::new();
    adaptive_quantization_row.set_title(tr!("Adaptive Quantization").as_ref());
    adaptive_quantization_row.set_active(true);

    let b_frames_row = adw::SpinRow::with_range(0.0, 16.0, 1.0);
    b_frames_row.set_title(tr!("B Frames").as_ref());
    b_frames_row.set_value(2.0);
    b_frames_row.set_digits(0);

    let b_frame_as_reference_row = adw::SwitchRow::new();
    b_frame_as_reference_row.set_title(tr!("B Frame as Reference").as_ref());
    b_frame_as_reference_row.set_active(false);

    let audio_encoder_row = TypedComboRow::new(
        "Audio Encoder",
        [
            (video::ExportAudioEncoder::Aac, "AAC"),
            (video::ExportAudioEncoder::Opus, "Opus"),
        ],
        video::ExportAudioEncoder::Aac,
    );

    let audio_sample_rate_row = TypedComboRow::new(
        "Audio Sample Rate",
        [(44_100, "44100"), (48_000, "48000"), (96_000, "96000")],
        48_000,
    );

    let audio_bitrate_row = adw::SpinRow::with_range(32.0, 512.0, 8.0);
    audio_bitrate_row.set_title(tr!("Audio Bitrate").as_ref());
    audio_bitrate_row.set_subtitle(tr!("Kbps").as_ref());
    audio_bitrate_row.set_value(192.0);
    audio_bitrate_row.set_digits(0);

    bitrate_row.set_visible(false);
    max_bitrate_row.set_visible(false);
    target_quality_row.set_visible(false);
    rate_control_row.connect_selected({
        let bitrate_row = bitrate_row.clone();
        let max_bitrate_row = max_bitrate_row.clone();
        let target_quality_row = target_quality_row.clone();
        let constant_qp_row = default_constant_qp_row.clone();
        let tuning_row = tuning_row.clone();
        move |selected| {
            constant_qp_row.set_visible(selected == video::ExportRateControl::ConstantQp);
            bitrate_row.set_visible(matches!(
                selected,
                video::ExportRateControl::ConstantBitrate
                    | video::ExportRateControl::VariableBitrate
                    | video::ExportRateControl::VariableBitrateTargetQuality
            ));
            max_bitrate_row.set_visible(matches!(
                selected,
                video::ExportRateControl::VariableBitrate
                    | video::ExportRateControl::VariableBitrateTargetQuality
            ));
            target_quality_row
                .set_visible(selected == video::ExportRateControl::VariableBitrateTargetQuality);
            tuning_row.set_visible(selected != video::ExportRateControl::Lossless);
        }
    });

    let format_group = adw::PreferencesGroup::builder()
        .title(tr!("Video format").as_ref())
        .build();
    format_group.add(format_row.widget());
    format_group.add(container_row.widget());

    let encoder_group = adw::PreferencesGroup::builder()
        .title(tr!("Encoder settings").as_ref())
        .build();
    encoder_group.add(rate_control_row.widget());
    encoder_group.add(&bitrate_row);
    encoder_group.add(&max_bitrate_row);
    encoder_group.add(&target_quality_row);
    encoder_group.add(&default_constant_qp_row);
    encoder_group.add(&keyframe_interval_row);
    encoder_group.add(preset_row.widget());
    encoder_group.add(tuning_row.widget());
    encoder_group.add(multipass_row.widget());
    encoder_group.add(profile_row.widget());
    encoder_group.add(&look_ahead_row);
    encoder_group.add(&adaptive_quantization_row);
    encoder_group.add(&b_frames_row);
    encoder_group.add(&b_frame_as_reference_row);

    let output_group = adw::PreferencesGroup::builder()
        .title(tr!("Output").as_ref())
        .build();
    output_group.add(fps_row.widget());
    output_group.add(&background_alpha_row);

    let audio_group = adw::PreferencesGroup::builder()
        .title(tr!("Audio").as_ref())
        .build();
    audio_group.add(audio_encoder_row.widget());
    audio_group.add(audio_sample_rate_row.widget());
    audio_group.add(&audio_bitrate_row);

    format_row.connect_selected({
        let container_row = container_row.clone();
        let encoder_group = encoder_group.clone();
        let audio_group = audio_group.clone();
        let background_alpha_row = background_alpha_row.clone();
        move |format| {
            let nvenc = format != video::ExportVideoCodec::Gif;
            container_row.set_visible(nvenc);
            encoder_group.set_visible(nvenc);
            audio_group.set_visible(nvenc);
            background_alpha_row.set_visible(!nvenc);
        }
    });

    let actions_group = adw::PreferencesGroup::builder().build();
    let export_action = adw::ButtonRow::builder()
        .title(tr!("Export").as_ref())
        .build();
    export_action.add_css_class("suggested-action");
    actions_group.add(&export_action);

    let page = adw::PreferencesPage::builder()
        .title(tr!("Export").as_ref())
        .name("export")
        .build();
    page.add(&format_group);
    page.add(&encoder_group);
    page.add(&output_group);
    page.add(&audio_group);
    page.add(&actions_group);

    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Export").as_ref())
        .search_enabled(false)
        .build();
    dialog.add(&page);

    let format_row = format_row.clone();
    let container_row = container_row.clone();
    let fps_row = fps_row.clone();
    let background_alpha_row = background_alpha_row.clone();
    let rate_control_row = rate_control_row.clone();
    let bitrate_row = bitrate_row.clone();
    let max_bitrate_row = max_bitrate_row.clone();
    let target_quality_row = target_quality_row.clone();
    let default_constant_qp_row = default_constant_qp_row.clone();
    let keyframe_interval_row = keyframe_interval_row.clone();
    let preset_row = preset_row.clone();
    let tuning_row = tuning_row.clone();
    let multipass_row = multipass_row.clone();
    let profile_row = profile_row.clone();
    let look_ahead_row = look_ahead_row.clone();
    let adaptive_quantization_row = adaptive_quantization_row.clone();
    let b_frames_row = b_frames_row.clone();
    let b_frame_as_reference_row = b_frame_as_reference_row.clone();
    let audio_encoder_row = audio_encoder_row.clone();
    let audio_sample_rate_row = audio_sample_rate_row.clone();
    let audio_bitrate_row = audio_bitrate_row.clone();
    let dialog_for_close = dialog.clone();
    let parent_for_export = parent.clone();
    let toasts_for_export = toasts.clone();
    let project = project.clone();
    export_action.connect_activated(move |_| {
        let preferences = preferences::snapshot(&preferences);
        let settings = collect_export_video_settings(
            &format_row,
            &container_row,
            &fps_row,
            &background_alpha_row,
            &rate_control_row,
            &bitrate_row,
            &max_bitrate_row,
            &target_quality_row,
            &default_constant_qp_row,
            &keyframe_interval_row,
            &preset_row,
            &tuning_row,
            &multipass_row,
            &profile_row,
            &look_ahead_row,
            &adaptive_quantization_row,
            &b_frames_row,
            &b_frame_as_reference_row,
            &audio_encoder_row,
            &audio_sample_rate_row,
            &audio_bitrate_row,
            &preferences,
        );
        let project_snapshot = project.borrow().clone();
        let default_name = default_video_filename(&project_snapshot, settings.container);
        let label = "Export Video";
        let file_dialog = gtk::FileDialog::builder()
            .title(tr!(label).as_ref())
            .initial_name(&default_name)
            .build();
        let parent_for_save = parent_for_export.clone();
        let parent_for_error = parent_for_export.clone();
        let toasts = toasts_for_export.clone();
        shrimply_gtk_components::file_picker::save(
            label,
            &file_dialog,
            Some(parent_for_save.upcast_ref::<gtk::Window>()),
            move |result| {
                let Some(file) = result.ok() else {
                    return;
                };
                let Some(path) = file.path() else {
                    show_export_error(
                        &parent_for_error,
                        "Could not export video",
                        "Could not resolve the selected file path.",
                    );
                    return;
                };
                let extension = video::extension_for_container(settings.container);
                let path = output::ensure_extension(path, extension);
                let mut settings = settings.clone();
                settings.path = path;
                start_video_export(
                    &parent_for_error,
                    &toasts,
                    project_snapshot.clone(),
                    settings,
                );
            },
        );
        dialog_for_close.close();
    });

    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
}

#[allow(clippy::too_many_arguments)]
fn collect_export_video_settings(
    format_row: &TypedComboRow<video::ExportVideoCodec>,
    container_row: &TypedComboRow<video::ExportContainer>,
    fps_row: &TypedComboRow<Fraction>,
    background_alpha_row: &adw::SpinRow,
    rate_control_row: &TypedComboRow<video::ExportRateControl>,
    bitrate_row: &adw::SpinRow,
    max_bitrate_row: &adw::SpinRow,
    target_quality_row: &adw::SpinRow,
    constant_qp_row: &adw::SpinRow,
    keyframe_interval_row: &adw::SpinRow,
    preset_row: &TypedComboRow<video::ExportPreset>,
    tuning_row: &TypedComboRow<video::ExportTuning>,
    multipass_row: &TypedComboRow<video::ExportMultipass>,
    profile_row: &TypedComboRow<video::ExportProfile>,
    look_ahead_row: &adw::SwitchRow,
    adaptive_quantization_row: &adw::SwitchRow,
    b_frames_row: &adw::SpinRow,
    b_frame_as_reference_row: &adw::SwitchRow,
    audio_encoder_row: &TypedComboRow<video::ExportAudioEncoder>,
    audio_sample_rate_row: &TypedComboRow<u32>,
    audio_bitrate_row: &adw::SpinRow,
    preferences: &preferences::PreferencesSnapshot,
) -> video::ExportSettings {
    video::ExportSettings {
        path: std::path::PathBuf::new(),
        video_codec: format_row.selected(),
        container: match format_row.selected() {
            video::ExportVideoCodec::Gif => video::ExportContainer::Gif,
            _ => container_row.selected(),
        },
        fps: fps_row.selected(),
        background_alpha: if format_row.selected() == video::ExportVideoCodec::Gif {
            background_alpha_row.value() as u8
        } else {
            u8::MAX
        },
        rate_control: rate_control_row.selected(),
        constant_qp: constant_qp_row.value() as u32,
        bitrate_kbps: bitrate_row.value() as u32,
        max_bitrate_kbps: max_bitrate_row.value() as u32,
        target_quality: target_quality_row.value() as u32,
        keyframe_interval_seconds: keyframe_interval_row.value() as u32,
        preset: preset_row.selected(),
        tuning: tuning_row.selected(),
        multipass: multipass_row.selected(),
        profile: profile_row.selected(),
        look_ahead: look_ahead_row.is_active(),
        adaptive_quantization: adaptive_quantization_row.is_active(),
        b_frames: b_frames_row.value() as u32,
        b_frame_as_reference: b_frame_as_reference_row.is_active(),
        audio_encoder: audio_encoder_row.selected(),
        audio_sample_rate: audio_sample_rate_row.selected(),
        audio_bitrate_kbps: audio_bitrate_row.value() as u32,
        maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
        gpu_host_memory_gib: preferences.gpu_host_memory_gib,
    }
}

fn default_video_filename(project: &project::Project, container: video::ExportContainer) -> String {
    output::default_filename(project, video::extension_for_container(container))
}

enum VideoExportEvent {
    Progress(video::ExportProgress),
    Finished(Result<(), String>),
}

fn start_video_export(
    parent: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    project: project::Project,
    settings: video::ExportSettings,
) {
    let path = settings.path.clone();
    let (progress_dialog, state_label, progress_bar) = video_export_progress_dialog(parent);
    let (tx, rx) = std::sync::mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    progress_dialog.connect_closed({
        let cancelled = cancelled.clone();
        move |_| cancelled.store(true, Ordering::Relaxed)
    });
    let started = Instant::now();
    let worker_cancelled = cancelled.clone();
    thread::spawn(move || {
        let progress_tx = tx.clone();
        let result = video::export_project(project, settings, worker_cancelled, move |progress| {
            let _ = progress_tx.send(VideoExportEvent::Progress(progress));
        });
        let _ = tx.send(VideoExportEvent::Finished(result));
    });

    let parent = parent.clone();
    let toasts = toasts.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut finished = None;
        loop {
            match rx.try_recv() {
                Ok(VideoExportEvent::Progress(progress)) => {
                    update_video_export_progress(&state_label, &progress_bar, progress);
                }
                Ok(VideoExportEvent::Finished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err(
                        "The export worker stopped before reporting a result.".to_string()
                    ));
                    break;
                }
            }
        }

        match finished {
            Some(Ok(())) => {
                progress_dialog.close();
                let title = shrimply_gtk_components::i18n::text_args(
                    "Video exported in %{duration}",
                    &[("duration", time_format::human_duration(started.elapsed()))],
                );
                shrimply_gtk_components::export_feedback::show_export_finished_text(
                    &toasts, &parent, &title, &path,
                );
                glib::ControlFlow::Break
            }
            Some(Err(error)) => {
                let was_cancelled = cancelled.load(Ordering::Relaxed);
                progress_dialog.close();
                if !was_cancelled {
                    show_export_error(&parent, "Could not export video", &error);
                }
                glib::ControlFlow::Break
            }
            None => {
                if progress_bar.fraction() <= 0.0 {
                    progress_bar.pulse();
                }
                glib::ControlFlow::Continue
            }
        }
    });
}

fn video_export_progress_dialog(
    parent: &adw::ApplicationWindow,
) -> (adw::Dialog, gtk::Label, gtk::ProgressBar) {
    let dialog = adw::Dialog::builder()
        .title(tr!("Exporting Video").as_ref())
        .content_width(460)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let state = gtk::Label::new(Some(tr!("Preparing").as_ref()));
    state.set_halign(gtk::Align::Center);
    state.set_wrap(true);

    let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_text(Some(tr!("Preparing").as_ref()));
    progress.pulse();
    progress_box.append(&progress);

    content.append(&progress_box);
    content.append(&state);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
    (dialog, state, progress)
}

fn update_video_export_progress(
    state: &gtk::Label,
    progress: &gtk::ProgressBar,
    event: video::ExportProgress,
) {
    match event {
        video::ExportProgress::MixingAudio {
            current_frame,
            total_frames,
        } => {
            state.set_label(tr!("Preparing audio").as_ref());
            let fraction = if total_frames == 0 {
                1.0
            } else {
                current_frame as f64 / total_frames as f64
            }
            .clamp(0.0, 1.0);
            progress.set_fraction(fraction);
            progress.set_text(Some(&shrimply_gtk_components::i18n::text_args(
                "Preparing audio (%{percent}%)",
                &[("percent", format!("{:.0}", fraction * 100.0))],
            )));
        }
        video::ExportProgress::SettingUp(message) => {
            state.set_label(tr!(message).as_ref());
            progress.set_fraction(1.0);
            progress.set_text(Some(tr!(message).as_ref()));
        }
        video::ExportProgress::EncodingAudio {
            current_frame,
            total_frames,
        } => {
            state.set_label(tr!("Encoding audio").as_ref());
            let fraction = if total_frames == 0 {
                1.0
            } else {
                current_frame as f64 / total_frames as f64
            }
            .clamp(0.0, 1.0);
            progress.set_fraction(fraction);
            progress.set_text(Some(&shrimply_gtk_components::i18n::text_args(
                "Encoding audio (%{percent}%)",
                &[("percent", format!("{:.0}", fraction * 100.0))],
            )));
        }
        video::ExportProgress::EncodingVideo {
            current_frame,
            total_frames,
            fps_milli,
        } => {
            state.set_label(tr!("Rendering frames").as_ref());
            let fraction = if total_frames == 0 {
                1.0
            } else {
                current_frame as f64 / total_frames as f64
            }
            .clamp(0.0, 1.0);
            progress.set_fraction(fraction);
            let eta = (fps_milli > 0).then(|| {
                time_format::human_duration(crate::math::duration_for_frames_at_millifps(
                    total_frames.saturating_sub(current_frame),
                    fps_milli,
                ))
            });
            let progress_text = if fps_milli > 0 {
                shrimply_gtk_components::i18n::text_args(
                    "%{current} of %{total} frames (%{percent}%) - %{fps} fps - %{eta} left",
                    &[
                        ("current", current_frame.to_string()),
                        ("total", total_frames.to_string()),
                        ("percent", format!("{:.0}", fraction * 100.0)),
                        (
                            "fps",
                            format!("{}.{}", fps_milli / 1_000, fps_milli % 1_000 / 100),
                        ),
                        (
                            "eta",
                            eta.expect("positive frame rate produces an export ETA"),
                        ),
                    ],
                )
            } else {
                shrimply_gtk_components::i18n::text_args(
                    "%{current} of %{total} frames (%{percent}%)",
                    &[
                        ("current", current_frame.to_string()),
                        ("total", total_frames.to_string()),
                        ("percent", format!("{:.0}", fraction * 100.0)),
                    ],
                )
            };
            progress.set_text(Some(&progress_text));
        }
        video::ExportProgress::Finalizing => {
            state.set_label(tr!("Finishing file").as_ref());
            progress.set_fraction(1.0);
            progress.set_text(Some(tr!("Finishing").as_ref()));
        }
    }
}

fn open_project_json_dialog(
    parent: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    project: Rc<RefCell<project::Project>>,
) {
    let default_name = output::default_filename(&project.borrow(), "json");
    let label = "Export JSON";
    let dialog = gtk::FileDialog::builder()
        .title(tr!(label).as_ref())
        .initial_name(&default_name)
        .build();
    let parent_for_save = parent.clone();
    let parent_for_error = parent.clone();
    let toasts = toasts.clone();
    let project = project.clone();
    shrimply_gtk_components::file_picker::save(
        label,
        &dialog,
        Some(parent_for_save.upcast_ref::<gtk::Window>()),
        move |result| {
            let Some(file) = result.ok() else {
                return;
            };
            let path = match file.path() {
                Some(path) => path,
                None => {
                    show_export_error(
                        &parent_for_error,
                        "Could not export JSON",
                        "Could not resolve the selected file path.",
                    );
                    return;
                }
            };
            let path = output::ensure_extension(path, "json");

            let project = project.borrow().clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let result = json::export(&project, &path);
                let _ = sender.send((path, result));
            });

            let parent = parent_for_error.downgrade();
            let toasts = toasts.downgrade();
            glib::timeout_add_local(Duration::from_millis(16), move || {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        let Some(parent) = parent.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        show_export_error(
                            &parent,
                            "Could not export JSON",
                            "The JSON export worker stopped before reporting a result.",
                        );
                        return glib::ControlFlow::Break;
                    }
                };
                let Some(parent) = parent.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match result {
                    (path, Ok(())) => {
                        if let Some(toasts) = toasts.upgrade() {
                            show_export_finished(&toasts, &parent, "JSON exported", &path);
                        }
                    }
                    (_, Err(error)) => show_export_error(&parent, "Could not export JSON", &error),
                }
                glib::ControlFlow::Break
            });
        },
    );
}

fn show_export_error(parent: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", tr!("Close").as_ref());
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog.choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        |_| {},
    );
}

pub(crate) fn show_export_finished(
    toasts: &adw::ToastOverlay,
    parent: &adw::ApplicationWindow,
    title: &str,
    path: &std::path::Path,
) {
    shrimply_gtk_components::export_feedback::show_export_finished(toasts, parent, title, path);
}
