//! Android transfers document descriptors; the shared Scene owns drop semantics.
use super::Runtime;
use serde_json::{Value, json};
use shrimply_media_import::OwnedFile;
use shrimply_timeline_skia::import_queue::BatchId;
use std::{fs, path::Path};

pub(super) struct Session {
    id: String,
    destination: Option<shrimply_timeline_skia::import_queue::Destination>,
    files: Vec<OwnedFile>,
    remux: Option<BatchId>,
}

impl Runtime {
    pub(super) fn file_drop_request(
        &mut self,
        request: &Value,
        source: Option<fs::File>,
    ) -> Result<Value, String> {
        let operation = request["op"].as_str().expect("validated drop operation");
        if operation == "file_drop_begin" {
            if self.file_drop.is_some() {
                return Err("Finish the current file drop first".into());
            }
            let scene = self.timeline.as_ref().ok_or("Open a project first")?;
            let point = request
                .get("point")
                .map(|point| {
                    let x = point["x"]
                        .as_f64()
                        .ok_or("drop requires a valid x position")?;
                    let y = point["y"]
                        .as_f64()
                        .ok_or("drop requires a valid y position")?;
                    let point = glam::vec2(x as f32, y as f32);
                    if !point.is_finite() {
                        return Err(
                            "files cannot be inserted at this timeline position".to_string()
                        );
                    }
                    Ok(point)
                })
                .transpose()?;
            let destination = scene.external_files_destination(point)?;
            let id = uuid::Uuid::new_v4().to_string();
            self.file_drop = Some(Session {
                id: id.clone(),
                destination: Some(destination),
                files: Vec::new(),
                remux: None,
            });
            return Ok(json!({"id": id}));
        }
        let session = self.file_drop.as_ref().ok_or("The file drop is closed")?;
        if request["id"].as_str() != Some(session.id.as_str()) {
            return Err("The file drop changed; drop the files again".into());
        }
        match operation {
            "file_drop_file" => {
                if session.remux.is_some() {
                    return Err("The file drop is already being imported".into());
                }
                let name = request["name"]
                    .as_str()
                    .ok_or("dropped file requires a name")?;
                shrimply_media_import::file_kind(Path::new(name)).ok_or("unsupported file type")?;
                let path = request["path"]
                    .as_str()
                    .ok_or("dropped file requires a path")?;
                let directory = self
                    .backend
                    .path()
                    .ok_or("Open a project first")?
                    .parent()
                    .expect("project directory")
                    .join("media/dropped")
                    .join(&session.id)
                    .join(session.files.len().to_string());
                let staged = shrimply_media_import::stage_source_in(
                    Path::new(path),
                    source,
                    name,
                    &directory,
                )?;
                assert!(staged.created, "drop media must have a unique destination");
                self.file_drop
                    .as_mut()
                    .expect("validated drop session")
                    .files
                    .push(OwnedFile::new(staged.path));
                Ok(json!({}))
            }
            "file_drop_commit" => {
                let mut session = self.file_drop.take().expect("validated drop session");
                if session.remux.is_some() {
                    self.file_drop = Some(session);
                    return Err("The file drop is already being imported".into());
                }
                let started = self
                    .timeline
                    .as_mut()
                    .ok_or("Open a project first")?
                    .enqueue_external_owned_files(
                        std::mem::take(&mut session.files),
                        session
                            .destination
                            .take()
                            .expect("staging drop destination"),
                    )?;
                if started.needs_remux {
                    session.remux = Some(started.batch);
                    let prompt = shrimply_timeline_skia::external_content::remux_prompt();
                    let result = json!({"confirmation":{"title":prompt.title,"body":prompt.body,
                        "confirm_label":prompt.confirm,"cancel_label":prompt.cancel}});
                    self.file_drop = Some(session);
                    Ok(result)
                } else {
                    Ok(json!({}))
                }
            }
            "file_drop_cancel" | "file_drop_remux" => {
                let accepted = operation == "file_drop_remux"
                    && request["accepted"]
                        .as_bool()
                        .ok_or("remux requires a boolean")?;
                let session = self.file_drop.take().expect("validated drop session");
                if let Some(batch) = session.remux {
                    self.timeline
                        .as_mut()
                        .ok_or("Open a project first")?
                        .confirm_import_remux(batch, accepted)?;
                }
                Ok(json!({}))
            }
            _ => unreachable!("validated drop operation"),
        }
    }
}
