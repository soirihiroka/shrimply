//! Native image services capture the completed shared GPU preview asynchronously.
use super::Runtime;
use crate::bridge::export::write_document;
use serde_json::{Value, json};
use shrimply_export_core::job::{Event, Job, Outcome};
use shrimply_preview_render_core::playback::toolbar::ContextRequest;
use std::sync::atomic::Ordering;

pub(super) struct PreviewImage {
    job: Option<Job<Option<std::path::PathBuf>>>,
    pending: Option<Capture>,
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
        let saving = action == "save";
        let (job, pending) = if saving {
            (None, Some(capture))
        } else {
            (Some(self.start_image_capture(capture, &id, None)?), None)
        };
        self.preview_image = Some(PreviewImage {
            job,
            pending,
            state: json!({"id":id,"status":if saving {"choose"} else {"running"},"action":action,"filename":filename,
                "mime":"image/png","label":shrimply_i18n::text(label),"saved_title":shrimply_i18n::text("Image saved"),
                "error_title":shrimply_i18n::text(error_title),"close_label":shrimply_i18n::text("Close")}),
        });
        Ok(())
    }

    fn start_image_capture(
        &self,
        capture: Capture,
        id: &str,
        destination: Option<String>,
    ) -> Result<Job<Option<std::path::PathBuf>>, String> {
        let path = if destination.is_none() {
            Some(
                shrimply_path_core::cache_directory()?
                    .join("exports/preview-images")
                    .join(format!("{id}.png")),
            )
        } else {
            None
        };
        let wake = self.wake.clone();
        Job::spawn(
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
                if let Some(destination) = destination {
                    write_document(&destination, "image/png", None, |open| {
                        shrimply_export_core::android::write_document(open()?, &png, cancelled)
                            .map_err(|error| error.to_string())
                    })?;
                } else if let Some(path) = &path {
                    std::fs::create_dir_all(path.parent().expect("preview image directory"))
                        .map_err(|error| error.to_string())?;
                    std::fs::write(path, png).map_err(|error| error.to_string())?;
                }
                Ok(path)
            },
            move || wake(),
        )
    }

    pub(super) fn save_preview_image(&mut self, request: &Value) -> Result<Value, String> {
        let uri = super::export::destination(request)?;
        let image = self
            .preview_image
            .as_mut()
            .filter(|image| image.state["id"] == request["id"])
            .ok_or("The preview image changed")?;
        let capture = image
            .pending
            .take()
            .ok_or("No preview image is waiting to be saved")?;
        let id = image.state["id"]
            .as_str()
            .expect("preview image ID")
            .to_owned();
        let job = self.start_image_capture(capture, &id, Some(uri))?;
        let image = self.preview_image.as_mut().expect("pending preview image");
        image.job = Some(job);
        image.state["status"] = json!("running");
        Ok(json!({}))
    }

    pub fn poll_preview_image(&mut self) -> bool {
        let Some(image) = &mut self.preview_image else {
            return false;
        };
        let Some(job) = &mut image.job else {
            return false;
        };
        let Some(result) = job.poll().find_map(|event| match event {
            Event::Finished(outcome) => Some(outcome),
            _ => None,
        }) else {
            return false;
        };
        match result {
            Outcome::Complete(Some(path)) => {
                image.state["path"] = json!(path);
                image.state["status"] = json!("ready");
            }
            Outcome::Complete(None) => image.state["status"] = json!("complete"),
            Outcome::Failed(error) => {
                image.state["status"] = json!("failed");
                image.state["error"] = json!(error);
            }
            Outcome::Cancelled => image.state["status"] = json!("cancelled"),
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
