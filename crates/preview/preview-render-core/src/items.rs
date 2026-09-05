use super::*;
use shrimply_project::project::{ItemAddress, VideoItem, VisualTrack};
use shrimply_video_core::clip_transition::{ActiveClipTransition, held_item};
use std::borrow::Cow;

pub(super) struct PreparedItem<'a> {
    pub item: Cow<'a, VideoItem>,
    pub address: ItemAddress,
    pub time: Time,
    pub audio: Option<FrameAudioAnalysis>,
    pub clip_transition: Option<ActiveClipTransition>,
    pub morph_peer: Option<uuid::Uuid>,
    pub children: Option<Vec<PreparedItem<'a>>>,
}

#[derive(Default)]
pub(super) struct Scope {
    pub time: Time,
    pub path: Vec<uuid::Uuid>,
    pub ancestors: Vec<uuid::Uuid>,
}

impl Scene {
    pub(super) fn items<'a>(
        &mut self,
        project: &'a Project,
        tracks: &'a [VisualTrack],
        audio: &FrameAudioAnalysis,
        scope: &Scope,
        requests: &mut Vec<media::Request>,
    ) -> Result<Vec<PreparedItem<'a>>, String> {
        let mut active_items =
            shrimply_video_core::sequence::active_tracks(tracks, scope.time, None);
        active_items.retain(|active| Some(active.item.id) != self.excluded_item_id);
        let mut items = Vec::new();
        for (active_index, active) in active_items.iter().enumerate() {
            let morph = morph_endpoint(&active_items, active_index);
            let endpoint_time = morph.map(|(_, time)| time);
            let item_time = endpoint_time.unwrap_or(scope.time);
            let endpoint_audio = endpoint_time.map(|time| {
                let audio = self
                    .audio_sampler
                    .sample(project, time, self.audio_revision);
                self.sampled_audio.push(audio.clone());
                audio
            });
            let item_audio = endpoint_audio.as_ref().unwrap_or(audio);
            let item = if endpoint_time.is_some() {
                Cow::Borrowed(active.item)
            } else {
                held_item(active.item, scope.time, active.clip_transition.is_some())
            };
            if !resolve_bool(
                &item.visibility,
                &VisualEvaluation::for_item_with_audio(project, &item, item_time, item_audio),
                &mut self.expressions,
            ) {
                continue;
            }
            let source_time = match item.content {
                VideoItemContent::Media | VideoItemContent::Gif => {
                    let Some(time) = video_source_time_at(&item, item_time) else {
                        continue;
                    };
                    time
                }
                VideoItemContent::Background(_)
                | VideoItemContent::Shape(_)
                | VideoItemContent::Text(_)
                | VideoItemContent::Paint(_) => {
                    if shrimply_project::project::generated_item_time(&item, item_time).is_none() {
                        continue;
                    }
                    Time::ZERO
                }
                _ => Time::ZERO,
            };
            let address = ItemAddress::Video {
                sequence_path: scope.path.clone(),
                track_id: active.track_id,
                item_id: item.id,
            };
            let children = if let VideoItemContent::FoldedSequence(reference) = item.content {
                let Some((sequence, time)) = shrimply_video_core::sequence::resolve(
                    project,
                    &item,
                    reference,
                    item_time,
                    &scope.ancestors,
                )?
                else {
                    continue;
                };
                let mut child_scope = Scope {
                    time,
                    path: scope.path.clone(),
                    ancestors: scope.ancestors.clone(),
                };
                child_scope.path.push(item.id);
                child_scope.ancestors.push(reference.sequence_id);
                let children = self.items(
                    project,
                    &sequence.video_tracks,
                    item_audio,
                    &child_scope,
                    requests,
                )?;
                if children.is_empty() {
                    continue;
                }
                Some(children)
            } else {
                if let Some(request) = media::Request::new(
                    &item,
                    address.clone(),
                    source_time,
                    self.requested_accuracy,
                ) {
                    requests.push(request);
                }
                None
            };
            items.push(PreparedItem {
                item,
                address,
                time: item_time,
                audio: endpoint_audio,
                clip_transition: active.clip_transition,
                morph_peer: morph.map(|(peer, _)| peer),
                children,
            });
        }
        let prepared_ids = items
            .iter()
            .map(|prepared| prepared.address.item_id())
            .collect::<std::collections::HashSet<_>>();
        items.retain(|prepared| {
            prepared
                .morph_peer
                .is_none_or(|peer| prepared_ids.contains(&peer))
        });
        Ok(items)
    }
}

fn morph_endpoint(
    items: &[shrimply_video_core::sequence::ActiveVideoItem<'_>],
    index: usize,
) -> Option<(uuid::Uuid, Time)> {
    use shrimply_project::project::VisualClipTransitionKind;
    use shrimply_video_core::clip_transition::ClipTransitionRole;
    let active = &items[index];
    let transition = active
        .clip_transition
        .filter(|transition| transition.definition.kind == VisualClipTransitionKind::Morph)?;
    let (peer, cut) = match transition.role {
        ClipTransitionRole::Outgoing => {
            let peer = items[index + 1..].iter().find(|candidate| {
                candidate.track_id == active.track_id
                    && candidate
                        .clip_transition
                        .is_some_and(|candidate_transition| {
                            candidate_transition.definition.kind == VisualClipTransitionKind::Morph
                                && candidate_transition.role == ClipTransitionRole::Incoming
                                && candidate_transition.progress == transition.progress
                        })
            })?;
            (peer.item.id, active.item.end)
        }
        ClipTransitionRole::Incoming => {
            let peer = items[..index].iter().rev().find(|candidate| {
                candidate.track_id == active.track_id
                    && candidate
                        .clip_transition
                        .is_some_and(|candidate_transition| {
                            candidate_transition.definition.kind == VisualClipTransitionKind::Morph
                                && candidate_transition.role == ClipTransitionRole::Outgoing
                                && candidate_transition.progress == transition.progress
                        })
            })?;
            (peer.item.id, peer.item.end)
        }
    };
    let (source, target) =
        shrimply_math_media::clip_transition_bounds(cut, transition.definition.duration);
    Some((
        peer,
        match transition.role {
            ClipTransitionRole::Outgoing => source,
            ClipTransitionRole::Incoming => target,
        },
    ))
}
