use super::Runtime;
use crate::bridge::export::write_document;
use serde_json::{Value, json};
use shrimply_caption_export_core::{self as captions, CaptionFormat, ExportMode};
use shrimply_export_core::{
    audio,
    job::{Event, Job, Outcome},
    output::default_filename,
    video::{self, settings},
};
use shrimply_project_document::Project;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Export {
    job: Job<Vec<String>>,
    pub state: Value,
}

enum Operation {
    Video(video::ExportSettings),
    Captions(captions::ExportSettings),
    Audio(audio::Format),
}

impl Operation {
    fn from_request(project: &Project, request: &Value) -> Result<Self, String> {
        match request["kind"].as_str() {
            Some("video") => Ok(Self::Video(
                settings::parse_configuration(
                    project,
                    PathBuf::new(),
                    request,
                    shrimply_export_core::android::settings_backend(),
                )?
                .common,
            )),
            Some("captions") => {
                let format = CaptionFormat::ALL
                    .into_iter()
                    .find(|format| request["format"] == format.extension())
                    .ok_or("Unknown caption export format")?;
                let mode = match request["mode"].as_str() {
                    Some("merge") => ExportMode::Merge,
                    Some("separate") => ExportMode::Separate,
                    _ => return Err("Unknown caption export track mode".into()),
                };
                Ok(Self::Captions(captions::ExportSettings { format, mode }))
            }
            _ => Err("Unknown export kind".into()),
        }
    }

    fn document_type(&self) -> (&'static str, &'static str, bool) {
        match self {
            Self::Video(settings) => (
                video::extension_for_container(settings.container),
                video::mime_type(settings.container),
                false,
            ),
            Self::Captions(settings) => (
                settings.format.extension(),
                settings.format.mime_type(),
                settings.mode == ExportMode::Separate,
            ),
            Self::Audio(format) => (format.extension(), format.mime_type(), false),
        }
    }

    fn filename(&self, project: &Project) -> Result<String, String> {
        let suggested = default_filename(project, self.document_type().0);
        Path::new(&suggested)
            .file_name()
            .map(|filename| filename.to_string_lossy().into_owned())
            .ok_or_else(|| "Invalid export filename".to_string())
    }

    fn run(
        self,
        project: Project,
        uri: &str,
        limit: usize,
        cancelled: Arc<AtomicBool>,
        progress: &mut dyn FnMut(video::ExportProgress),
    ) -> Result<Vec<String>, String> {
        let (_, mime, folder) = self.document_type();
        let filename = self.filename(&project)?;
        match self {
            Self::Video(mut settings) => {
                settings.path = filename.into();
                let name = write_document(uri, mime, None, |open| {
                    shrimply_export_core::android::export_project(
                        project, settings, limit, open, cancelled, progress,
                    )
                })?;
                Ok(vec![name])
            }
            Self::Captions(settings) => {
                progress(video::ExportProgress::SettingUp("Exporting Captions"));
                let outputs = captions::prepare_export(&project, Path::new(&filename), settings)?;
                let mut names = Vec::with_capacity(outputs.len());
                for output in outputs {
                    check_cancelled(&cancelled)?;
                    let name = output
                        .path
                        .file_name()
                        .expect("caption filename")
                        .to_string_lossy();
                    match write_document(uri, mime, folder.then_some(name.as_ref()), |open| {
                        check_cancelled(&cancelled)?;
                        shrimply_export_core::android::write_document(
                            open()?,
                            output.content.as_bytes(),
                            cancelled.clone(),
                        )
                        .map_err(|error| format!("Write captions: {error}"))?;
                        check_cancelled(&cancelled)
                    }) {
                        Ok(name) => names.push(name),
                        Err(error) if names.is_empty() => return Err(error),
                        Err(error) => {
                            return Err(format!(
                                "{error}\nFiles already exported: {}",
                                names.join("\n")
                            ));
                        }
                    }
                }
                Ok(names)
            }
            Self::Audio(format) => {
                let name = write_document(uri, mime, None, |open| {
                    audio::export_to_stream(
                        &project,
                        format,
                        || {
                            shrimply_export_core::android::open_document_stream(
                                open()?,
                                cancelled.clone(),
                            )
                        },
                        |event| {
                            progress(event);
                            !cancelled.load(Ordering::Acquire)
                        },
                    )
                })?;
                Ok(vec![name])
            }
        }
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Export cancelled".into())
    } else {
        Ok(())
    }
}

impl Export {
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for event in self.job.poll() {
            changed = true;
            match event {
                Event::Started => {}
                Event::Progress(progress) => {
                    self.state["progress"] = video::progress::localized(progress)
                }
                Event::Cancelling => self.state["cancelling"] = json!(true),
                Event::Finished(Outcome::Complete(names)) => {
                    self.state["files"] = json!(names);
                    self.state["status"] = json!("complete");
                }
                Event::Finished(Outcome::Cancelled) => self.state["status"] = json!("cancelled"),
                Event::Finished(Outcome::Failed(error)) => {
                    self.state["status"] = json!("failed");
                    self.state["error"] = json!(error);
                }
            }
        }
        if changed {
            self.state["presentation"] = job_view(&self.state);
        }
        changed
    }
}

impl Runtime {
    pub fn poll_export(&mut self) -> bool {
        self.export.as_mut().is_some_and(Export::poll)
    }

    pub(super) fn export_request(&mut self, request: &Value) -> Result<Value, String> {
        let operation = request["op"].as_str().expect("validated request operation");
        match operation {
            "export_schema" => {
                let project = self.backend.project().ok_or("open a project first")?;
                Ok(settings::schema_for(
                    &project,
                    shrimply_export_core::android::settings_backend(),
                    request.get("values"),
                ))
            }
            "export_destination" => {
                let project = self.backend.project().ok_or("Open a project first")?;
                let operation = Operation::from_request(&project, request)?;
                let (_, mime, folder) = operation.document_type();
                Ok(json!({"filename":operation.filename(&project)?,"mime":mime,"folder":folder}))
            }
            "caption_export_schema" => Ok(json!({
                "formats":CaptionFormat::ALL.map(|format| json!({"value":format.extension(),"label":format.label()})),
                "modes":[{"value":"merge","label":"Merge into one file"},{"value":"separate","label":"Export each track separately"}]
            })),
            "export_start" | "preview_image_save" => {
                if self
                    .export
                    .as_ref()
                    .is_some_and(|export| export.state["status"] == "running")
                {
                    return Err("An export is already running".into());
                }
                if operation == "export_start" {
                    self.start_export(request)
                } else {
                    self.save_preview_image(request)
                }
            }
            "export_cancel" | "export_dismiss" => {
                let export = self.export.as_mut().ok_or("No export is active")?;
                if request["id"] != export.state["id"] {
                    return Err("The export changed while this action was open".into());
                }
                export.poll();
                if operation == "export_cancel" {
                    if export.state["status"] == "running" {
                        export.job.cancel();
                        export.poll();
                    }
                } else {
                    if export.state["status"] == "running" {
                        return Err("Wait for the export to stop before dismissing it".into());
                    }
                    self.export = None;
                }
                self.backend.snapshot()
            }
            _ => unreachable!("export request dispatcher"),
        }
    }

    fn start_export(&mut self, request: &Value) -> Result<Value, String> {
        let uri = destination(request)?;
        let kind = request["kind"].as_str().ok_or("Missing export kind")?;
        let (project, operation) = if kind == "audio" {
            let (project, format) = self
                .timeline_service
                .as_ref()
                .ok_or("The timeline service is closed")?
                .audio_export(
                    shrimply_player_core::snapshot(&self.player).revision,
                    request,
                )?;
            (project, Operation::Audio(format))
        } else {
            self.backend.save()?;
            let project = self
                .backend
                .project()
                .ok_or("Open a project first")?
                .clone();
            let operation = Operation::from_request(&project, request)?;
            (project, operation)
        };
        let limit = shrimply_preferences_core::snapshot(&self.backend.preferences)
            .temporal_decoder_pool_size as usize;
        self.begin_export(kind, uri, move |uri, cancelled, progress| {
            operation.run(project, uri, limit, cancelled, progress)
        })?;
        if kind == "audio" {
            self.timeline_service = None;
        }
        Ok(json!({}))
    }

    pub(super) fn begin_export(
        &mut self,
        kind: &str,
        destination: String,
        operation: impl FnOnce(
            &str,
            Arc<AtomicBool>,
            &mut dyn FnMut(video::ExportProgress),
        ) -> Result<Vec<String>, String>
        + Send
        + 'static,
    ) -> Result<(), String> {
        let wake = self.backend.wake.clone();
        let uri = destination.clone();
        let job = Job::spawn(
            move |cancelled, progress| operation(&uri, cancelled, progress),
            move || wake(),
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut state = json!({"id":id,"status":"running","files":[],
            "kind":kind,"destination":destination,"progress":null,"error":null,"cancelling":false});
        state["presentation"] = job_view(&state);
        self.export = Some(Export { job, state });
        Ok(())
    }
}

fn job_view(state: &Value) -> Value {
    use shrimply_i18n::text;
    let running = state["status"] == "running";
    let complete = state["status"] == "complete";
    let cancelling = state["cancelling"] == true;
    let (running_title, complete_title, failed_title) = match state["kind"].as_str() {
        Some("captions") => (
            "Export Captions",
            "Captions exported",
            "Could not export captions",
        ),
        Some("audio") => (
            "Export Selected Audio",
            "Audio exported",
            "Could not export audio",
        ),
        Some("preview_image") => (
            "Save Preview Image",
            "Image saved",
            "Could not save preview image",
        ),
        Some("selected_frame") => (
            "Save Selected Frame",
            "Image saved",
            "Could not save selected frame",
        ),
        Some("video") => ("Export Video", "Video exported", "Could not export video"),
        _ => unreachable!("export kind"),
    };
    let title = match state["status"].as_str().expect("export status") {
        "running" => running_title,
        "complete" => complete_title,
        "cancelled" => "Export cancelled",
        "failed" => failed_title,
        _ => unreachable!("export job status"),
    };
    let dismiss = if running {
        "export_cancel"
    } else {
        "export_dismiss"
    };
    json!({"title":text(title),"running":running,"complete":complete,"failed":state["status"] == "failed",
        "dismiss":dismiss,"cancelling":if cancelling {Some(text("Cancelling…"))} else {None},
        "confirm":{"label":text(if running {"Cancel"} else {"Close"}),
            "action":dismiss,"enabled":!running || !cancelling}})
}

pub(super) fn destination(request: &Value) -> Result<String, String> {
    request["destination"]
        .as_str()
        .filter(|uri| uri.starts_with("content://"))
        .map(str::to_owned)
        .ok_or_else(|| "Choose an export destination first".into())
}
