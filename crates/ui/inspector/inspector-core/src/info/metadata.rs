use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

use super::{InspectorMedia, MediaInfoPresentation};

/// A bounded PNG produced by the shared still-image decoder on the metadata worker.
/// Its file lives as long as the metadata or a native inspector holds the thumbnail.
#[derive(Debug)]
pub struct Artwork(tempfile::TempPath);

impl PartialEq for Artwork {
    fn eq(&self, other: &Self) -> bool {
        self.path() == other.path()
    }
}

impl Artwork {
    pub fn path(&self) -> &Path {
        &self.0
    }

    fn prepare(bytes: Vec<u8>) -> Result<Arc<Self>, String> {
        let source = shrimply_drawing_skia::media::prepare(Arc::new(bytes))?;
        let mut surface = source.thumbnail(
            super::ARTWORK_PREVIEW_HEIGHT,
            skia_safe::ColorType::N32,
            skia_safe::ColorSpace::new_srgb(),
        )?;
        let encoded = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .ok_or("Could not encode artwork thumbnail")?;
        let directory = shrimply_path_core::cache_directory()?;
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let mut file = tempfile::Builder::new()
            .prefix("artwork-")
            .suffix(".png")
            .tempfile_in(directory)
            .map_err(|error| error.to_string())?;
        file.write_all(encoded.as_bytes())
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(Self(file.into_temp_path())))
    }
}

#[derive(Debug)]
pub struct CachedMetadata {
    pub presentation: MediaInfoPresentation,
    pub artwork: Option<Result<Arc<Artwork>, String>>,
    pub audio_stream_count: u32,
    pub video_stream_count: u32,
}

#[derive(Clone, Debug)]
pub enum MetadataState {
    Loading,
    Ready(Arc<CachedMetadata>),
    Failed(String),
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    fingerprint: Option<String>,
    remote_generation: Option<u64>,
}

type Response = (Key, Result<Arc<CachedMetadata>, String>);

pub struct Metadata {
    sender: mpsc::Sender<Response>,
    receiver: mpsc::Receiver<Response>,
    active: Option<Key>,
    pending: Vec<Key>,
    state: MetadataState,
    remote_generation: u64,
}

impl Default for Metadata {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active: None,
            pending: Vec::new(),
            state: MetadataState::Loading,
            remote_generation: 0,
        }
    }
}

impl Metadata {
    pub fn remote_reconnected(&mut self) {
        self.remote_generation = self.remote_generation.wrapping_add(1);
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, result)) = self.receiver.try_recv() {
            self.pending.retain(|pending| pending != &key);
            if self.active.as_ref() == Some(&key) {
                self.state = match result {
                    Ok(info) => MetadataState::Ready(info),
                    Err(error) => MetadataState::Failed(error),
                };
                changed = true;
            }
        }
        changed
    }

    pub fn request(
        &mut self,
        media: Option<&InspectorMedia>,
        format_date: fn(i64) -> Option<String>,
    ) -> Option<MetadataState> {
        let Some(media) = media else {
            self.active = None;
            self.state = MetadataState::Loading;
            return None;
        };
        let key = Key {
            path: media.asset.path().to_path_buf(),
            fingerprint: media.fingerprint.clone(),
            remote_generation: media
                .asset
                .has_bound_reader()
                .then_some(self.remote_generation),
        };
        if self.active.as_ref() != Some(&key) {
            self.active = Some(key.clone());
            self.state = MetadataState::Loading;
        }
        if matches!(self.state, MetadataState::Loading) && !self.pending.contains(&key) {
            self.pending.push(key.clone());
            let sender = self.sender.clone();
            let asset = media.asset.clone();
            thread::spawn(move || {
                let result = shrimply_media_info::inspect(&asset).map(|info| {
                    let mut presentation = super::media_info_presentation(&info, format_date);
                    let artwork = presentation
                        .artwork
                        .take()
                        .map(|artwork| Artwork::prepare(artwork.data));
                    let stream_count = |kind| {
                        u32::try_from(
                            info.streams
                                .iter()
                                .filter(|stream| stream.kind == kind)
                                .count(),
                        )
                        .expect("media stream count must fit u32")
                    };
                    Arc::new(CachedMetadata {
                        presentation,
                        artwork,
                        audio_stream_count: stream_count("audio"),
                        video_stream_count: stream_count("video"),
                    })
                });
                let _ = sender.send((key, result));
            });
        }
        Some(self.state.clone())
    }
}
