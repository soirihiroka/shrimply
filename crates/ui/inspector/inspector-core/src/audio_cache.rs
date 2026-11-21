use shrimply_audio_modifiers::{CacheFormat, CacheModifier, OpusCacheQuality};

use crate::{
    CacheControlPresentation, CacheStatus, ControlKind, InspectorControl, InspectorSection,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioCachePreset {
    OpusCompact,
    OpusBalanced,
    OpusHigh,
    Flac,
}

impl AudioCachePreset {
    pub const OPTIONS: &[(Self, &'static str)] = &[
        (Self::OpusCompact, "Opus · Compact"),
        (Self::OpusBalanced, "Opus · Balanced"),
        (Self::OpusHigh, "Opus · High"),
        (Self::Flac, "FLAC · Lossless"),
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::OpusCompact => "opus_compact",
            Self::OpusBalanced => "opus_balanced",
            Self::OpusHigh => "opus_high",
            Self::Flac => "flac",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::OPTIONS
            .iter()
            .map(|(preset, _)| *preset)
            .find(|preset| preset.key() == key)
    }

    pub fn apply(self, value: &mut CacheModifier) -> bool {
        let (format, quality) = match self {
            Self::OpusCompact => (CacheFormat::Opus, OpusCacheQuality::Compact),
            Self::OpusBalanced => (CacheFormat::Opus, OpusCacheQuality::Balanced),
            Self::OpusHigh => (CacheFormat::Opus, OpusCacheQuality::High),
            Self::Flac => (CacheFormat::Flac, value.opus_quality),
        };
        if value.format == format && value.opus_quality == quality {
            return false;
        }
        value.format = format;
        value.opus_quality = quality;
        true
    }
}

pub fn audio_cache_preset(value: &CacheModifier) -> AudioCachePreset {
    match (value.format, value.opus_quality) {
        (CacheFormat::Flac, _) => AudioCachePreset::Flac,
        (CacheFormat::Opus, OpusCacheQuality::Compact) => AudioCachePreset::OpusCompact,
        (CacheFormat::Opus, OpusCacheQuality::Balanced) => AudioCachePreset::OpusBalanced,
        (CacheFormat::Opus, OpusCacheQuality::High) => AudioCachePreset::OpusHigh,
    }
}

pub fn audio_cache_status(id: uuid::Uuid) -> CacheStatus {
    match shrimply_audio_engine::modifier_cache::status(id) {
        shrimply_audio_engine::modifier_cache::Status::Missing => CacheStatus::Missing,
        shrimply_audio_engine::modifier_cache::Status::Baking { completed, total } => {
            CacheStatus::Baking { completed, total }
        }
        shrimply_audio_engine::modifier_cache::Status::Ready => CacheStatus::Ready,
        shrimply_audio_engine::modifier_cache::Status::Failed(error) => CacheStatus::Failed(error),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioCachePresentation {
    pub section: InspectorSection,
    pub status: CacheStatus,
}

pub fn audio_cache_presentation(value: &CacheModifier, id: uuid::Uuid) -> AudioCachePresentation {
    let status = audio_cache_status(id);
    let control = audio_cache_control(status.clone());
    let mut section = InspectorSection::default();
    section.add(
        InspectorControl::new(ControlKind::AudioCachePreset, "", "Format")
            .target(id)
            .value(audio_cache_preset(value).key())
            .choices(
                AudioCachePreset::OPTIONS
                    .iter()
                    .map(|(preset, _)| preset.key().to_string())
                    .collect(),
                AudioCachePreset::OPTIONS
                    .iter()
                    .map(|(_, label)| (*label).to_string())
                    .collect(),
            )
            .sensitive(!control.baking)
            .immediate_commit("audio-cache-format"),
    );
    section.add(
        InspectorControl::new(ControlKind::AudioCache, "", "")
            .target(id)
            .value(control.label)
            .components(vec![
                control.progress.to_string(),
                u8::from(control.baking).to_string(),
            ])
            .tooltip(control.tooltip),
    );
    AudioCachePresentation { section, status }
}

pub struct CacheStatusTracker<K> {
    statuses: Vec<(K, uuid::Uuid, CacheStatus)>,
}

impl<K> Default for CacheStatusTracker<K> {
    fn default() -> Self {
        Self {
            statuses: Vec::new(),
        }
    }
}

impl<K: Clone + Eq> CacheStatusTracker<K> {
    pub fn observe(&mut self, kind: K, id: uuid::Uuid, status: CacheStatus) -> CacheStatus {
        if let Some((_, _, stored)) = self
            .statuses
            .iter_mut()
            .find(|(stored_kind, stored_id, _)| *stored_kind == kind && *stored_id == id)
        {
            *stored = status.clone();
        } else {
            self.statuses.push((kind, id, status.clone()));
        }
        status
    }

    pub fn tracked(&self, kind: K, id: uuid::Uuid) -> Option<&CacheStatus> {
        self.statuses
            .iter()
            .find(|(stored_kind, stored_id, _)| *stored_kind == kind && *stored_id == id)
            .map(|(_, _, status)| status)
    }

    pub fn poll(
        &mut self,
        mut status: impl FnMut(K, uuid::Uuid) -> CacheStatus,
    ) -> CacheStatusPoll<K> {
        let mut changed = false;
        let mut finished = Vec::new();
        for (kind, id, stored) in &mut self.statuses {
            if !matches!(stored, CacheStatus::Baking { .. }) {
                continue;
            }
            let current = status(kind.clone(), *id);
            changed |= *stored != current;
            if !matches!(current, CacheStatus::Baking { .. }) && !finished.contains(kind) {
                finished.push(kind.clone());
            }
            *stored = current;
        }
        CacheStatusPoll { changed, finished }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(K, uuid::Uuid) -> bool) {
        self.statuses
            .retain(|(kind, id, _)| keep(kind.clone(), *id));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CacheStatusPoll<K> {
    pub changed: bool,
    pub finished: Vec<K>,
}

pub fn audio_cache_control(status: CacheStatus) -> CacheControlPresentation {
    crate::cache_control_presentation(status, "")
}

impl crate::InspectorController {
    pub(crate) fn poll_audio_caches(&self) -> bool {
        let project = self.project.borrow();
        let mut active = self.audio_cache_transitions.borrow_mut();
        active.retain(|target, id| {
            crate::audio_modifiers::audio_item_address(&target)
                .ok()
                .and_then(|address| project.audio_item(address))
                .is_some_and(|item| item.modifiers.iter().any(|modifier| modifier.id == id))
        });
        let poll = active.poll(|_, id| audio_cache_status(id));
        drop(active);
        drop(project);
        if !poll.finished.is_empty() {
            self.refresh_audio_cache();
        }
        poll.changed
    }

    pub fn cache_status(
        &self,
        target: &crate::InspectorTarget,
        kind: ControlKind,
        id: uuid::Uuid,
    ) -> CacheStatus {
        match kind {
            ControlKind::AudioCache => audio_cache_status(id),
            ControlKind::VisualCache => match target {
                crate::InspectorTarget::Item(address) => crate::visual_cache_status(address, id),
                _ => CacheStatus::Failed("visual cache requires a video item".into()),
            },
            _ => CacheStatus::Failed("control is not a cache".into()),
        }
    }
}
