//! Shared labels, exact-time ETA and display arguments for export progress.
use super::ExportProgress;
use serde::Serialize;
use std::collections::BTreeMap;

pub fn localized(event: ExportProgress) -> serde_json::Value {
    let value = presentation(event);
    let arguments: Vec<_> = value.arguments.into_iter().collect();
    serde_json::json!({"label":shrimply_i18n::text(value.label),
        "message":shrimply_i18n::text_args(value.message,&arguments),"fraction":value.fraction})
}

/// Native hosts render this job state without inventing labels or cancel policy.
pub fn job_view(state: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    use shrimply_i18n::text;
    let captions = state["kind"] == "captions";
    let running = state["status"] == "running";
    let complete = state["status"] == "complete";
    let cancelling = state["cancelling"] == true;
    let title = match state["status"].as_str().expect("export status") {
        "running" => {
            if captions {
                "Export Captions"
            } else {
                "Export Video"
            }
        }
        "complete" => {
            if captions {
                "Captions exported"
            } else {
                "Video exported"
            }
        }
        "cancelled" => "Export cancelled",
        "failed" => {
            if captions {
                "Could not export captions"
            } else {
                "Could not export video"
            }
        }
        _ => unreachable!("export job status"),
    };
    let dismiss = if running {
        "export_cancel"
    } else {
        "export_dismiss"
    };
    json!({"title":text(title),"running":running,"complete":complete,"failed":state["status"] == "failed",
        "dismiss":dismiss,"cancelling":if cancelling {Some(text("Cancelling…"))} else {None},
        "confirm":{"label":text(if complete {"Save"} else if running {"Cancel"} else {"Close"}),
            "action":if complete {"save"} else {dismiss},"enabled":!running || !cancelling},
        "secondary":if complete {Some(json!({"label":text("Close"),"action":"export_dismiss"}))} else {None}})
}

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
