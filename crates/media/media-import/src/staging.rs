use std::{
    fs,
    path::{Path, PathBuf},
};

/// Reserve a local name without replacing a previous import or remux.
pub(super) fn create_unique(mut path: PathBuf) -> Result<(PathBuf, fs::File), String> {
    loop {
        match fs::File::create_new(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut unique = path.with_file_name(uuid::Uuid::new_v4().to_string());
                // Keep the media extension when a downloaded file is named e.g. film.mkv.part.
                if let Some(extension) = super::media_extension(&path) {
                    unique.set_extension(extension);
                }
                if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("part"))
                {
                    unique.add_extension("part");
                }
                path = unique;
            }
            Err(error) => return Err(format!("could not create {}: {error}", path.display())),
        }
    }
}

/// Project-owned media copied from a path or a native document descriptor.
/// Callers retain existing project-local files and discard newly staged files
/// only after checking that an intervening edit has not referenced them.
pub struct StagedSource {
    pub path: PathBuf,
    pub created: bool,
}

pub fn stage_source(
    source: &Path,
    source_file: Option<fs::File>,
    name: &str,
    project_directory: &Path,
) -> Result<StagedSource, String> {
    let canonical_source = if source_file.is_none() {
        source.canonicalize().ok()
    } else {
        None
    };
    let directory = canonical_source
        .as_ref()
        .filter(|source| source.starts_with(project_directory))
        .and_then(|source| source.parent())
        .map(Path::to_owned)
        .unwrap_or_else(|| project_directory.join("media"));
    stage_source_in(source, source_file, name, &directory)
}

/// Copy a document-provider source into a caller-owned media directory.
pub fn stage_source_in(
    source: &Path,
    source_file: Option<fs::File>,
    name: &str,
    directory: &Path,
) -> Result<StagedSource, String> {
    if Path::new(name).file_name().and_then(|name| name.to_str()) != Some(name) {
        return Err("imported file name must not contain a directory".into());
    }
    let canonical_source = if source_file.is_none() {
        source.canonicalize().ok()
    } else {
        None
    };
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create project media directory: {error}"))?;
    let destination = directory.join(name);
    let already_staged = canonical_source
        .as_ref()
        .is_some_and(|source| destination.canonicalize().ok().as_ref() == Some(source));
    if already_staged {
        return Ok(StagedSource {
            path: destination,
            created: false,
        });
    }
    let (destination, mut file) = create_unique(destination)?;
    let copied = source_file
        .map(Ok)
        .unwrap_or_else(|| fs::File::open(source))
        .and_then(|mut source| std::io::copy(&mut source, &mut file))
        .and_then(|_| file.sync_all());
    if let Err(error) = copied {
        drop(file);
        fs::remove_file(&destination).map_err(|cleanup| {
            format!(
                "could not save imported asset: {error}; could not remove {}: {cleanup}",
                destination.display()
            )
        })?;
        return Err(format!("could not save imported asset: {error}"));
    }
    Ok(StagedSource {
        path: destination,
        created: true,
    })
}
