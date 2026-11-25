use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use shrimply_resource_pipeline::fingerprint::{self, HEX_DIGEST_BYTES};

use super::atomic_file;

const MAGIC: &[u8] = b"SHRIMPLY-RECOVERY\0\x01";
const HEADER_LEN: usize = MAGIC.len() + HEX_DIGEST_BYTES;

pub(super) fn path(project: &Path) -> PathBuf {
    let project = super::normalized_project_path(project);
    let key = fingerprint::digest(project.as_os_str().as_encoded_bytes());
    shrimply_path_core::config_directory()
        .join("session-recovery")
        .join(key)
}

pub(super) fn load(path: &Path, checkpoint: &str) -> Result<Option<Vec<u8>>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read recovery data {}: {error}",
                path.display()
            ));
        }
    };
    let (header, contents) = bytes
        .split_at_checked(HEADER_LEN)
        .filter(|(header, _)| header.starts_with(MAGIC))
        .ok_or_else(|| format!("invalid project recovery data at {}", path.display()))?;
    if fingerprint::digest(contents) == checkpoint {
        discard(path);
        return Ok(None);
    }
    if &header[MAGIC.len()..] != checkpoint.as_bytes() {
        return Err(format!(
            "project changed outside Shrimply while it had unsaved edits; recovery data is at {}",
            path.display()
        ));
    }
    Ok(Some(contents.to_vec()))
}

pub(super) fn write(path: &Path, checkpoint: &str, contents: &[u8]) -> Result<(), String> {
    assert_eq!(checkpoint.len(), HEX_DIGEST_BYTES);
    let directory = path.parent().expect("project recovery path has a parent");
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "could not create recovery directory {}: {error}",
            directory.display()
        )
    })?;
    atomic_file::replace(path, true, |file| {
        file.write_all(MAGIC)?;
        file.write_all(checkpoint.as_bytes())?;
        file.write_all(contents)
    })
    .map_err(|error| format!("could not write recovery data {}: {error}", path.display()))
}

pub(super) fn discard(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), "Could not clear project recovery data: {error}");
    }
}
