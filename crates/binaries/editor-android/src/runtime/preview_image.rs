//! Native image services capture the completed shared GPU preview asynchronously.
use super::Runtime;
use serde_json::{Value, json};
use shrimply_export_core::job::Job;
use shrimply_preview_render_core::playback::toolbar::ContextRequest;
use std::sync::atomic::Ordering;

pub(super) struct PreviewImage {
    job: Option<Job<std::path::PathBuf>>,
    state: Value,
}

enum Capture {
    Preview(shrimply_preview_render_vulkan::worker::Frame),
    Selected {
        project: Box<shrimply_project_document::Project>,
        position: shrimply_math_core::Time,
        items: Vec<uuid::Uuid>,
        decoder_limit: usize,
    },
}

impl Runtime {
    pub(super) fn capture_preview_image(&mut self, request: ContextRequest) -> Result<(), String> {
        let frame = self
            .preview_frame
            .clone()
            .ok_or("The preview has no rendered image")?;
        self.capture_image(request, Capture::Preview(frame))
    }

    pub(super) fn capture_selected_frame(
        &mut self,
        selection: shrimply_timeline_skia::VideoFrameSelection,
        request: ContextRequest,
    ) -> Result<(), String> {
        let (project, position, items) = self
            .timeline
            .as_ref()
            .ok_or("Open a project first")?
            .selected_video_frame(selection);
        if items.is_empty() {
            return Err("No video is selected.".into());
        }
        let decoder_limit = shrimply_preferences_core::snapshot(&self.backend.preferences)
            .temporal_decoder_pool_size as usize;
        self.capture_image(
            request,
            Capture::Selected {
                project: Box::new(project),
                position,
                items,
                decoder_limit,
            },
        )
    }

    fn capture_image(&mut self, request: ContextRequest, capture: Capture) -> Result<(), String> {
        if self.preview_image.is_some() {
            return Err("An image capture is already pending".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = shrimply_path_core::cache_directory()?
            .join("exports/preview-images")
            .join(format!("{id}.png"));
        let (action, label, error_title) = match (&capture, request) {
            (Capture::Preview(_), ContextRequest::CopyImage) => {
                ("copy", "Copy Preview Image", "Could not copy preview image")
            }
            (Capture::Preview(_), ContextRequest::SaveImage) => {
                ("save", "Save Preview Image", "Could not save preview image")
            }
            (Capture::Selected { .. }, ContextRequest::CopyImage) => (
                "copy",
                "Copy Selected Frame",
                "Could not copy selected frame",
            ),
            (Capture::Selected { .. }, ContextRequest::SaveImage) => (
                "save",
                "Save Selected Frame",
                "Could not save selected frame",
            ),
        };
        let filename = if matches!(capture, Capture::Preview(_)) {
            "preview.png"
        } else {
            "frame.png"
        };
        let wake = self.wake.clone();
        let job = Job::spawn(
            move |cancelled, _| {
                let png = match capture {
                    Capture::Preview(frame) => frame.png()?,
                    Capture::Selected {
                        project,
                        position,
                        items,
                        decoder_limit,
                    } => {
                        let mut compositor = shrimply_preview_render_vulkan::Compositor::new()?;
                        compositor.set_decoder_limit(decoder_limit);
                        compositor
                            .render_rgba(
                                &project,
                                position,
                                Some(shrimply_preview_render_core::CaptureTarget::Items(items)),
                                &cancelled,
                            )?
                            .png()?
                    }
                };
                if cancelled.load(Ordering::Acquire) {
                    return Err("Preview image capture cancelled".into());
                }
                std::fs::create_dir_all(path.parent().expect("preview image directory"))
                    .map_err(|error| error.to_string())?;
                std::fs::write(&path, png).map_err(|error| error.to_string())?;
                if cancelled.load(Ordering::Acquire) {
                    std::fs::remove_file(&path).map_err(|error| error.to_string())?;
                    return Err("Preview image capture cancelled".into());
                }
                Ok(path)
            },
            move || wake(),
        )?;
        self.preview_image = Some(PreviewImage {
            job: Some(job),
            state: json!({"id":id,"status":"running","action":action,"filename":filename,
                "mime":"image/png","label":shrimply_i18n::text(label),
                "error_title":shrimply_i18n::text(error_title),"close_label":shrimply_i18n::text("Close")}),
        });
        Ok(())
    }

    pub fn poll_preview_image(&mut self) -> bool {
        let Some(image) = &mut self.preview_image else {
            return false;
        };
        let Some(job) = &mut image.job else {
            return false;
        };
        let Some(result) = job.poll().finished else {
            return false;
        };
        match result {
            Ok(path) => {
                image.state["path"] = json!(path);
                image.state["status"] = json!("ready");
            }
            Err(error) => {
                image.state["status"] = json!("failed");
                image.state["error"] = json!(error);
            }
        }
        image.job = None;
        true
    }

    pub(super) fn dismiss_preview_image(&mut self, request: &Value) -> Result<Value, String> {
        if self
            .preview_image
            .as_ref()
            .is_some_and(|image| image.state["id"] == request["id"] && image.job.is_none())
        {
            let image = self
                .preview_image
                .as_ref()
                .expect("completed preview image");
            // Clipboard URIs remain readable after dismissal. Save and picker
            // cancellation have no remaining consumer of the staging file.
            if image.state["action"] == "save"
                && let Some(path) = image.state["path"].as_str()
            {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            self.preview_image = None;
        }
        Ok(json!({}))
    }

    pub(super) fn preview_image_snapshot(&self) -> Value {
        self.preview_image
            .as_ref()
            .map_or(Value::Null, |image| image.state.clone())
    }

    pub(super) fn preview_image_available(&self) -> bool {
        self.preview_frame.is_some() && self.preview_image.is_none()
    }
}
