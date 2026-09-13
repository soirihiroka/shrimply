use crate::{
    DragCollisionMode, import,
    items::NewItemTarget,
    project::{self, Project, Time},
};
use shrimply_resource_pipeline::{Event, Subscription, TryNext};
use shrimply_timeline_edit::{TrackKey, TrackKind, selection_state};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_BATCH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BatchId(u64);

pub struct Completion {
    pub batch: BatchId,
    pub paths: Vec<PathBuf>,
    pub result: Result<(import::ImportResult, Time), String>,
}

#[derive(Clone, Copy)]
pub struct Placement {
    pub start: Time,
    pub target: NewItemTarget,
    pub collision: DragCollisionMode,
}

struct Pending {
    path: PathBuf,
    batch: BatchId,
    target: Target,
    placement: Placement,
    inspection: Subscription<import::InspectionKey, (), import::MediaInfo>,
    info: Option<std::sync::Arc<import::MediaInfo>>,
}

#[derive(Clone)]
enum Target {
    Timeline(Option<project::TrackAddress>),
    Tracks(Vec<project::TrackAddress>),
}

/// Inspects files on the existing media worker pool and applies results in drop order.
#[derive(Default)]
pub struct ImportQueue {
    pending: VecDeque<Pending>,
}

impl ImportQueue {
    pub fn enqueue(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        placement: Placement,
        default_duration: Time,
    ) -> Result<BatchId, String> {
        let paths: Vec<_> = paths.into_iter().collect();
        if crate::external_content::external_files_need_remux(&paths)? {
            return Err("MKV and WebM must be remuxed before timeline import".into());
        }
        let track = match placement.target {
            NewItemTarget::Automatic => None,
            NewItemTarget::AtY(y) => {
                let rows = crate::items::track_rows(project);
                let row = crate::math::track_row_at_y(y).and_then(|index| rows.get(index));
                if row.is_some_and(|row| row.root_key.is_none()) {
                    return Err("Import into an expanded nested track is not supported yet. Drop onto a top-level track.".into());
                }
                row.map(|row| row.address.clone())
            }
        };
        let batch = self.reserve_batch();
        self.enqueue_batch(batch, paths, project, placement, default_duration, track)?;
        Ok(batch)
    }

    fn enqueue_batch(
        &mut self,
        batch: BatchId,
        paths: Vec<PathBuf>,
        project: &Project,
        placement: Placement,
        default_duration: Time,
        track: Option<project::TrackAddress>,
    ) -> Result<(), String> {
        self.pending.extend(paths.into_iter().map(|path| Pending {
            batch,
            target: Target::Timeline(track.clone()),

            inspection: import::request_inspection(
                path.clone(),
                project.canvas_size,
                default_duration,
            ),
            info: None,
            path,
            placement,
        }));
        Ok(())
    }

    pub(crate) fn reserve_batch(&mut self) -> BatchId {
        BatchId(
            NEXT_BATCH
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |batch| {
                    batch.checked_add(1)
                })
                .expect("import batch counter overflow"),
        )
    }

    pub(crate) fn enqueue_reserved(
        &mut self,
        batch: BatchId,
        paths: Vec<PathBuf>,
        project: &Project,
        placement: Placement,
        default_duration: Time,
    ) -> Result<(), String> {
        if crate::external_content::external_files_need_remux(&paths)? {
            return Err("MKV and WebM must be remuxed before timeline import".into());
        }
        let track = match placement.target {
            NewItemTarget::Automatic => None,
            NewItemTarget::AtY(y) => {
                let rows = crate::items::track_rows(project);
                let row = crate::math::track_row_at_y(y).and_then(|index| rows.get(index));
                if row.is_some_and(|row| row.root_key.is_none()) {
                    return Err("Import into an expanded nested track is not supported yet. Drop onto a top-level track.".into());
                }
                row.map(|row| row.address.clone())
            }
        };
        self.enqueue_batch(batch, paths, project, placement, default_duration, track)
    }

    pub fn enqueue_tracks(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        keys: &[TrackKey],
        start: Time,
        default_duration: Time,
    ) -> Result<BatchId, String> {
        let mut tracks = Vec::new();
        for key in keys {
            let address = selection_state::track_address(project, *key)
                .ok_or("import destination track no longer exists")?;
            if !tracks.contains(&address) {
                tracks.push(address);
            }
        }
        let batch = self.reserve_batch();
        self.enqueue_track_addresses_reserved(
            batch,
            paths.into_iter().collect(),
            project,
            tracks,
            start,
            default_duration,
        )?;
        Ok(batch)
    }

    pub(crate) fn enqueue_track_addresses_reserved(
        &mut self,
        batch: BatchId,
        paths: Vec<PathBuf>,
        project: &Project,
        tracks: Vec<project::TrackAddress>,
        start: Time,
        default_duration: Time,
    ) -> Result<(), String> {
        let keys = tracks
            .iter()
            .map(|address| {
                selection_state::track_key(project, address)
                    .ok_or("import destination track no longer exists")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let kind = keys.first().ok_or("no import tracks were selected")?.kind;
        if keys.iter().any(|key| key.kind != kind) {
            return Err("import tracks must have the same kind".into());
        }
        if paths.is_empty() {
            return Err("no import files were selected".into());
        }
        for path in &paths {
            let file_kind = import::file_kind(path).ok_or("unsupported file type")?;
            if kind == TrackKind::Caption && file_kind != import::FileKind::Vtt {
                return Err("only VTT files can be imported to caption tracks".into());
            }
            if kind != TrackKind::Caption && file_kind == import::FileKind::Vtt {
                return Err("VTT files can only be imported to caption tracks".into());
            }
            if kind != TrackKind::Caption && !import::direct_media_kind(file_kind) {
                return Err("MKV and WebM need to be remuxed before track import".into());
            }
        }
        self.pending.extend(paths.into_iter().map(|path| Pending {
            batch,
            target: Target::Tracks(tracks.clone()),
            inspection: import::request_inspection(
                path.clone(),
                project.canvas_size,
                default_duration,
            ),
            info: None,
            path,
            placement: Placement {
                start,
                target: NewItemTarget::Automatic,
                collision: DragCollisionMode::NewTrack,
            },
        }));
        Ok(())
    }

    pub fn poll(&mut self, project: &mut Project) -> Option<Completion> {
        let batch = self.pending.front()?.batch;
        let batch_len = self
            .pending
            .iter()
            .take_while(|pending| pending.batch == batch)
            .count();
        for index in 0..batch_len {
            let pending = self.pending.get_mut(index).expect("batch item exists");
            if pending.info.is_some() {
                continue;
            }
            loop {
                match pending.inspection.try_next() {
                    TryNext::Empty => return None,
                    TryNext::Event(Event::Progress(_)) => continue,
                    TryNext::Event(Event::Finished(info)) => {
                        pending.info = Some(info);
                        break;
                    }
                    event => {
                        let path = pending.path.clone();
                        let error = match event {
                            TryNext::Event(Event::Failed(error)) => error.to_string(),
                            TryNext::Event(Event::Cancelled) => {
                                "media inspection was cancelled".into()
                            }
                            TryNext::Closed => {
                                "media inspection worker stopped unexpectedly".into()
                            }
                            _ => unreachable!("handled nonterminal event"),
                        };
                        let paths = self
                            .pending
                            .drain(..batch_len)
                            .map(|pending| pending.path)
                            .collect();
                        return Some(Completion {
                            batch,
                            paths,
                            result: Err(format!("{}: {error}", path.display())),
                        });
                    }
                }
            }
        }

        let pending: Vec<_> = self.pending.drain(..batch_len).collect();
        let paths = pending.iter().map(|pending| pending.path.clone()).collect();
        Some(Completion {
            batch,
            paths,
            result: (|| {
                let mut candidate = project.clone();
                let mut imported = import::ImportResult {
                    selection: Vec::new(),
                    video: false,
                    audio: false,
                    captions: false,
                };
                let mut start = pending
                    .first()
                    .expect("completed import batch is not empty")
                    .placement
                    .start;
                for mut item in pending {
                    item.placement.start = start;
                    let info = item
                        .info
                        .take()
                        .expect("completed inspection has media info");
                    let (next, end) = apply_pending(&mut candidate, &item, &info)
                        .map_err(|error| format!("{}: {error}", item.path.display()))?;
                    imported.selection.extend(next.selection);
                    imported.video |= next.video;
                    imported.audio |= next.audio;
                    imported.captions |= next.captions;
                    start = end;
                }
                project::commit_edit_checked(&candidate, "import-media")?;
                let duration = candidate.duration();
                *project = candidate;
                Ok((imported, duration))
            })(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

fn apply_pending(
    project: &mut Project,
    pending: &Pending,
    info: &import::MediaInfo,
) -> Result<(import::ImportResult, Time), String> {
    info.snapshot.ensure_current()?;
    if info.video_streams == 0 && info.audio_streams == 0 && info.caption_cues.is_empty() {
        return Err("file contains no importable audio or video stream".into());
    }
    match &pending.target {
        Target::Timeline(track) => {
            if !info.caption_cues.is_empty() {
                return Err("Use a caption track's import button to import VTT files".into());
            }
            let target = if let Some(track) = track {
                let row = crate::items::row_for_address(project, track)
                    .ok_or("drop destination track was removed while inspecting media")?;
                NewItemTarget::AtY(crate::drawing::row_y(row))
            } else {
                NewItemTarget::Automatic
            };
            let preview = import::preview(
                project,
                info.duration,
                info.video_streams,
                info.audio_streams,
                pending.placement.start,
                target,
                pending.placement.collision,
            );
            Ok((import::apply(project, info, &preview), preview.end))
        }
        Target::Tracks(tracks) => {
            let keys = tracks
                .iter()
                .map(|address| {
                    selection_state::track_key(project, address)
                        .ok_or("import destination track was removed while inspecting media")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let kind = keys.first().expect("validated import tracks").kind;
            let indices = keys.iter().map(|key| key.track_index).collect::<Vec<_>>();
            let imported = if kind == TrackKind::Caption {
                import::apply_vtt_cues_to_tracks(
                    project,
                    &info.caption_cues,
                    &indices,
                    pending.placement.start,
                )?
            } else {
                import::apply_media_to_tracks(
                    project,
                    info,
                    kind,
                    &indices,
                    pending.placement.start,
                )?
            };
            let step = project.frame_step();
            let start = pending.placement.start.max(Time::ZERO).snapped(step);
            let end = start
                .saturating_add(info.duration)
                .snapped(step)
                .max(start.saturating_add(step));
            Ok((imported, end))
        }
    }
}
