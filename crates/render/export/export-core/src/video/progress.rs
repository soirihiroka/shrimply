//! Shared labels, exact-time ETA and display arguments for export progress.
use super::ExportProgress;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Presentation {
    pub label: &'static str,
    pub message: &'static str,
    pub arguments: BTreeMap<&'static str, String>,
    pub fraction: Option<f64>,
}

pub fn presentation(event: ExportProgress) -> Presentation {
    let mut arguments = BTreeMap::new();
    let (label, message, fraction) = match event {
        ExportProgress::SettingUp(message) => (message, message, None),
        ExportProgress::Finalizing => ("Finishing file", "Finishing", Some(1.0)),
        ExportProgress::MixingAudio {
            current_frame,
            total_frames,
        }
        | ExportProgress::EncodingAudio {
            current_frame,
            total_frames,
        } => {
            let fraction = shrimply_math_media::completed_fraction(current_frame, total_frames);
            arguments.insert("percent", format!("{:.0}", fraction * 100.0));
            let (label, message) = if matches!(event, ExportProgress::MixingAudio { .. }) {
                ("Preparing audio", "Preparing audio (%{percent}%)")
            } else {
                ("Encoding audio", "Encoding audio (%{percent}%)")
            };
            (label, message, Some(fraction))
        }
        ExportProgress::EncodingVideo {
            current_frame,
            total_frames,
            fps_milli,
        } => {
            let fraction = shrimply_math_media::completed_fraction(current_frame, total_frames);
            arguments.insert("current", current_frame.to_string());
            arguments.insert("total", total_frames.to_string());
            arguments.insert("percent", format!("{:.0}", fraction * 100.0));
            let message = if fps_milli > 0 {
                arguments.insert(
                    "fps",
                    format!("{}.{}", fps_milli / 1_000, fps_milli % 1_000 / 100),
                );
                arguments.insert(
                    "eta",
                    shrimply_project_document::time_format::human_duration(
                        shrimply_math_media::duration_for_frames_at_millifps(
                            total_frames.saturating_sub(current_frame),
                            fps_milli,
                        ),
                    ),
                );
                "%{current} of %{total} frames (%{percent}%) - %{fps} fps - %{eta} left"
            } else {
                "%{current} of %{total} frames (%{percent}%)"
            };
            ("Rendering frames", message, Some(fraction))
        }
    };
    Presentation {
        label,
        message,
        arguments,
        fraction,
    }
}
