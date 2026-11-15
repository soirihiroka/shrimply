use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use super::{InspectorMedia, MediaInfoPresentation};

#[derive(Debug)]
pub struct CachedMetadata {
    pub presentation: MediaInfoPresentation,
    pub artwork: Option<Arc<[u8]>>,
    pub artwork_version: Option<String>,
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
    revision: Option<u64>,
}

type Response = (Key, Result<Arc<CachedMetadata>, String>);

pub struct Metadata {
    sender: mpsc::Sender<Response>,
    receiver: mpsc::Receiver<Response>,
    active: Option<Key>,
    pending: Vec<Key>,
    state: MetadataState,
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
        }
    }
}

impl Metadata {
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
            path: media.path.clone(),
            revision: media.revision,
        };
        if self.active.as_ref() != Some(&key) {
            self.active = Some(key.clone());
            self.state = MetadataState::Loading;
        }
        if matches!(self.state, MetadataState::Loading) && !self.pending.contains(&key) {
            self.pending.push(key.clone());
            let sender = self.sender.clone();
            thread::spawn(move || {
                let result = shrimply_media_info::inspect(&key.path, key.revision).map(|info| {
                    let mut presentation = super::media_info_presentation(&info, format_date);
                    let artwork: Option<Arc<[u8]>> = presentation
                        .artwork
                        .take()
                        .map(|artwork| artwork.data.into());
                    let artwork_version = artwork.as_deref().map(|bytes| {
                        let mut hash = DefaultHasher::new();
                        bytes.hash(&mut hash);
                        format!("{:016x}", hash.finish())
                    });
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
                        artwork_version,
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
