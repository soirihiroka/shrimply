#![cfg_attr(windows, feature(windows_process_extensions_raw_attribute))]

mod process;
pub use process::Child;

use rusqlite::{Connection, params};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock, mpsc};

pub const EXPLANATION: &str = "Blender and Manim files can run code with your account's permissions, including code they import. Only trust sources you control or whose authors you trust. File trust includes future edits and replacements. Folder trust includes all current and future files and subfolders.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    File,
    Folder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
}

#[derive(Clone, Debug, Default)]
pub struct Review {
    pub files: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
}

static STORE: LazyLock<Result<Mutex<Connection>, String>> = LazyLock::new(|| {
    let directory = shrimply_path_core::config_directory();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let conn = Connection::open(directory.join("settings.sqlite"))
        .map_err(|error| format!("Could not open trust database: {error}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trusted_sources (
            path BLOB NOT NULL,
            folder INTEGER NOT NULL CHECK (folder IN (0, 1)),
            PRIMARY KEY (path, folder)
        );",
    )
    .map_err(|error| format!("Could not initialize trust database: {error}"))?;
    Ok(Mutex::new(conn))
});

pub fn entries() -> Result<Vec<Entry>, String> {
    let conn = STORE
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .expect("trust database poisoned");
    let mut statement = conn
        .prepare("SELECT path, folder FROM trusted_sources ORDER BY path, folder")
        .map_err(|error| format!("Could not read trusted locations: {error}"))?;
    statement
        .query_map([], |row| {
            let path: Vec<u8> = row.get(0)?;
            Ok(Entry {
                path: bytes_to_path(path)?,
                kind: if row.get::<_, bool>(1)? {
                    Kind::Folder
                } else {
                    Kind::File
                },
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn covers(entries: &[Entry], path: &Path) -> bool {
    entries.iter().any(|entry| match entry.kind {
        Kind::File => path == entry.path,
        Kind::Folder => path.starts_with(&entry.path),
    })
}

pub fn require(path: &Path) -> Result<PathBuf, String> {
    let resolved = path.canonicalize().map_err(|error| {
        format!(
            "Could not resolve executable source {}: {error}",
            path.display()
        )
    })?;
    if !resolved.is_file() {
        return Err(format!(
            "Executable source is not a file: {}",
            resolved.display()
        ));
    }
    if !covers(&entries()?, &resolved) {
        return Err(format!(
            "Trust required to execute {}. Open the project or import the source in Shrimply to approve it.",
            resolved.display()
        ));
    }
    Ok(resolved)
}

impl Review {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let paths: Vec<_> = paths.into_iter().collect();
        if paths.is_empty() {
            return Ok(Self::default());
        }
        let entries = entries()?;
        let mut files = BTreeSet::new();
        for path in paths {
            let path = path.canonicalize().map_err(|error| {
                format!(
                    "Could not resolve executable source {}: {error}",
                    path.display()
                )
            })?;
            if !path.is_file() {
                return Err(format!(
                    "Executable source is not a file: {}",
                    path.display()
                ));
            }
            if !covers(&entries, &path) {
                files.insert(path);
            }
        }
        let mut folders = Vec::<PathBuf>::new();
        for parent in files
            .iter()
            .filter_map(|path| path.parent())
            .collect::<BTreeSet<_>>()
        {
            if !folders.iter().any(|folder| parent.starts_with(folder)) {
                folders.push(parent.to_path_buf());
            }
        }
        Ok(Self {
            files: files.into_iter().collect(),
            folders,
        })
    }

    pub fn details(&self) -> String {
        format!(
            "Files:\n{}\n\nFolders (including subfolders):\n{}",
            self.files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            self.folders
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    pub fn approve(&self, kind: Kind) -> Result<(), String> {
        // Approve exactly the paths shown, never a changed symlink target.
        for file in &self.files {
            if file.canonicalize().ok().as_ref() != Some(file) || !file.is_file() {
                return Err(format!(
                    "Source changed while awaiting trust: {}",
                    file.display()
                ));
            }
        }
        let paths = match kind {
            Kind::File => &self.files,
            Kind::Folder => &self.folders,
        };
        let mut conn = STORE
            .as_ref()
            .map_err(Clone::clone)?
            .lock()
            .expect("trust database poisoned");
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        for path in paths {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO trusted_sources (path, folder) VALUES (?1, ?2)",
                    params![path_to_bytes(path), kind == Kind::Folder],
                )
                .map_err(|error| format!("Could not save trust: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Could not save trust: {error}"))
    }
}

pub fn remove(entry: &Entry) -> Result<(), String> {
    let conn = STORE
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .expect("trust database poisoned");
    conn.execute(
        "DELETE FROM trusted_sources WHERE path = ?1 AND folder = ?2",
        params![path_to_bytes(&entry.path), entry.kind == Kind::Folder],
    )
    .map_err(|error| format!("Could not remove trust: {error}"))?;
    Ok(())
}

#[cfg(unix)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn bytes_to_path(bytes: Vec<u8>) -> rusqlite::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(bytes).into())
}

#[cfg(windows)]
fn path_to_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn bytes_to_path(bytes: Vec<u8>) -> rusqlite::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    if !bytes.len().is_multiple_of(2) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let wide = bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    Ok(std::ffi::OsString::from_wide(&wide).into())
}

pub struct Request {
    pub review: Review,
    pub response: mpsc::Sender<Result<(), String>>,
}

static INTERACTIVE: OnceLock<mpsc::Sender<Request>> = OnceLock::new();

type PendingEdit = (
    mpsc::Receiver<Result<(), String>>,
    Box<dyn FnOnce() -> Result<(), String>>,
);
thread_local! {
    static PENDING_EDITS: std::cell::RefCell<Vec<PendingEdit>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Preserve a native editor action while its source approval is pending.
pub fn defer_edit(
    review: Review,
    apply: impl FnOnce() -> Result<(), String> + 'static,
) -> Result<(), String> {
    let sender = INTERACTIVE
        .get()
        .ok_or("Trust approval requires a native editor")?;
    let (response, result) = mpsc::channel();
    sender
        .send(Request { review, response })
        .map_err(|_| "Trust dialog is unavailable")?;
    PENDING_EDITS.with_borrow_mut(|pending| pending.push((result, Box::new(apply))));
    Ok(())
}

pub fn poll_edits() -> Vec<String> {
    let mut errors = Vec::new();
    let edits = PENDING_EDITS.with_borrow_mut(std::mem::take);
    for (receiver, apply) in edits {
        let result = match receiver.try_recv() {
            Ok(Ok(())) => apply(),
            Ok(Err(error)) => Err(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Trust dialog closed before the edit was approved".into())
            }
            Err(mpsc::TryRecvError::Empty) => {
                PENDING_EDITS.with_borrow_mut(|pending| pending.push((receiver, apply)));
                continue;
            }
        };
        if let Err(error) = result
            && error != "Trust approval was canceled"
        {
            errors.push(error);
        }
    }
    errors
}

/// Installed only by native editors. Execution backends never call this API.
pub fn interactive_requests() -> mpsc::Receiver<Request> {
    let (sender, receiver) = mpsc::channel();
    INTERACTIVE
        .set(sender)
        .unwrap_or_else(|_| panic!("trust request handler already installed"));
    receiver
}

/// Called by an explicit import worker, never by preview, export, or MCP.
pub fn approve_import(paths: Vec<PathBuf>) -> Result<(), String> {
    let review = Review::new(paths)?;
    if review.files.is_empty() {
        return Ok(());
    }
    let sender = INTERACTIVE
        .get()
        .ok_or_else(|| format!("Trust required:\n{}", review.details()))?;
    let (response, result) = mpsc::channel();
    sender
        .send(Request { review, response })
        .map_err(|_| "Trust dialog is unavailable")?;
    result.recv().map_err(|_| "Trust approval was canceled")?
}
