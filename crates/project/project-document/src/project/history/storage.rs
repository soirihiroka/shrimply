use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{Project, ProjectEncoding, atomic_file};

const SNAPSHOT_LIMIT: usize = 20;

pub fn create_project_file(path: &Path, project: &Project) -> Result<(), String> {
    require_shrimp(path)?;
    write_project(path, project).map(|_| ())
}

pub fn create_new_project_file(path: &Path, project: &Project) -> Result<(), String> {
    require_shrimp(path)?;
    let bytes = ProjectEncoding::Shrimp.encode(&project_for_storage(path, project)?)?;
    atomic_file::create(path, &bytes)
        .map_err(|error| format!("could not create {}: {error}", path.display()))
}

fn require_shrimp(path: &Path) -> Result<(), String> {
    if ProjectEncoding::from_path(path) == Some(ProjectEncoding::Shrimp) {
        Ok(())
    } else {
        Err("new projects must use the .shrimp extension".into())
    }
}

pub fn serialize_project_json(path: &Path, project: &Project) -> Result<Vec<u8>, String> {
    ProjectEncoding::Json.encode(&project_for_storage(path, project)?)
}

pub(super) fn read_project(path: &Path) -> Result<(Project, String), String> {
    let format = ProjectEncoding::from_path(path).ok_or("unsupported project file extension")?;
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let project = format
        .decode(&bytes)
        .map_err(|error| format!("could not load {}: {error}", path.display()))?;
    Ok((
        project,
        shrimply_resource_pipeline::fingerprint::digest(&bytes),
    ))
}

pub(super) fn write_project(path: &Path, project: &Project) -> Result<String, String> {
    let bytes = encode_project(path, project)?;
    write_checkpoint(path, &bytes)
}

pub(super) fn encode_project(path: &Path, project: &Project) -> Result<Vec<u8>, String> {
    let format = ProjectEncoding::from_path(path)
        .ok_or("projects can only be saved as .shrimp, .sjson, or legacy .json files")?;
    format.encode(&project_for_storage(path, project)?)
}

pub(super) fn write_checkpoint(path: &Path, bytes: &[u8]) -> Result<String, String> {
    atomic_file::replace(path, true, |file| file.write_all(bytes))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(shrimply_resource_pipeline::fingerprint::digest(bytes))
}

pub(super) fn write_snapshot(path: &Path, project: &Project) -> Result<PathBuf, String> {
    let directory = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".snapshots");
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "could not create snapshot directory {}: {error}",
            directory.display()
        )
    })?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("project");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("could not timestamp project snapshot: {error}"))?
        .as_millis();
    let snapshot = directory.join(format!("{stem}-{timestamp}.shrimp"));
    let bytes = ProjectEncoding::Shrimp.encode(&project_for_storage(path, project)?)?;
    atomic_file::replace(&snapshot, false, |file| file.write_all(&bytes))
        .map_err(|error| format!("could not write {}: {error}", snapshot.display()))?;
    prune_snapshots(&directory, stem)?;
    Ok(snapshot)
}

fn project_for_storage(path: &Path, project: &Project) -> Result<Project, String> {
    let mut project = project.clone();
    project.ensure_ids();
    project.validate()?;
    let directory = std::path::absolute(path.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|error| format!("could not resolve {}: {error}", path.display()))?;
    project.make_asset_paths_portable(&directory);
    Ok(project)
}

fn prune_snapshots(directory: &Path, stem: &str) -> Result<(), String> {
    let prefix = format!("{stem}-");
    let mut snapshots = fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".shrimp"))
        })
        .collect::<Vec<_>>();
    snapshots.sort();
    let remove_count = snapshots.len().saturating_sub(SNAPSHOT_LIMIT);
    for snapshot in snapshots.into_iter().take(remove_count) {
        fs::remove_file(&snapshot)
            .map_err(|error| format!("could not remove {}: {error}", snapshot.display()))?;
    }
    Ok(())
}
