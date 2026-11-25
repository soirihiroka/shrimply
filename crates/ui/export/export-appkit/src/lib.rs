#![cfg(target_os = "macos")]

mod audio;
pub use audio::choose_audio_format;
mod captions;
pub use captions::choose_caption_settings;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, sel};
use objc2_app_kit::{NSPopUpButton, NSTextField, NSView, NSWindow};
use objc2_foundation::{NSObject, NSObjectProtocol, NSSize};
use shrimply_components_appkit::export_dialog::{
    self, ROW_HEIGHT, WIDTH, add_choices, popup_row, text_row,
};
use shrimply_export_core::video::{
    ExportContainer, ExportVideoCodec, available_audio_encoders, settings::VideoEncoder,
};
use shrimply_export_metal::{
    ExportSettings, H264Entropy, VideoToolboxProfile, VideoToolboxRateControl,
};
use shrimply_project_document::project::{self, Project};
use std::{cell::OnceCell, path::PathBuf};

const HEIGHT: f64 = 555.0;

#[derive(Clone, Copy, Debug)]
enum Visibility {
    Always,
    Gif,
    Video,
    Hardware,
    HardwareH264,
}

struct DialogIvars {
    sheet: OnceCell<Retained<NSWindow>>,
    formats: &'static [VideoEncoder],
    profile: OnceCell<Retained<NSPopUpButton>>,
    rows: OnceCell<Vec<(Retained<NSView>, Visibility)>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DialogIvars]
    struct Dialog;

    unsafe impl NSObjectProtocol for Dialog {}

    impl Dialog {
        #[unsafe(method(formatChanged:))]
        fn format_changed(&self, sender: &NSPopUpButton) {
            let ivars = self.ivars();
            let format = ivars.formats[sender.indexOfSelectedItem() as usize].format();
            let codec = format.codec;
            let gif = codec == ExportVideoCodec::Gif;
            let hardware = format.hardware;
            let mut row_index = 0;
            for (row, visibility) in ivars.rows.get().expect("export rows installed").iter().rev() {
                let visible = match visibility {
                    Visibility::Always => true,
                    Visibility::Gif => gif,
                    Visibility::Video => !gif,
                    Visibility::Hardware => hardware,
                    Visibility::HardwareH264 => hardware && codec == ExportVideoCodec::H264,
                };
                row.setHidden(!visible);
                if visible {
                    let mut frame = row.frame();
                    frame.origin.y = 60.0 + row_index as f64 * ROW_HEIGHT;
                    row.setFrame(frame);
                    row_index += 1;
                }
            }
            ivars.sheet.get().expect("sheet installed")
                .setContentSize(NSSize::new(WIDTH, 75.0 + row_index as f64 * ROW_HEIGHT));
            if gif { return; }
            let profile = ivars.profile.get().expect("profile installed");
            profile.removeAllItems();
            if codec == ExportVideoCodec::H264 {
                add_choices(profile, &["Baseline", "Main", "High"]);
                profile.selectItemAtIndex(2);
            } else {
                add_choices(profile, &["Main", "Main 10"]);
                profile.selectItemAtIndex(0);
            }
        }
    }
);

pub fn choose_settings(
    parent: &NSWindow,
    project: &Project,
    maximum_temporal_decoders: usize,
) -> Option<ExportSettings> {
    let mtm = parent.mtm();
    let encoders = shrimply_export_metal::settings_backend();
    let dialog = Dialog::alloc(mtm).set_ivars(DialogIvars {
        sheet: OnceCell::new(),
        formats: encoders.formats(),
        profile: OnceCell::new(),
        rows: OnceCell::new(),
    });
    let dialog: Retained<Dialog> = unsafe { objc2::msg_send![super(dialog), init] };
    let sheet = export_dialog::new("Export Video", HEIGHT, mtm);
    dialog
        .ivars()
        .sheet
        .set(sheet.clone())
        .expect("sheet installed once");
    let content = sheet
        .contentView()
        .expect("export dialog content installed");

    let formats = &dialog.ivars().formats;
    let labels: Vec<_> = formats
        .iter()
        .map(|encoder| encoder.format().label)
        .collect();
    let default = encoders.default_encoder();
    let initial = formats
        .iter()
        .position(|&encoder| encoder == default)
        .expect("default export format");
    let (codec_row, codec) = popup_row("Format", &labels, 15, mtm);
    codec.selectItemAtIndex(initial as isize);
    unsafe {
        codec.setTarget(Some(&*dialog));
        codec.setAction(Some(sel!(formatChanged:)));
    }
    content.addSubview(&codec_row);
    let (container_row, container) = popup_row("Container", &["MP4", "MKV"], 14, mtm);
    content.addSubview(&container_row);

    let mut fps_values = project::COMMON_FRAME_RATES
        .iter()
        .map(|rate| rate.value)
        .collect::<Vec<_>>();
    let mut fps_labels = project::COMMON_FRAME_RATES
        .iter()
        .map(|rate| rate.label.to_string())
        .collect::<Vec<_>>();
    let selected_fps = fps_values
        .iter()
        .position(|fps| *fps == project.fps)
        .unwrap_or_else(|| {
            fps_values.push(project.fps);
            fps_labels.push(format!(
                "{} (Project)",
                project::fraction_as_label(project.fps)
            ));
            fps_values.len() - 1
        });
    let fps_label_refs = fps_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let (fps_row, fps) = popup_row("Frame rate", &fps_label_refs, 13, mtm);
    fps.selectItemAtIndex(selected_fps as isize);
    content.addSubview(&fps_row);
    let (rate_row, rate) = popup_row(
        "Rate control",
        &["Average bitrate", "Constant bitrate"],
        12,
        mtm,
    );
    content.addSubview(&rate_row);
    let (bitrate_row, bitrate) = text_row("Video bitrate (Kbps)", "10000", 11, mtm);
    content.addSubview(&bitrate_row);
    let (keyframe_row, keyframes) = text_row("Keyframe interval (s)", "2", 10, mtm);
    content.addSubview(&keyframe_row);
    let (profile_row, profile) = popup_row("Profile", &["Main", "Main 10"], 9, mtm);
    content.addSubview(&profile_row);
    dialog
        .ivars()
        .profile
        .set(profile.clone())
        .expect("profile installed once");
    let (b_frames_row, b_frames) = text_row("B frames", "0", 8, mtm);
    content.addSubview(&b_frames_row);
    let (entropy_row, entropy) = popup_row("H.264 entropy", &["CAVLC", "CABAC"], 7, mtm);
    entropy.selectItemAtIndex(1);
    entropy_row.setHidden(true);
    content.addSubview(&entropy_row);
    let (speed_row, prioritize_speed) =
        popup_row("Prioritize speed", &["Disabled", "Enabled"], 6, mtm);
    content.addSubview(&speed_row);
    let (power_row, power_efficient) =
        popup_row("Power efficiency", &["Disabled", "Enabled"], 5, mtm);
    content.addSubview(&power_row);
    let (aq_row, spatial_aq) = popup_row("Spatial AQ", &["Disabled", "Enabled"], 4, mtm);
    content.addSubview(&aq_row);
    let (refs_row, max_refs) = text_row("Max refs (0 = auto)", "0", 3, mtm);
    content.addSubview(&refs_row);
    let (encoders, labels): (Vec<_>, Vec<_>) = available_audio_encoders().unzip();
    let (audio_row, audio) = popup_row("Audio encoder", &labels, 2, mtm);
    content.addSubview(&audio_row);
    let (audio_rate_row, audio_rate) =
        popup_row("Audio sample rate", &["44100", "48000", "96000"], 1, mtm);
    audio_rate.selectItemAtIndex(1);
    content.addSubview(&audio_rate_row);
    let (audio_bitrate_row, audio_bitrate) = text_row("Audio bitrate (Kbps)", "192", 0, mtm);
    content.addSubview(&audio_bitrate_row);
    let (alpha_row, alpha) = text_row("Background alpha (0–255)", "0", 0, mtm);
    alpha_row.setHidden(true);
    content.addSubview(&alpha_row);
    use Visibility::*;
    dialog
        .ivars()
        .rows
        .set(vec![
            (codec_row, Always),
            (container_row, Video),
            (fps_row, Always),
            (rate_row, Video),
            (bitrate_row, Video),
            (keyframe_row, Video),
            (profile_row, Hardware),
            (b_frames_row, Video),
            (entropy_row, HardwareH264),
            (speed_row, Hardware),
            (power_row, Hardware),
            (aq_row, Hardware),
            (refs_row, Hardware),
            (audio_row, Video),
            (audio_rate_row, Video),
            (audio_bitrate_row, Video),
            (alpha_row, Gif),
        ])
        .expect("export rows installed once");
    dialog.format_changed(sel!(formatChanged:), &codec);

    if !export_dialog::run(parent, &sheet) {
        return None;
    }

    let encoder = *formats.get(codec.indexOfSelectedItem() as usize)?;
    let format = encoder.format();
    let video_codec = format.codec;
    let container = if video_codec == ExportVideoCodec::Gif {
        ExportContainer::Gif
    } else if container.indexOfSelectedItem() == 1 {
        ExportContainer::Mkv
    } else {
        ExportContainer::Mp4
    };
    let profile = if video_codec == ExportVideoCodec::H264 {
        match profile.indexOfSelectedItem() {
            0 => VideoToolboxProfile::H264Baseline,
            1 => VideoToolboxProfile::H264Main,
            _ => VideoToolboxProfile::H264High,
        }
    } else {
        match profile.indexOfSelectedItem() {
            1 => VideoToolboxProfile::HevcMain10,
            _ => VideoToolboxProfile::HevcMain,
        }
    };
    Some(ExportSettings {
        maximum_temporal_decoders,
        path: PathBuf::new(),
        overwrite: true,
        video_encoder: encoder,
        container,
        fps: *fps_values.get(fps.indexOfSelectedItem() as usize)?,
        background_alpha: if video_codec == ExportVideoCodec::Gif {
            parse_u8(&alpha)?
        } else {
            u8::MAX
        },
        rate_control: if rate.indexOfSelectedItem() == 1 {
            VideoToolboxRateControl::ConstantBitrate
        } else {
            VideoToolboxRateControl::AverageBitrate
        },
        profile,
        bitrate_kbps: parse_u32(&bitrate)?,
        keyframe_interval_seconds: parse_u32(&keyframes)?,
        b_frames: parse_u32(&b_frames)?,
        h264_entropy: if entropy.indexOfSelectedItem() == 0 {
            H264Entropy::Cavlc
        } else {
            H264Entropy::Cabac
        },
        prioritize_speed: prioritize_speed.indexOfSelectedItem() == 1,
        power_efficient: power_efficient.indexOfSelectedItem() == 1,
        spatial_aq: spatial_aq.indexOfSelectedItem() == 1,
        max_reference_frames: if format.hardware {
            parse_u32(&max_refs)?
        } else {
            0
        },
        audio_encoder: *encoders.get(audio.indexOfSelectedItem() as usize)?,
        audio_sample_rate: match audio_rate.indexOfSelectedItem() {
            0 => 44_100,
            2 => 96_000,
            _ => 48_000,
        },
        audio_bitrate_kbps: parse_u32(&audio_bitrate)?,
    })
}

fn parse_u32(field: &NSTextField) -> Option<u32> {
    field.stringValue().to_string().parse().ok()
}
fn parse_u8(field: &NSTextField) -> Option<u8> {
    field.stringValue().to_string().parse().ok()
}
