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
use shrimply_export_core::video::{ExportAudioEncoder, ExportContainer, ExportVideoCodec};
use shrimply_export_metal::{
    ExportSettings, H264Entropy, VideoToolboxProfile, VideoToolboxRateControl,
};
use shrimply_project_document::project::{self, Project};
use std::{cell::OnceCell, path::PathBuf};

const HEIGHT: f64 = 555.0;
const GIF_HEIGHT: f64 = 165.0;

struct DialogIvars {
    sheet: OnceCell<Retained<NSWindow>>,
    format_row: OnceCell<Retained<NSView>>,
    fps_row: OnceCell<Retained<NSView>>,
    profile: OnceCell<Retained<NSPopUpButton>>,
    video_rows: OnceCell<Vec<Retained<NSView>>>,
    rows_below_entropy: OnceCell<Vec<Retained<NSView>>>,
    alpha_row: OnceCell<Retained<NSView>>,
    entropy_row: OnceCell<Retained<NSView>>,
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
            let codec = sender.indexOfSelectedItem();
            let gif = codec == 2;
            for row in self.ivars().video_rows.get().expect("video rows installed") {
                row.setHidden(gif);
            }
            self.ivars().alpha_row.get().expect("alpha row installed").setHidden(!gif);
            let mut format_frame = self.ivars().format_row.get().expect("format row installed").frame();
            format_frame.origin.y = if gif { 120.0 } else { 510.0 };
            self.ivars().format_row.get().expect("format row installed").setFrame(format_frame);
            let mut fps_frame = self.ivars().fps_row.get().expect("FPS row installed").frame();
            fps_frame.origin.y = if gif { 90.0 } else { 450.0 };
            self.ivars().fps_row.get().expect("FPS row installed").setFrame(fps_frame);
            self.ivars()
                .sheet
                .get()
                .expect("sheet installed")
                .setContentSize(NSSize::new(WIDTH, if gif { GIF_HEIGHT } else { HEIGHT }));
            let entropy_hidden = gif || codec != 0;
            self.ivars().entropy_row.get().expect("entropy row installed").setHidden(entropy_hidden);
            for (index, row) in self
                .ivars()
                .rows_below_entropy
                .get()
                .expect("lower rows installed")
                .iter()
                .enumerate()
            {
                let mut frame = row.frame();
                frame.origin.y = 60.0
                    + (6 - index + usize::from(entropy_hidden)) as f64 * ROW_HEIGHT;
                row.setFrame(frame);
            }
            if gif { return; }
            let profile = self.ivars().profile.get().expect("profile installed");
            profile.removeAllItems();
            if codec == 0 {
                add_choices(profile, &["Baseline", "Main", "High"]);
                profile.selectItemAtIndex(2);
            } else {
                add_choices(profile, &["Main", "Main 10"]);
                profile.selectItemAtIndex(0);
            }
        }
    }
);

pub fn choose_settings(parent: &NSWindow, project: &Project, maximum_temporal_decoders: usize) -> Option<ExportSettings> {
    let mtm = parent.mtm();
    let dialog = Dialog::alloc(mtm).set_ivars(DialogIvars {
        sheet: OnceCell::new(),
        format_row: OnceCell::new(),
        fps_row: OnceCell::new(),
        profile: OnceCell::new(),
        video_rows: OnceCell::new(),
        rows_below_entropy: OnceCell::new(),
        alpha_row: OnceCell::new(),
        entropy_row: OnceCell::new(),
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

    let (codec_row, codec) = popup_row("Format", &["H.264", "H.265 / HEVC", "GIF"], 15, mtm);
    codec.selectItemAtIndex(1);
    unsafe {
        codec.setTarget(Some(&*dialog));
        codec.setAction(Some(sel!(formatChanged:)));
    }
    content.addSubview(&codec_row);
    dialog
        .ivars()
        .format_row
        .set(codec_row.clone())
        .expect("format row installed once");
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
    dialog
        .ivars()
        .fps_row
        .set(fps_row.clone())
        .expect("FPS row installed once");

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
    dialog
        .ivars()
        .entropy_row
        .set(entropy_row.clone())
        .expect("entropy row installed once");
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
    let (audio_row, audio) = popup_row(
        "Audio encoder",
        &["AudioToolbox AAC", "AAC", "Opus"],
        2,
        mtm,
    );
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
    dialog
        .ivars()
        .alpha_row
        .set(alpha_row)
        .expect("alpha row installed once");
    dialog
        .ivars()
        .video_rows
        .set(vec![
            container_row,
            rate_row,
            bitrate_row,
            keyframe_row,
            profile_row,
            b_frames_row,
            entropy_row,
            speed_row.clone(),
            power_row.clone(),
            aq_row.clone(),
            refs_row.clone(),
            audio_row.clone(),
            audio_rate_row.clone(),
            audio_bitrate_row.clone(),
        ])
        .expect("video rows installed once");
    dialog
        .ivars()
        .rows_below_entropy
        .set(vec![
            speed_row,
            power_row,
            aq_row,
            refs_row,
            audio_row,
            audio_rate_row,
            audio_bitrate_row,
        ])
        .expect("lower rows installed once");
    dialog.format_changed(sel!(formatChanged:), &codec);

    if !export_dialog::run(parent, &sheet) {
        return None;
    }

    let video_codec = match codec.indexOfSelectedItem() {
        0 => ExportVideoCodec::H264,
        1 => ExportVideoCodec::H265,
        _ => ExportVideoCodec::Gif,
    };
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
        video_codec,
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
        max_reference_frames: parse_u32(&max_refs)?,
        audio_encoder: match audio.indexOfSelectedItem() {
            1 => ExportAudioEncoder::Aac,
            2 => ExportAudioEncoder::Opus,
            _ => ExportAudioEncoder::AudioToolboxAac,
        },
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
