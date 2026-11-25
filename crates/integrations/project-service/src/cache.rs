use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::SystemTime,
};

use shrimply_resource_pipeline::fingerprint;
use uuid::Uuid;

use crate::{MediaBlob, ProjectClient, RequestError};

const BLOCK_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct MediaCache(Arc<DiskCache>);

struct DiskCache {
    directory: PathBuf,
    budget: u64,
    index: Mutex<CacheIndex>,
    fetching: Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>,
}

struct CacheEntry {
    length: u64,
    used: SystemTime,
}

#[derive(Default)]
struct CacheIndex {
    length: u64,
    entries: BTreeMap<PathBuf, CacheEntry>,
}

impl CacheIndex {
    fn record(&mut self, path: PathBuf, entry: CacheEntry) {
        self.length += entry.length;
        if let Some(previous) = self.entries.insert(path, entry) {
            self.length -= previous.length;
        }
    }

    fn trim(&mut self, maximum: u64) -> io::Result<()> {
        while self.length > maximum {
            let (path, entry) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .expect("a nonempty cache has an entry");
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.length -= entry.length;
            let path = path.clone();
            self.entries.remove(&path);
        }
        Ok(())
    }
}

impl MediaCache {
    pub fn new(directory: PathBuf, budget: u64) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let mut index = CacheIndex::default();
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "range" || extension == "asset")
                && entry.file_type()?.is_file()
            {
                let metadata = entry.metadata()?;
                index.record(
                    entry.path(),
                    CacheEntry {
                        length: metadata.len(),
                        used: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    },
                );
            }
        }
        index.trim(budget)?;
        Ok(Self(Arc::new(DiskCache {
            directory,
            budget,
            index: Mutex::new(index),
            fetching: Mutex::new(BTreeMap::new()),
        })))
    }

    pub(super) fn keep_uploaded(
        &self,
        endpoint: &str,
        project: Uuid,
        blob: &MediaBlob,
        source: &Path,
    ) -> io::Result<()> {
        if blob.length > self.0.budget {
            return Ok(());
        }
        let destination = self.complete_path(&media_key(endpoint, project, &blob.digest));
        match fs::hard_link(source, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                let mut temporary = tempfile::NamedTempFile::new_in(&self.0.directory)?;
                let copied = io::copy(&mut File::open(source)?, &mut temporary)?;
                if copied != blob.length {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "staged media changed",
                    ));
                }
                temporary.persist(destination.clone())?;
            }
        }
        self.record(destination, blob.length)
    }

    fn complete_path(&self, key: &str) -> PathBuf {
        self.0.directory.join(format!("{key}.asset"))
    }

    fn complete_file(&self, key: &str, length: u64) -> Option<File> {
        let path = self.complete_path(key);
        let file = File::open(&path).ok()?;
        if file.metadata().ok()?.len() != length {
            return None;
        }
        let _ = self.record(path, length);
        Some(file)
    }

    fn read_block(&self, reader: &MediaReader, start: u64) -> io::Result<Vec<u8>> {
        let cache = &self.0;
        let block = start / BLOCK_BYTES;
        let length = (reader.blob.length - start).min(BLOCK_BYTES) as usize;
        let path = cache
            .directory
            .join(format!("{}-{block}.range", reader.key));
        if let Some(bytes) = self.read_cached(&path, length) {
            return Ok(bytes);
        }
        let fetch = {
            let mut fetching = cache
                .fetching
                .lock()
                .expect("media cache fetch list poisoned");
            fetching.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = fetching.get(&path).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                fetching.insert(path.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _fetch = fetch.lock().expect("media cache fetch lock poisoned");
        if let Some(bytes) = self.read_cached(&path, length) {
            return Ok(bytes);
        }
        let bytes = reader
            .client
            .read_range(reader.project, &reader.blob, start, length)
            .map_err(io::Error::other)?;
        if length as u64 <= cache.budget
            && let Err(error) = self.write_block(&path, &bytes)
        {
            eprintln!("Could not cache media at {}: {error}", path.display());
        }
        Ok(bytes)
    }

    fn read_cached(&self, path: &Path, length: usize) -> Option<Vec<u8>> {
        let mut bytes = Vec::with_capacity(length);
        let mut file = match File::open(path) {
            Ok(file) => file.take(length as u64 + 1),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                eprintln!("Could not read cached media at {}: {error}", path.display());
                return None;
            }
        };
        if let Err(error) = file.read_to_end(&mut bytes) {
            eprintln!("Could not read cached media at {}: {error}", path.display());
            return None;
        }
        if bytes.len() != length {
            return None;
        }
        if let Err(error) = self.record(path.to_path_buf(), length as u64) {
            eprintln!("Could not prune media cache: {error}");
        }
        Some(bytes)
    }

    fn write_block(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut temporary = tempfile::NamedTempFile::new_in(&self.0.directory)?;
        temporary.write_all(bytes)?;
        temporary.persist(path)?;
        self.record(path.to_path_buf(), bytes.len() as u64)
    }

    fn record(&self, path: PathBuf, length: u64) -> io::Result<()> {
        let cache = &self.0;
        let mut index = cache.index.lock().expect("media range cache poisoned");
        index.record(
            path,
            CacheEntry {
                length,
                used: SystemTime::now(),
            },
        );
        index.trim(cache.budget)
    }
}

/// Seekable editor input. It keeps one active block in memory and retrieves
/// missing byte ranges on demand. Other readers share the same disk cache.
pub struct MediaReader {
    client: ProjectClient,
    cache: MediaCache,
    project: Uuid,
    blob: MediaBlob,
    key: String,
    position: u64,
    active: Option<(u64, Vec<u8>)>,
    complete: Option<File>,
}

fn media_key(endpoint: &str, project: Uuid, digest: &str) -> String {
    fingerprint::digest(format!("{endpoint}\n{project}\n{digest}").as_bytes())
}

impl MediaReader {
    pub(super) fn new(
        client: ProjectClient,
        endpoint: &str,
        project: Uuid,
        blob: MediaBlob,
        cache: MediaCache,
    ) -> Result<Self, RequestError> {
        if !shrimply_project_protocol::valid_digest(&blob.digest) {
            return Err(RequestError::Media(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid media digest",
            )));
        }
        let key = media_key(endpoint, project, &blob.digest);
        let complete = cache.complete_file(&key, blob.length);
        Ok(Self {
            client,
            cache,
            project,
            blob,
            key,
            position: 0,
            active: None,
            complete,
        })
    }
}

impl Read for MediaReader {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if let Some(file) = &mut self.complete {
            let read = file.read(destination)?;
            self.position += read as u64;
            return Ok(read);
        }
        if self.position >= self.blob.length || destination.is_empty() {
            return Ok(0);
        }
        let start = self.position - self.position % BLOCK_BYTES;
        if self
            .active
            .as_ref()
            .is_none_or(|(offset, _)| *offset != start)
        {
            self.active = Some((start, self.cache.read_block(self, start)?));
        }
        let (_, block) = self.active.as_ref().expect("read initialized active block");
        let offset = (self.position - start) as usize;
        let copied = destination.len().min(block.len() - offset);
        destination[..copied].copy_from_slice(&block[offset..offset + copied]);
        self.position += copied as u64;
        Ok(copied)
    }
}

impl Seek for MediaReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.blob.length.checked_add_signed(offset),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid media seek"))?;
        if let Some(file) = &mut self.complete {
            file.seek(SeekFrom::Start(position))?;
        }
        self.position = position;
        Ok(self.position)
    }
}
