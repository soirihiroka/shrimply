use super::*;
use shrimply_project::project::{ItemAddress, VideoItem, VisualTrack};
use shrimply_video_core::clip_transition::{ActiveClipTransition, held_item};
use std::borrow::Cow;

pub(super) struct PreparedItem<'a> {
    pub item: Cow<'a, VideoItem>,
    pub address: ItemAddress,
    pub time: Time,
    pub clip_transition: Option<ActiveClipTransition>,
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
        let mut items = Vec::new();
        for active in shrimply_video_core::sequence::active_tracks(tracks, scope.time, None) {
            if Some(active.item.id) == self.excluded_item_id {
                continue;
            }
            let item = held_item(active.item, scope.time, active.clip_transition.is_some());
            if !resolve_bool(
                &item.visibility,
                &VisualEvaluation::for_item_with_audio(project, &item, scope.time, audio),
                &mut self.expressions,
            ) {
                continue;
            }
            let source_time = match item.content {
                VideoItemContent::Media | VideoItemContent::Gif => {
                    let Some(time) = video_source_time_at(&item, scope.time) else {
                        continue;
                    };
                    time
                }
                VideoItemContent::Background(_)
                | VideoItemContent::Shape(_)
                | VideoItemContent::Text(_)
                | VideoItemContent::Paint(_) => {
                    if shrimply_project::project::generated_item_time(&item, scope.time).is_none() {
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
                    scope.time,
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
                    audio,
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
                time: scope.time,
                clip_transition: active.clip_transition,
                children,
            });
        }
        Ok(items)
    }
}
