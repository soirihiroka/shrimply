use sanim_core::digest;
pub use shrimply_asset::remote::{is_url as is_remote,invalidate as invalidate_remote};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A source location in the same revision as its prepared animation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SourceLocation {
    pub source: Arc<str>,
    pub span: sanim_core::Span,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ElementSource {
    pub creation: SourceLocation,
    /// Outer caller first, immediate caller last. Generated children share this metadata.
    pub call_sites: imbl::Vector<SourceLocation>,
}

impl ElementSource {
    /// Conservative retained size; deliberately includes shared call-site storage.
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.creation.source.len()
            + self.call_sites.len() * std::mem::size_of::<SourceLocation>()
            + self
                .call_sites
                .iter()
                .map(|site| site.source.len())
                .sum::<usize>()
    }
}

impl crate::Prepared {
    /// Read the original line from this revision, without accessing the filesystem.
    pub fn source_line(&self, location: &SourceLocation) -> Option<&str> {
        let bytes = self.source_texts.get(location.source.as_ref())?;
        let offset = location.span.start;
        if offset > bytes.len() {
            return None;
        }
        let start = bytes[..offset]
            .iter()
            .rposition(|byte| matches!(*byte, b'\n' | b'\r'))
            .map_or(0, |index| index + 1);
        let end = bytes[offset..]
            .iter()
            .position(|byte| matches!(*byte, b'\n' | b'\r'))
            .map_or(bytes.len(), |index| offset + index);
        std::str::from_utf8(&bytes[start..end])
            .ok()
            .map(|line| line.trim_end_matches('\r'))
    }
}

/// Hosts supply bytes; Sanim itself has no network or ambient filesystem access.
pub trait Sources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    /// One immutable resource revision and its content fingerprint. Event-driven
    /// hosts may retain these together until that resource is invalidated.
    /// Plain/strict providers deliberately read fresh bytes on every request.
    fn read_snapshot(&self, path: &str) -> Result<(Arc<Vec<u8>>, String), String> {
        let bytes = Arc::new(self.read(path)?);
        let fingerprint = digest(&bytes);
        Ok((bytes, fingerprint))
    }
}


#[derive(Clone, Debug, Default)]
pub struct MemorySources {
    pub files: BTreeMap<String, Vec<u8>>,
}
impl MemorySources {
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl AsRef<[u8]>) {
        self.files.insert(path.into(), bytes.as_ref().to_vec());
    }
}
impl Sources for MemorySources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("resource '{path}' does not exist"))
    }
}

#[derive(Clone, Debug)]
pub struct FileSources {
    root: PathBuf,
    pub max_bytes: usize,
}
impl FileSources {
    /// Immutable canonical boundary shared with dependency watchers.
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn new(root: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            root: root.as_ref().canonicalize().map_err(|e| e.to_string())?,
            max_bytes: 64 * 1024 * 1024,
        })
    }
}
impl Sources for FileSources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let resolved = self
            .root
            .join(path)
            .canonicalize()
            .map_err(|e| format!("cannot read '{path}': {e}"))?;
        if !resolved.starts_with(&self.root) {
            return Err(format!(
                "resource '{path}' escapes the source root (including symlinks)"
            ));
        }
        let mut bytes = vec![];
        std::fs::File::open(resolved)
            .map_err(|e| e.to_string())?
            .take(self.max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > self.max_bytes {
            return Err(format!(
                "resource '{path}' exceeds {} bytes",
                self.max_bytes
            ));
        }
        Ok(bytes)
    }
    fn read_snapshot(&self,path:&str)->Result<(Arc<Vec<u8>>,String),String> {
        if is_remote(path) {
            let download=shrimply_asset::remote::load(path,self.max_bytes)?;
            Ok((download.bytes.clone(),download.fingerprint.clone()))
        } else {
            let bytes=Arc::new(self.read(path)?);
            let fingerprint=digest(&bytes);
            Ok((bytes,fingerprint))
        }
    }

}

pub fn resolve(module: &str, path: &str) -> Result<String, String> {
    if Path::new(path).is_absolute() {
        return Err("resource paths must be relative to the source root".into());
    }
    let base = Path::new(module).parent().unwrap_or(Path::new(""));
    let joined = base.join(path);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err("resource path escapes the source root".into());
                }
            }
            _ => return Err("invalid resource path".into()),
        }
    }
    out.to_str()
        .map(|v| v.replace('\\', "/"))
        .ok_or("resource path must be UTF-8".into())
}

pub(crate) struct Snapshot<'a> {
    pub provider: &'a dyn Sources,
    pub reads: BTreeMap<String, Result<Arc<Vec<u8>>, String>>,
    pub hashes: std::cell::RefCell<BTreeMap<String, String>>,
}
impl Snapshot<'_> {
    pub fn read(&mut self, path: &str) -> Result<Arc<Vec<u8>>, String> {
        if !self.reads.contains_key(path) {
            let read = self.provider.read_snapshot(path).map(|(bytes, fingerprint)| {
                self.hashes.borrow_mut().insert(path.into(), fingerprint);
                bytes
            });
            self.reads.insert(path.into(), read);
        }
        self.reads[path].clone()
    }
    pub fn fingerprints(&self) -> BTreeMap<String, String> {
        self.reads
            .keys()
            .map(|path| (path.clone(), self.fingerprint(path)))
            .collect()
    }
    pub fn fingerprint(&self, path: &str) -> String {
        if let Some(hash) = self.hashes.borrow().get(path) {
            return hash.clone();
        }
        let hash = match &self.reads[path] {
            Ok(bytes) => digest(bytes),
            Err(error) => format!("error:{error}"),
        };
        self.hashes
            .borrow_mut()
            .insert(path.to_owned(), hash.clone());
        hash
    }
}
