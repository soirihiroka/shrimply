use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shrimply_resource_pipeline::fingerprint;
use uuid::Uuid;

use super::{Project, Time, atomic_file, normalized_project_path};

#[derive(Deserialize, Serialize)]
pub(super) struct View {
    cursor: Option<Time>,
    zoom: Option<Time>,
    expanded: Vec<Vec<Uuid>>,
}

impl View {
    pub(super) fn capture(project: &Project) -> Self {
        Self {
            cursor: project.cursor_position,
            zoom: project.timeline_zoom,
            expanded: project.expanded_sequence_paths.clone(),
        }
    }

    pub(super) fn apply(self, project: &mut Project) {
        project.cursor_position = self.cursor;
        project.timeline_zoom = self.zoom;
        project.expanded_sequence_paths = self.expanded;
    }

    pub(super) fn write(&self, project: &Path) -> Result<(), String> {
        let path = path_for(project);
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if fs::read(&path)
            .as_deref()
            .is_ok_and(|current| current == bytes)
        {
            return Ok(());
        }
        fs::create_dir_all(path.parent().expect("view path has a directory"))
            .and_then(|()| atomic_file::replace(&path, false, |file| file.write_all(&bytes)))
            .map_err(|error| error.to_string())
    }

    pub(super) fn read(project: &Path) -> Result<Option<Self>, String> {
        match fs::read(path_for(project)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn path_for(project: &Path) -> PathBuf {
    let key = fingerprint::digest(
        normalized_project_path(project)
            .as_os_str()
            .as_encoded_bytes(),
    );
    shrimply_path_core::config_directory()
        .join("editor-views")
        .join(format!("{key}.json"))
}
