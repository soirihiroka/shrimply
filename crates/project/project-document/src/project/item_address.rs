use shrimply_math_core::Fraction;
use uuid::Uuid;

use super::{
    AudioItem, AudioSource, AudioTrack, CaptionItem, CaptionTrack, CommentItem, CommentTrack,
    Project, Time, VideoItem, VideoItemContent, VisualTrack, playback_speed_is_zero,
    scaled_time_delta, unscaled_time_delta,
};

pub use shrimply_project_types::TrackKind as ItemKind;

/// The logical nesting of shared folded-sequence instances.
///
/// Concrete item paths contain the IDs of the video or audio items presenting a
/// sequence.  Those paths differ for linked video/audio presenters.  A scope is
/// instead made from `SequenceReference::instance_id` values, so linked
/// presenters resolve to the same editing context.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct SequenceScopeId(Vec<Uuid>);

impl SequenceScopeId {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn from_instance_ids(instance_ids: Vec<Uuid>) -> Self {
        Self(instance_ids)
    }

    pub fn instance_ids(&self) -> &[Uuid] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parent(&self) -> Option<Self> {
        (!self.0.is_empty()).then(|| Self(self.0[..self.0.len() - 1].to_vec()))
    }

    pub fn child(&self, instance_id: Uuid) -> Self {
        let mut instance_ids = self.0.clone();
        instance_ids.push(instance_id);
        Self(instance_ids)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TrackAddress {
    Comment {
        track_id: Uuid,
    },
    Caption {
        track_id: Uuid,
    },
    Video {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
    },
    Audio {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ItemAddress {
    Comment {
        track_id: Uuid,
        item_id: Uuid,
    },
    Caption {
        track_id: Uuid,
        item_id: Uuid,
    },
    Video {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
        item_id: Uuid,
    },
    Audio {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
        item_id: Uuid,
    },
}

pub enum TrackRef<'a> {
    Comment(&'a CommentTrack),
    Caption(&'a CaptionTrack),
    Video(&'a VisualTrack),
    Audio(&'a AudioTrack),
}

impl TrackRef<'_> {
    pub fn enabled(&self) -> bool {
        match self {
            Self::Comment(track) => track.enabled,
            Self::Caption(track) => track.enabled,
            Self::Video(track) => track.enabled,
            Self::Audio(track) => track.enabled,
        }
    }
}

pub enum TrackMut<'a> {
    Comment(&'a mut CommentTrack),
    Caption(&'a mut CaptionTrack),
    Video(&'a mut VisualTrack),
    Audio(&'a mut AudioTrack),
}

pub enum ItemRef<'a> {
    Comment(&'a CommentItem),
    Caption(&'a CaptionItem),
    Video(&'a VideoItem),
    Audio(&'a AudioItem),
}

pub enum ItemMut<'a> {
    Comment(&'a mut CommentItem),
    Caption(&'a mut CaptionItem),
    Video(&'a mut VideoItem),
    Audio(&'a mut AudioItem),
}

#[derive(Clone, Debug)]
pub enum ProjectItem {
    Comment(CommentItem),
    Caption(CaptionItem),
    Video(Box<VideoItem>),
    Audio(Box<AudioItem>),
}

impl TrackAddress {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Comment { .. } => ItemKind::Comment,
            Self::Caption { .. } => ItemKind::Caption,
            Self::Video { .. } => ItemKind::Video,
            Self::Audio { .. } => ItemKind::Audio,
        }
    }

    pub fn sequence_path(&self) -> &[Uuid] {
        match self {
            Self::Comment { .. } => &[],
            Self::Caption { .. } => &[],
            Self::Video { sequence_path, .. } | Self::Audio { sequence_path, .. } => sequence_path,
        }
    }

    pub fn track_id(&self) -> Uuid {
        match self {
            Self::Comment { track_id }
            | Self::Caption { track_id }
            | Self::Video { track_id, .. }
            | Self::Audio { track_id, .. } => *track_id,
        }
    }

    pub fn is_root(&self) -> bool {
        self.sequence_path().is_empty()
    }

    pub fn item(&self, item_id: Uuid) -> ItemAddress {
        match self {
            Self::Comment { track_id } => ItemAddress::Comment {
                track_id: *track_id,
                item_id,
            },
            Self::Caption { track_id } => ItemAddress::Caption {
                track_id: *track_id,
                item_id,
            },
            Self::Video {
                sequence_path,
                track_id,
            } => ItemAddress::Video {
                sequence_path: sequence_path.clone(),
                track_id: *track_id,
                item_id,
            },
            Self::Audio {
                sequence_path,
                track_id,
            } => ItemAddress::Audio {
                sequence_path: sequence_path.clone(),
                track_id: *track_id,
                item_id,
            },
        }
    }
}

impl ItemAddress {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Comment { .. } => ItemKind::Comment,
            Self::Caption { .. } => ItemKind::Caption,
            Self::Video { .. } => ItemKind::Video,
            Self::Audio { .. } => ItemKind::Audio,
        }
    }

    pub fn track(&self) -> TrackAddress {
        match self {
            Self::Comment { track_id, .. } => TrackAddress::Comment {
                track_id: *track_id,
            },
            Self::Caption { track_id, .. } => TrackAddress::Caption {
                track_id: *track_id,
            },
            Self::Video {
                sequence_path,
                track_id,
                ..
            } => TrackAddress::Video {
                sequence_path: sequence_path.clone(),
                track_id: *track_id,
            },
            Self::Audio {
                sequence_path,
                track_id,
                ..
            } => TrackAddress::Audio {
                sequence_path: sequence_path.clone(),
                track_id: *track_id,
            },
        }
    }

    pub fn sequence_path(&self) -> &[Uuid] {
        match self {
            Self::Comment { .. } => &[],
            Self::Caption { .. } => &[],
            Self::Video { sequence_path, .. } | Self::Audio { sequence_path, .. } => sequence_path,
        }
    }

    pub fn track_id(&self) -> Uuid {
        match self {
            Self::Comment { track_id, .. }
            | Self::Caption { track_id, .. }
            | Self::Video { track_id, .. }
            | Self::Audio { track_id, .. } => *track_id,
        }
    }

    pub fn item_id(&self) -> Uuid {
        match self {
            Self::Comment { item_id, .. }
            | Self::Caption { item_id, .. }
            | Self::Video { item_id, .. }
            | Self::Audio { item_id, .. } => *item_id,
        }
    }

    pub fn is_root(&self) -> bool {
        self.sequence_path().is_empty()
    }
}

impl<'a> TrackRef<'a> {
    /// Visit borrowed items once without resolving each stable ID again.
    pub fn items(self) -> impl Iterator<Item = ItemRef<'a>> {
        let (mut comments, mut captions, mut videos, mut audio) = (None, None, None, None);
        match self {
            Self::Comment(track) => comments = Some(track.items.as_slice()),
            Self::Caption(track) => captions = Some(track.items.as_slice()),
            Self::Video(track) => videos = Some(track.items.as_slice()),
            Self::Audio(track) => audio = Some(track.items.as_slice()),
        }
        comments
            .into_iter()
            .flatten()
            .map(ItemRef::Comment)
            .chain(captions.into_iter().flatten().map(ItemRef::Caption))
            .chain(videos.into_iter().flatten().map(ItemRef::Video))
            .chain(audio.into_iter().flatten().map(ItemRef::Audio))
    }
}

impl<'a> ItemRef<'a> {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Comment(_) => ItemKind::Comment,
            Self::Caption(_) => ItemKind::Caption,
            Self::Video(_) => ItemKind::Video,
            Self::Audio(_) => ItemKind::Audio,
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            Self::Comment(item) => item.id,
            Self::Caption(item) => item.id,
            Self::Video(item) => item.id,
            Self::Audio(item) => item.id,
        }
    }

    pub fn times(&self) -> (Time, Time) {
        match self {
            Self::Comment(item) => (item.start, item.end),
            Self::Caption(item) => (item.start, item.end),
            Self::Video(item) => (item.start, item.end),
            Self::Audio(item) => (item.start, item.end),
        }
    }
}

impl ProjectItem {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Comment(_) => ItemKind::Comment,
            Self::Caption(_) => ItemKind::Caption,
            Self::Video(_) => ItemKind::Video,
            Self::Audio(_) => ItemKind::Audio,
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            Self::Comment(item) => item.id,
            Self::Caption(item) => item.id,
            Self::Video(item) => item.id,
            Self::Audio(item) => item.id,
        }
    }

    pub fn times(&self) -> (Time, Time) {
        match self {
            Self::Comment(item) => (item.start, item.end),
            Self::Caption(item) => (item.start, item.end),
            Self::Video(item) => (item.start, item.end),
            Self::Audio(item) => (item.start, item.end),
        }
    }

    pub fn set_times(&mut self, start: Time, end: Time) {
        let (start, end) = (start.min(end), start.max(end));
        match self {
            Self::Comment(item) => {
                item.start = start;
                item.end = end;
            }
            Self::Caption(item) => {
                item.start = start;
                item.end = end;
            }
            Self::Video(item) => {
                item.start = start;
                item.end = end;
            }
            Self::Audio(item) => {
                item.start = start;
                item.end = end;
            }
        }
    }
}

impl Project {
    pub fn root_track_index(&self, kind: ItemKind, id: Uuid) -> Option<usize> {
        match kind {
            ItemKind::Comment => self.comment_tracks.iter().position(|track| track.id == id),
            ItemKind::Caption => self.caption_tracks.iter().position(|track| track.id == id),
            ItemKind::Video => self.video_tracks.iter().position(|track| track.id == id),
            ItemKind::Audio => self.audio_tracks.iter().position(|track| track.id == id),
        }
    }

    pub fn track_scope(&self, address: &TrackAddress) -> Option<SequenceScopeId> {
        self.sequence_scope_for_path(address.kind(), address.sequence_path())
    }

    pub fn item_scope(&self, address: &ItemAddress) -> Option<SequenceScopeId> {
        self.sequence_scope_for_path(address.kind(), address.sequence_path())
    }

    pub fn sequence_scope_for_path(
        &self,
        kind: ItemKind,
        sequence_path: &[Uuid],
    ) -> Option<SequenceScopeId> {
        let mut instance_ids = Vec::with_capacity(sequence_path.len());
        match kind {
            ItemKind::Comment => return sequence_path.is_empty().then(SequenceScopeId::root),
            ItemKind::Caption => return sequence_path.is_empty().then(SequenceScopeId::root),
            ItemKind::Video => {
                let mut tracks = self.video_tracks.as_slice();
                for host_id in sequence_path {
                    let host = tracks
                        .iter()
                        .flat_map(|track| &track.items)
                        .find(|item| item.id == *host_id)?;
                    let VideoItemContent::FoldedSequence(reference) = host.content else {
                        return None;
                    };
                    instance_ids.push(reference.instance_id);
                    tracks = &self.folded_sequence(reference.sequence_id)?.video_tracks;
                }
            }
            ItemKind::Audio => {
                let mut tracks = self.audio_tracks.as_slice();
                for host_id in sequence_path {
                    let host = tracks
                        .iter()
                        .flat_map(|track| &track.items)
                        .find(|item| item.id == *host_id)?;
                    let AudioSource::FoldedSequence(reference) = &host.source else {
                        return None;
                    };
                    instance_ids.push(reference.instance_id);
                    tracks = &self.folded_sequence(reference.sequence_id)?.audio_tracks;
                }
            }
        }
        Some(SequenceScopeId::from_instance_ids(instance_ids))
    }

    /// Returns every concrete presentation path for a logical sequence scope.
    /// More than one path is possible when malformed or intentionally duplicated
    /// presenters share an instance ID, so callers must not silently pick one.
    pub fn sequence_paths_for_scope(
        &self,
        kind: ItemKind,
        scope: &SequenceScopeId,
    ) -> Vec<Vec<Uuid>> {
        match kind {
            ItemKind::Comment => scope.is_root().then(Vec::new).into_iter().collect(),
            ItemKind::Caption => scope.is_root().then(Vec::new).into_iter().collect(),
            ItemKind::Video => self.video_paths_for_scope(scope),
            ItemKind::Audio => self.audio_paths_for_scope(scope),
        }
    }

    /// Resolves a logical scope only when it has exactly one concrete presenter
    /// for the requested item kind.
    pub fn sequence_path_for_scope(
        &self,
        kind: ItemKind,
        scope: &SequenceScopeId,
    ) -> Option<Vec<Uuid>> {
        let mut paths = self.sequence_paths_for_scope(kind, scope).into_iter();
        let path = paths.next()?;
        paths.next().is_none().then_some(path)
    }

    pub fn sequence_id_for_scope(&self, scope: &SequenceScopeId) -> Option<Option<Uuid>> {
        let mut sequence_id = None;
        for instance_id in scope.instance_ids() {
            let (video_tracks, audio_tracks) = match sequence_id {
                Some(sequence_id) => {
                    let sequence = self.folded_sequence(sequence_id)?;
                    (
                        sequence.video_tracks.as_slice(),
                        sequence.audio_tracks.as_slice(),
                    )
                }
                None => (self.video_tracks.as_slice(), self.audio_tracks.as_slice()),
            };
            let mut references = video_tracks
                .iter()
                .flat_map(|track| &track.items)
                .filter_map(|item| match item.content {
                    VideoItemContent::FoldedSequence(reference)
                        if reference.instance_id == *instance_id =>
                    {
                        Some(reference)
                    }
                    _ => None,
                })
                .chain(
                    audio_tracks
                        .iter()
                        .flat_map(|track| &track.items)
                        .filter_map(|item| match &item.source {
                            AudioSource::FoldedSequence(reference)
                                if reference.instance_id == *instance_id =>
                            {
                                Some(*reference)
                            }
                            _ => None,
                        }),
                );
            let reference = references.next()?;
            if references.any(|candidate| candidate.sequence_id != reference.sequence_id) {
                return None;
            }
            self.folded_sequence(reference.sequence_id)?;
            sequence_id = Some(reference.sequence_id);
        }
        Some(sequence_id)
    }

    pub fn video_tracks_for_scope(&self, scope: &SequenceScopeId) -> Option<&[VisualTrack]> {
        match self.sequence_id_for_scope(scope)? {
            Some(sequence_id) => Some(&self.folded_sequence(sequence_id)?.video_tracks),
            None => Some(&self.video_tracks),
        }
    }

    pub fn audio_tracks_for_scope(&self, scope: &SequenceScopeId) -> Option<&[AudioTrack]> {
        match self.sequence_id_for_scope(scope)? {
            Some(sequence_id) => Some(&self.folded_sequence(sequence_id)?.audio_tracks),
            None => Some(&self.audio_tracks),
        }
    }

    fn video_paths_for_scope(&self, scope: &SequenceScopeId) -> Vec<Vec<Uuid>> {
        let mut candidates = vec![(self.video_tracks.as_slice(), Vec::new())];
        for instance_id in scope.instance_ids() {
            let mut next = Vec::new();
            for (tracks, path) in candidates {
                for item in tracks.iter().flat_map(|track| &track.items) {
                    let VideoItemContent::FoldedSequence(reference) = item.content else {
                        continue;
                    };
                    if reference.instance_id != *instance_id {
                        continue;
                    }
                    let Some(sequence) = self.folded_sequence(reference.sequence_id) else {
                        continue;
                    };
                    let mut child_path = path.clone();
                    child_path.push(item.id);
                    next.push((sequence.video_tracks.as_slice(), child_path));
                }
            }
            candidates = next;
        }
        candidates.into_iter().map(|(_, path)| path).collect()
    }

    fn audio_paths_for_scope(&self, scope: &SequenceScopeId) -> Vec<Vec<Uuid>> {
        let mut candidates = vec![(self.audio_tracks.as_slice(), Vec::new())];
        for instance_id in scope.instance_ids() {
            let mut next = Vec::new();
            for (tracks, path) in candidates {
                for item in tracks.iter().flat_map(|track| &track.items) {
                    let AudioSource::FoldedSequence(reference) = &item.source else {
                        continue;
                    };
                    if reference.instance_id != *instance_id {
                        continue;
                    }
                    let Some(sequence) = self.folded_sequence(reference.sequence_id) else {
                        continue;
                    };
                    let mut child_path = path.clone();
                    child_path.push(item.id);
                    next.push((sequence.audio_tracks.as_slice(), child_path));
                }
            }
            candidates = next;
        }
        candidates.into_iter().map(|(_, path)| path).collect()
    }

    pub fn track(&self, address: &TrackAddress) -> Option<TrackRef<'_>> {
        match address {
            TrackAddress::Comment { track_id } => self
                .comment_tracks
                .iter()
                .find(|track| track.id == *track_id)
                .map(TrackRef::Comment),
            TrackAddress::Caption { track_id } => self
                .caption_tracks
                .iter()
                .find(|track| track.id == *track_id)
                .map(TrackRef::Caption),
            TrackAddress::Video {
                sequence_path,
                track_id,
            } => self
                .video_tracks_for_path(sequence_path)?
                .iter()
                .find(|track| track.id == *track_id)
                .map(TrackRef::Video),
            TrackAddress::Audio {
                sequence_path,
                track_id,
            } => self
                .audio_tracks_for_path(sequence_path)?
                .iter()
                .find(|track| track.id == *track_id)
                .map(TrackRef::Audio),
        }
    }

    pub fn track_mut(&mut self, address: &TrackAddress) -> Option<TrackMut<'_>> {
        match address {
            TrackAddress::Comment { track_id } => self
                .comment_tracks
                .iter_mut()
                .find(|track| track.id == *track_id)
                .map(TrackMut::Comment),
            TrackAddress::Caption { track_id } => self
                .caption_tracks
                .iter_mut()
                .find(|track| track.id == *track_id)
                .map(TrackMut::Caption),
            TrackAddress::Video {
                sequence_path,
                track_id,
            } => {
                let sequence_id = self.video_sequence_for_path(sequence_path)?;
                let tracks = match sequence_id {
                    Some(sequence_id) => &mut self.folded_sequence_mut(sequence_id)?.video_tracks,
                    None => &mut self.video_tracks,
                };
                tracks
                    .iter_mut()
                    .find(|track| track.id == *track_id)
                    .map(TrackMut::Video)
            }
            TrackAddress::Audio {
                sequence_path,
                track_id,
            } => {
                let sequence_id = self.audio_sequence_for_path(sequence_path)?;
                let tracks = match sequence_id {
                    Some(sequence_id) => &mut self.folded_sequence_mut(sequence_id)?.audio_tracks,
                    None => &mut self.audio_tracks,
                };
                tracks
                    .iter_mut()
                    .find(|track| track.id == *track_id)
                    .map(TrackMut::Audio)
            }
        }
    }

    pub fn item(&self, address: &ItemAddress) -> Option<ItemRef<'_>> {
        match self.track(&address.track())? {
            TrackRef::Comment(track) => track
                .items
                .iter()
                .find(|item| item.id == address.item_id())
                .map(ItemRef::Comment),
            TrackRef::Caption(track) => track
                .items
                .iter()
                .find(|item| item.id == address.item_id())
                .map(ItemRef::Caption),
            TrackRef::Video(track) => track
                .items
                .iter()
                .find(|item| item.id == address.item_id())
                .map(ItemRef::Video),
            TrackRef::Audio(track) => track
                .items
                .iter()
                .find(|item| item.id == address.item_id())
                .map(ItemRef::Audio),
        }
    }

    pub fn item_mut(&mut self, address: &ItemAddress) -> Option<ItemMut<'_>> {
        let item_id = address.item_id();
        match self.track_mut(&address.track())? {
            TrackMut::Comment(track) => track
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .map(ItemMut::Comment),
            TrackMut::Caption(track) => track
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .map(ItemMut::Caption),
            TrackMut::Video(track) => track
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .map(ItemMut::Video),
            TrackMut::Audio(track) => track
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
                .map(ItemMut::Audio),
        }
    }

    pub fn comment_item(&self, address: &ItemAddress) -> Option<&CommentItem> {
        match self.item(address)? {
            ItemRef::Caption(_) => None,
            ItemRef::Comment(item) => Some(item),
            ItemRef::Video(_) | ItemRef::Audio(_) => None,
        }
    }

    pub fn comment_item_mut(&mut self, address: &ItemAddress) -> Option<&mut CommentItem> {
        match self.item_mut(address)? {
            ItemMut::Caption(_) => None,
            ItemMut::Comment(item) => Some(item),
            ItemMut::Video(_) | ItemMut::Audio(_) => None,
        }
    }

    pub fn caption_item(&self, address: &ItemAddress) -> Option<&CaptionItem> {
        match self.item(address)? {
            ItemRef::Comment(_) => None,
            ItemRef::Caption(item) => Some(item),
            ItemRef::Video(_) | ItemRef::Audio(_) => None,
        }
    }

    pub fn caption_item_mut(&mut self, address: &ItemAddress) -> Option<&mut CaptionItem> {
        match self.item_mut(address)? {
            ItemMut::Comment(_) => None,
            ItemMut::Caption(item) => Some(item),
            ItemMut::Video(_) | ItemMut::Audio(_) => None,
        }
    }

    pub fn video_item(&self, address: &ItemAddress) -> Option<&VideoItem> {
        match self.item(address)? {
            ItemRef::Video(item) => Some(item),
            ItemRef::Comment(_) | ItemRef::Caption(_) | ItemRef::Audio(_) => None,
        }
    }

    pub fn video_item_mut(&mut self, address: &ItemAddress) -> Option<&mut VideoItem> {
        match self.item_mut(address)? {
            ItemMut::Video(item) => Some(item),
            ItemMut::Comment(_) | ItemMut::Caption(_) | ItemMut::Audio(_) => None,
        }
    }

    pub fn audio_item(&self, address: &ItemAddress) -> Option<&AudioItem> {
        match self.item(address)? {
            ItemRef::Audio(item) => Some(item),
            ItemRef::Comment(_) | ItemRef::Caption(_) | ItemRef::Video(_) => None,
        }
    }

    pub fn audio_item_mut(&mut self, address: &ItemAddress) -> Option<&mut AudioItem> {
        match self.item_mut(address)? {
            ItemMut::Audio(item) => Some(item),
            ItemMut::Comment(_) | ItemMut::Caption(_) | ItemMut::Video(_) => None,
        }
    }

    pub fn take_item(&mut self, address: &ItemAddress) -> Option<ProjectItem> {
        let item_id = address.item_id();
        match self.track_mut(&address.track())? {
            TrackMut::Comment(track) => track
                .items
                .iter()
                .position(|item| item.id == item_id)
                .map(|index| ProjectItem::Comment(track.items.remove(index))),
            TrackMut::Caption(track) => track
                .items
                .iter()
                .position(|item| item.id == item_id)
                .map(|index| ProjectItem::Caption(track.items.remove(index))),
            TrackMut::Video(track) => {
                track
                    .items
                    .iter()
                    .position(|item| item.id == item_id)
                    .map(|index| {
                        let mut item = track.items.remove(index);
                        item.transitions.to_next = None;
                        for candidate in &mut track.items {
                            if candidate
                                .transitions
                                .to_next
                                .as_ref()
                                .is_some_and(|transition| transition.target_item_id == item_id)
                            {
                                candidate.transitions.to_next = None;
                            }
                        }
                        ProjectItem::Video(Box::new(item))
                    })
            }
            TrackMut::Audio(track) => {
                track
                    .items
                    .iter()
                    .position(|item| item.id == item_id)
                    .map(|index| {
                        let mut item = track.items.remove(index);
                        item.transitions.to_next = None;
                        for candidate in &mut track.items {
                            if candidate
                                .transitions
                                .to_next
                                .as_ref()
                                .is_some_and(|transition| transition.target_item_id == item_id)
                            {
                                candidate.transitions.to_next = None;
                            }
                        }
                        ProjectItem::Audio(Box::new(item))
                    })
            }
        }
    }

    pub fn can_insert_item(&self, track: &TrackAddress, kind: ItemKind) -> bool {
        track.kind() == kind && self.track(track).is_some()
    }

    pub fn can_move_item_to_sequence_path(
        &self,
        source: &ItemAddress,
        sequence_path: &[Uuid],
    ) -> bool {
        let reference = match self.item(source) {
            Some(ItemRef::Video(item)) => match &item.content {
                VideoItemContent::FoldedSequence(reference) => Some(*reference),
                _ => None,
            },
            Some(ItemRef::Audio(item)) => match &item.source {
                AudioSource::FoldedSequence(reference) => Some(*reference),
                AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
            },
            Some(ItemRef::Comment(_)) => None,
            Some(ItemRef::Caption(_)) => None,
            None => return false,
        };
        let Some(reference) = reference else {
            return true;
        };
        let destination = match source.kind() {
            ItemKind::Comment => Some(None),
            ItemKind::Caption => Some(None),
            ItemKind::Video => self.video_sequence_for_path(sequence_path),
            ItemKind::Audio => self.audio_sequence_for_path(sequence_path),
        };
        destination.is_some_and(|destination| {
            self.can_insert_sequence_reference(reference.sequence_id, destination)
        })
    }

    pub fn insert_item(&mut self, track: &TrackAddress, item: ProjectItem) -> Option<ItemAddress> {
        let item_id = item.id();
        let sequence_path = track.sequence_path().to_vec();
        match (self.track_mut(track)?, item) {
            (TrackMut::Comment(track), ProjectItem::Comment(item)) => {
                let index = track
                    .items
                    .partition_point(|candidate| candidate.start <= item.start);
                track.items.insert(index, item);
                Some(ItemAddress::Comment {
                    track_id: track.id,
                    item_id,
                })
            }
            (TrackMut::Caption(track), ProjectItem::Caption(item)) => {
                let index = track
                    .items
                    .partition_point(|candidate| candidate.start <= item.start);
                track.items.insert(index, item);
                Some(ItemAddress::Caption {
                    track_id: track.id,
                    item_id,
                })
            }
            (TrackMut::Video(track), ProjectItem::Video(item)) => {
                let index = track
                    .items
                    .partition_point(|candidate| candidate.start <= item.start);
                track.items.insert(index, *item);
                Some(ItemAddress::Video {
                    sequence_path,
                    track_id: track.id,
                    item_id,
                })
            }
            (TrackMut::Audio(track), ProjectItem::Audio(item)) => {
                let index = track
                    .items
                    .partition_point(|candidate| candidate.start <= item.start);
                track.items.insert(index, *item);
                Some(ItemAddress::Audio {
                    sequence_path,
                    track_id: track.id,
                    item_id,
                })
            }
            _ => None,
        }
    }

    pub fn insert_item_on_new_track(
        &mut self,
        sequence_path: &[Uuid],
        item: ProjectItem,
    ) -> Option<ItemAddress> {
        self.insert_item_on_new_track_at(sequence_path, item, false)
    }

    fn insert_item_on_new_track_at(
        &mut self,
        sequence_path: &[Uuid],
        item: ProjectItem,
        top: bool,
    ) -> Option<ItemAddress> {
        match item {
            ProjectItem::Comment(item) if sequence_path.is_empty() => {
                let track = CommentTrack {
                    items: vec![item],
                    ..Default::default()
                };
                let address = ItemAddress::Comment {
                    track_id: track.id,
                    item_id: track.items[0].id,
                };
                self.comment_tracks.push(track);
                Some(address)
            }
            ProjectItem::Caption(item) if sequence_path.is_empty() => {
                let track = CaptionTrack {
                    items: vec![item],
                    ..Default::default()
                };
                let address = ItemAddress::Caption {
                    track_id: track.id,
                    item_id: track.items[0].id,
                };
                self.caption_tracks.push(track);
                Some(address)
            }
            ProjectItem::Video(item) => {
                let sequence_id = self.video_sequence_for_path(sequence_path)?;
                let track = VisualTrack {
                    items: vec![*item],
                    ..Default::default()
                };
                let address = ItemAddress::Video {
                    sequence_path: sequence_path.to_vec(),
                    track_id: track.id,
                    item_id: track.items[0].id,
                };
                let tracks = match sequence_id {
                    Some(sequence_id) => &mut self.folded_sequence_mut(sequence_id)?.video_tracks,
                    None => &mut self.video_tracks,
                };
                tracks.push(track);
                Some(address)
            }
            ProjectItem::Audio(item) => {
                let sequence_id = self.audio_sequence_for_path(sequence_path)?;
                let track = AudioTrack {
                    items: vec![*item],
                    ..Default::default()
                };
                let address = ItemAddress::Audio {
                    sequence_path: sequence_path.to_vec(),
                    track_id: track.id,
                    item_id: track.items[0].id,
                };
                let tracks = match sequence_id {
                    Some(sequence_id) => &mut self.folded_sequence_mut(sequence_id)?.audio_tracks,
                    None => &mut self.audio_tracks,
                };
                if top {
                    tracks.insert(0, track);
                } else {
                    tracks.push(track);
                }
                Some(address)
            }
            ProjectItem::Comment(_) => None,
            ProjectItem::Caption(_) => None,
        }
    }

    pub fn move_item(
        &mut self,
        source: &ItemAddress,
        target: &TrackAddress,
        start: Time,
        end: Time,
    ) -> Option<ItemAddress> {
        if !self.can_move_item(source, target, start, end) {
            return None;
        }
        let mut item = self.take_item(source)?;
        item.set_times(start, end);
        Some(
            self.insert_item(target, item)
                .expect("a prevalidated item insertion must succeed"),
        )
    }

    pub fn can_move_item(
        &self,
        source: &ItemAddress,
        target: &TrackAddress,
        start: Time,
        end: Time,
    ) -> bool {
        if start >= end
            || !self.can_insert_item(target, source.kind())
            || !self.can_move_item_to_sequence_path(source, target.sequence_path())
        {
            return false;
        }
        let source_id = source.item_id();
        let Some(target) = self.track(target) else {
            return false;
        };
        !match target {
            TrackRef::Comment(track) => track
                .items
                .iter()
                .any(|item| item.id != source_id && item.start < end && item.end > start),
            TrackRef::Caption(track) => track
                .items
                .iter()
                .any(|item| item.id != source_id && item.start < end && item.end > start),
            TrackRef::Video(track) => track
                .items
                .iter()
                .any(|item| item.id != source_id && item.start < end && item.end > start),
            TrackRef::Audio(track) => track
                .items
                .iter()
                .any(|item| item.id != source_id && item.start < end && item.end > start),
        }
    }

    pub fn move_item_to_new_track(
        &mut self,
        source: &ItemAddress,
        sequence_path: &[Uuid],
        start: Time,
        end: Time,
    ) -> Option<ItemAddress> {
        if sequence_path.contains(&source.item_id())
            || !self.can_move_item_to_sequence_path(source, sequence_path)
        {
            return None;
        }
        let valid_target = match source.kind() {
            ItemKind::Comment => sequence_path.is_empty(),
            ItemKind::Caption => sequence_path.is_empty(),
            ItemKind::Video => self.video_sequence_for_path(sequence_path).is_some(),
            ItemKind::Audio => self.audio_sequence_for_path(sequence_path).is_some(),
        };
        if !valid_target {
            return None;
        }

        let mut item = self.take_item(source)?;
        item.set_times(start, end);
        Some(
            self.insert_item_on_new_track(sequence_path, item)
                .expect("a prevalidated new-track insertion must succeed"),
        )
    }

    pub fn move_item_to_new_top_track(
        &mut self,
        source: &ItemAddress,
        sequence_path: &[Uuid],
        start: Time,
        end: Time,
    ) -> Option<ItemAddress> {
        if start >= end
            || sequence_path.contains(&source.item_id())
            || !self.can_move_item_to_sequence_path(source, sequence_path)
        {
            return None;
        }
        match source.kind() {
            ItemKind::Comment if !sequence_path.is_empty() => return None,
            ItemKind::Caption if !sequence_path.is_empty() => return None,
            ItemKind::Video if self.video_sequence_for_path(sequence_path).is_none() => {
                return None;
            }
            ItemKind::Audio if self.audio_sequence_for_path(sequence_path).is_none() => {
                return None;
            }
            ItemKind::Comment | ItemKind::Caption | ItemKind::Video | ItemKind::Audio => {}
        }
        let mut item = self.take_item(source)?;
        item.set_times(start, end);
        Some(
            self.insert_item_on_new_track_at(sequence_path, item, true)
                .expect("a prevalidated top-track insertion must succeed"),
        )
    }

    pub fn remove_track(&mut self, address: &TrackAddress) -> bool {
        match address {
            TrackAddress::Comment { track_id } => {
                let old_len = self.comment_tracks.len();
                self.comment_tracks.retain(|track| track.id != *track_id);
                self.comment_tracks.len() != old_len
            }
            TrackAddress::Caption { track_id } => {
                let old_len = self.caption_tracks.len();
                self.caption_tracks.retain(|track| track.id != *track_id);
                self.caption_tracks.len() != old_len
            }
            TrackAddress::Video {
                sequence_path,
                track_id,
            } => {
                let Some(sequence_id) = self.video_sequence_for_path(sequence_path) else {
                    return false;
                };
                let tracks = match sequence_id {
                    Some(sequence_id) => {
                        let Some(sequence) = self.folded_sequence_mut(sequence_id) else {
                            return false;
                        };
                        &mut sequence.video_tracks
                    }
                    None => &mut self.video_tracks,
                };
                let old_len = tracks.len();
                tracks.retain(|track| track.id != *track_id);
                tracks.len() != old_len
            }
            TrackAddress::Audio {
                sequence_path,
                track_id,
            } => {
                let Some(sequence_id) = self.audio_sequence_for_path(sequence_path) else {
                    return false;
                };
                let tracks = match sequence_id {
                    Some(sequence_id) => {
                        let Some(sequence) = self.folded_sequence_mut(sequence_id) else {
                            return false;
                        };
                        &mut sequence.audio_tracks
                    }
                    None => &mut self.audio_tracks,
                };
                let old_len = tracks.len();
                tracks.retain(|track| track.id != *track_id);
                tracks.len() != old_len
            }
        }
    }

    pub fn sequence_time_to_timeline(&self, track: &TrackAddress, mut time: Time) -> Option<Time> {
        self.track(track)?;
        let hosts = self.sequence_hosts(track)?;
        for host in hosts.iter().rev() {
            if playback_speed_is_zero(host.speed) {
                return None;
            }
            time = host.start.saturating_add(unscaled_time_delta(
                time.signed_sub(host.offset),
                host.speed,
            ));
        }
        Some(time)
    }

    pub fn timeline_time_to_sequence(&self, track: &TrackAddress, mut time: Time) -> Option<Time> {
        self.track(track)?;
        for host in self.sequence_hosts(track)? {
            if playback_speed_is_zero(host.speed) {
                return None;
            }
            time = host
                .offset
                .saturating_add(scaled_time_delta(time.signed_sub(host.start), host.speed));
        }
        Some(time)
    }

    pub fn keyframe_time(&self, address: &ItemAddress, timeline_time: Time) -> Option<Time> {
        let timeline_time = timeline_time.snapped(self.frame_step());
        let sequence_time = self.timeline_time_to_sequence(&address.track(), timeline_time)?;
        if let Some(item) = self.video_item(address) {
            return Some(super::generated_item_animation_time(item, sequence_time));
        }
        Some(sequence_time.signed_sub(self.audio_item(address)?.start))
    }

    pub fn keyframe_timeline_time(
        &self,
        address: &ItemAddress,
        keyframe_time: Time,
    ) -> Option<Time> {
        let sequence_time = if let Some(item) = self.video_item(address) {
            item.start
                .saturating_add(keyframe_time.signed_sub(item.animation_time_offset))
        } else {
            self.audio_item(address)?
                .start
                .saturating_add(keyframe_time)
        };
        self.sequence_time_to_timeline(&address.track(), sequence_time)
    }

    pub fn keyframe_step(&self, address: &ItemAddress) -> Option<Time> {
        Some(
            self.keyframe_time(address, Time::ZERO)?
                .abs_diff(self.keyframe_time(address, self.frame_step())?),
        )
    }

    pub fn timeline_time_to_sequence_path(
        &self,
        kind: ItemKind,
        sequence_path: &[Uuid],
        mut time: Time,
    ) -> Option<Time> {
        for host in self.sequence_hosts_for_path(kind, sequence_path)? {
            if playback_speed_is_zero(host.speed) {
                return None;
            }
            time = host
                .offset
                .saturating_add(scaled_time_delta(time.signed_sub(host.start), host.speed));
        }
        Some(time)
    }

    pub fn projected_item_times(&self, address: &ItemAddress) -> Option<(Time, Time)> {
        let (mut start, mut end) = self.timeline_item_times(address)?;
        let track = address.track();
        let hosts = self.sequence_hosts(&track)?;

        for (index, host) in hosts.iter().enumerate() {
            let mut host_start = map_sequence_time(&hosts[..index], host.start)?;
            let mut host_end = map_sequence_time(&hosts[..index], host.end)?;
            if host_start > host_end {
                std::mem::swap(&mut host_start, &mut host_end);
            }
            start = start.max(host_start);
            end = end.min(host_end);
        }
        (end > start).then_some((start, end))
    }

    pub fn timeline_item_times(&self, address: &ItemAddress) -> Option<(Time, Time)> {
        let (start, end) = self.item(address)?.times();
        let track = address.track();
        let start = self.sequence_time_to_timeline(&track, start)?;
        let end = self.sequence_time_to_timeline(&track, end)?;
        Some((start.min(end), start.max(end)))
    }

    fn video_sequence_for_path(&self, path: &[Uuid]) -> Option<Option<Uuid>> {
        let mut tracks = self.video_tracks.as_slice();
        let mut sequence_id = None;
        for host_id in path {
            let host = tracks
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| item.id == *host_id)?;
            let VideoItemContent::FoldedSequence(reference) = host.content else {
                return None;
            };
            sequence_id = Some(reference.sequence_id);
            tracks = &self.folded_sequence(reference.sequence_id)?.video_tracks;
        }
        Some(sequence_id)
    }

    fn audio_sequence_for_path(&self, path: &[Uuid]) -> Option<Option<Uuid>> {
        let mut tracks = self.audio_tracks.as_slice();
        let mut sequence_id = None;
        for host_id in path {
            let host = tracks
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| item.id == *host_id)?;
            let AudioSource::FoldedSequence(reference) = &host.source else {
                return None;
            };
            sequence_id = Some(reference.sequence_id);
            tracks = &self.folded_sequence(reference.sequence_id)?.audio_tracks;
        }
        Some(sequence_id)
    }

    pub fn video_tracks_for_path(&self, path: &[Uuid]) -> Option<&[VisualTrack]> {
        match self.video_sequence_for_path(path)? {
            Some(sequence_id) => Some(&self.folded_sequence(sequence_id)?.video_tracks),
            None => Some(&self.video_tracks),
        }
    }

    pub fn audio_tracks_for_path(&self, path: &[Uuid]) -> Option<&[AudioTrack]> {
        match self.audio_sequence_for_path(path)? {
            Some(sequence_id) => Some(&self.folded_sequence(sequence_id)?.audio_tracks),
            None => Some(&self.audio_tracks),
        }
    }

    fn sequence_hosts(&self, track: &TrackAddress) -> Option<Vec<SequenceHostTime>> {
        self.sequence_hosts_for_path(track.kind(), track.sequence_path())
    }

    fn sequence_hosts_for_path(
        &self,
        kind: ItemKind,
        sequence_path: &[Uuid],
    ) -> Option<Vec<SequenceHostTime>> {
        match kind {
            ItemKind::Comment => sequence_path.is_empty().then(Vec::new),
            ItemKind::Caption => sequence_path.is_empty().then(Vec::new),
            ItemKind::Video => {
                let mut tracks = self.video_tracks.as_slice();
                let mut hosts = Vec::with_capacity(sequence_path.len());
                for host_id in sequence_path {
                    let host = tracks
                        .iter()
                        .flat_map(|track| &track.items)
                        .find(|item| item.id == *host_id)?;
                    let VideoItemContent::FoldedSequence(reference) = host.content else {
                        return None;
                    };
                    hosts.push(SequenceHostTime {
                        start: host.start,
                        end: host.end,
                        offset: host.time_offset,
                        speed: host.playback_speed,
                    });
                    tracks = &self.folded_sequence(reference.sequence_id)?.video_tracks;
                }
                Some(hosts)
            }
            ItemKind::Audio => {
                let mut tracks = self.audio_tracks.as_slice();
                let mut hosts = Vec::with_capacity(sequence_path.len());
                for host_id in sequence_path {
                    let host = tracks
                        .iter()
                        .flat_map(|track| &track.items)
                        .find(|item| item.id == *host_id)?;
                    let AudioSource::FoldedSequence(reference) = &host.source else {
                        return None;
                    };
                    hosts.push(SequenceHostTime {
                        start: host.start,
                        end: host.end,
                        offset: host.time_offset,
                        speed: host.playback_speed,
                    });
                    tracks = &self.folded_sequence(reference.sequence_id)?.audio_tracks;
                }
                Some(hosts)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SequenceHostTime {
    start: Time,
    end: Time,
    offset: Time,
    speed: Fraction,
}

fn map_sequence_time(hosts: &[SequenceHostTime], mut time: Time) -> Option<Time> {
    for host in hosts.iter().rev() {
        if playback_speed_is_zero(host.speed) {
            return None;
        }
        time = host.start.saturating_add(unscaled_time_delta(
            time.signed_sub(host.offset),
            host.speed,
        ));
    }
    Some(time)
}
