use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project::Project;

static LATEST_SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
const SNAPSHOT_LIMIT: usize = 20;
const SHRIMP_MAGIC: &[u8; 8] = b"SHRIMP\0\x01";
const SHRIMP_VERSION: u32 = 7;

pub fn create_project_file(path: &Path, project: &Project) -> Result<(), String> {
    if !has_extension(path, "shrimp") {
        return Err("new projects must use the .shrimp extension".to_string());
    }
    write_project(path, project).map(|_| ())
}

pub fn create_new_project_file(path: &Path, project: &Project) -> Result<(), String> {
    if !has_extension(path, "shrimp") {
        return Err("new projects must use the .shrimp extension".to_string());
    }
    let project = project_for_storage(path, project)?;
    let bytes = shrimp_bytes(
        &rmp_serde::to_vec_named(&project)
            .map_err(|error| format!("could not encode MessagePack project: {error}"))?,
    );
    atomic_create(path, &bytes)
}

pub fn serialize_project_json(path: &Path, project: &Project) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&project_for_storage(path, project)?)
        .map_err(|error| format!("could not serialize project JSON: {error}"))
}

pub(super) fn read_project(path: &Path) -> Result<(Project, String), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let project = if has_extension(path, "sjson") || has_extension(path, "json") {
        super::super::conversion::from_json(
            std::str::from_utf8(&bytes)
                .map_err(|error| format!("could not read {} as UTF-8: {error}", path.display()))?,
        )
    } else {
        super::super::conversion::from_messagepack(shrimp_payload(path, &bytes)?)
    }
    .map_err(|error| format!("could not load {}: {error}", path.display()))?;
    Ok((
        project,
        shrimply_resource_pipeline::fingerprint::digest(&bytes),
    ))
}

pub(super) fn write_project(path: &Path, project: &Project) -> Result<String, String> {
    let bytes = if has_extension(path, "shrimp") {
        let project = project_for_storage(path, project)?;
        shrimp_bytes(
            &rmp_serde::to_vec_named(&project)
                .map_err(|error| format!("could not encode MessagePack project: {error}"))?,
        )
    } else if has_extension(path, "sjson") || has_extension(path, "json") {
        serialize_project_json(path, project)?
    } else {
        return Err(
            "projects can only be saved as .shrimp, .sjson, or legacy .json files".to_string(),
        );
    };
    atomic_write(path, &bytes)?;
    Ok(shrimply_resource_pipeline::fingerprint::digest(&bytes))
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
    let project = project_for_storage(path, project)?;
    let bytes = shrimp_bytes(
        &rmp_serde::to_vec_named(&project)
            .map_err(|error| format!("could not encode MessagePack project snapshot: {error}"))?,
    );
    atomic_write(&snapshot, &bytes)?;
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let generation = LATEST_SAVE_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let tmp_path = path.with_file_name(format!(
        "{}.tmp-{generation}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project.shrimp")
    ));
    fs::write(&tmp_path, bytes)
        .and_then(|_| fs::rename(&tmp_path, path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp_path);
        })
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let generation = LATEST_SAVE_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let tmp_path = path.with_file_name(format!(
        "{}.tmp-{}-{generation}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project.shrimp"),
        std::process::id()
    ));
    let result = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp_path)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|_| publish_new_file(&tmp_path, path));
    if let Err(error) = result {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("could not create {}: {error}", path.display()));
    }
    let _ = fs::remove_file(tmp_path);
    Ok(())
}

fn publish_new_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "android")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        // Android app domains cannot create hard links. The kernel's no-replace
        // rename publishes complete bytes atomically and preserves existing files.
        let temporary = CString::new(temporary.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                temporary.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE as libc::c_uint,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "android"))]
    fs::hard_link(temporary, destination)
}

fn shrimp_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes =
        Vec::with_capacity(SHRIMP_MAGIC.len() + std::mem::size_of::<u32>() + payload.len());
    bytes.extend_from_slice(SHRIMP_MAGIC);
    bytes.extend_from_slice(&SHRIMP_VERSION.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn shrimp_payload<'a>(path: &Path, bytes: &'a [u8]) -> Result<&'a [u8], String> {
    let header_len = SHRIMP_MAGIC.len() + std::mem::size_of::<u32>();
    if bytes.len() < header_len || &bytes[..SHRIMP_MAGIC.len()] != SHRIMP_MAGIC {
        return Err(format!(
            "{} is not a supported shrimp project file",
            path.display()
        ));
    }
    let version_offset = SHRIMP_MAGIC.len();
    let version = u32::from_le_bytes(
        bytes[version_offset..header_len]
            .try_into()
            .expect("shrimp version slice has a fixed length"),
    );
    if version != SHRIMP_VERSION {
        return Err(format!(
            "{} has unsupported shrimp version {version}, expected {SHRIMP_VERSION}",
            path.display()
        ));
    }
    Ok(&bytes[header_len..])
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

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}
