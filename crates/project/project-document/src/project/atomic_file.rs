use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static GENERATION: AtomicU64 = AtomicU64::new(0);

pub(super) fn create(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish(path, false, false, |file| file.write_all(bytes))
}

#[cfg(feature = "editor")]
pub(super) fn replace(
    path: &Path,
    durable: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    publish(path, true, durable, write)
}

fn publish(
    path: &Path,
    overwrite: bool,
    durable: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output filename"))?
        .to_os_string();
    name.push(format!(".tmp-{}-{generation}", std::process::id()));
    let temporary = path.with_file_name(name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = write(&mut file).and_then(|()| if durable { file.sync_all() } else { Ok(()) });
    drop(file);
    let result = result.and_then(|()| {
        if overwrite {
            fs::rename(&temporary, path)
        } else {
            publish_new_file(&temporary, path)
        }
    });
    let _ = fs::remove_file(&temporary);
    result?;
    #[cfg(unix)]
    if durable {
        let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
        // The rename has published the commit. A failure here must not
        // make the caller roll back media already referenced by it.
        if let Err(error) = File::open(parent.unwrap_or_else(|| Path::new(".")))
            .and_then(|directory| directory.sync_all())
        {
            tracing::warn!(path = %path.display(), "Could not sync project directory: {error}");
        }
    }
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
