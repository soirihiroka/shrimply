use crate::{
    DragCollisionMode, import,
    items::NewItemTarget,
    project::{self, Project, Time},
};
use shrimply_resource_pipeline::{Event, Subscription, TryNext};
use shrimply_timeline::{TrackKey, TrackKind, selection_state};
use std::{collections::VecDeque, path::PathBuf};

#[derive(Clone, Copy)]
pub struct Placement {
    pub start: Time,
    pub target: NewItemTarget,
    pub collision: DragCollisionMode,
}

struct Pending {
    path: PathBuf,
    batch: u64,
    target: Target,
    placement: Placement,
    inspection: Subscription<import::InspectionKey, (), import::MediaInfo>,
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
    next_batch: u64,
}

impl ImportQueue {
    pub fn enqueue(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        placement: Placement,
        default_duration: Time,
    ) -> Result<(), String> {
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
        let batch = self.next_batch;
        self.next_batch = self
            .next_batch
            .checked_add(1)
            .expect("import batch counter overflow");
        self.pending.extend(paths.into_iter().map(|path| Pending {
            batch,
            target: Target::Timeline(track.clone()),

            inspection: import::request_inspection(
                path.clone(),
                project.canvas_size,
                default_duration,
            ),
            path,
            placement,
        }));
        Ok(())
    }

    pub fn enqueue_tracks(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        keys: &[TrackKey],
        start: Time,
        default_duration: Time,
    ) -> Result<(), String> {
        let kind = keys.first().ok_or("no import tracks were selected")?.kind;
        let mut tracks = Vec::new();
        for key in keys {
            if key.kind != kind {
                return Err("import tracks must have the same kind".into());
            }
            let address = selection_state::track_address(project, *key)
                .ok_or("import destination track no longer exists")?;
            if !tracks.contains(&address) {
                tracks.push(address);
            }
        }
        let paths: Vec<_> = paths.into_iter().collect();
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
        let batch = self.next_batch;
        self.next_batch = self
            .next_batch
            .checked_add(1)
            .expect("import batch counter overflow");
        self.pending.extend(paths.into_iter().map(|path| Pending {
            batch,
            target: Target::Tracks(tracks.clone()),
            inspection: import::request_inspection(
                path.clone(),
                project.canvas_size,
                default_duration,
            ),
            path,
            placement: Placement {
                start,
                target: NewItemTarget::Automatic,
                collision: DragCollisionMode::NewTrack,
            },
        }));
        Ok(())
    }

    pub fn poll(
        &mut self,
        project: &mut Project,
    ) -> Option<Result<(import::ImportResult, Time), String>> {
        loop {
            let pending = self.pending.front_mut()?;
            match pending.inspection.try_next() {
                TryNext::Empty => return None,
                TryNext::Event(Event::Progress(_)) => continue,
                TryNext::Event(Event::Finished(info)) => {
                    let pending = self.pending.pop_front().expect("pending import exists");
                    return Some(
                        (|| {
                            info.snapshot.ensure_current()?;
                            if info.video_streams == 0
                                && info.audio_streams == 0
                                && info.caption_cues.is_empty()
                            {
                                return Err(
                                    "file contains no importable audio or video stream".into()
                                );
                            }
                            let mut candidate = project.clone();
                            let (imported, end) = match &pending.target {
                                Target::Timeline(track) => {
                                    if !info.caption_cues.is_empty() {
                                        return Err("Use a caption track's import button to import VTT files".into());
                                    }
                                    let target = if let Some(track) = track {
                                        let row = crate::items::row_for_address(&candidate, track)
                                            .ok_or("drop destination track was removed while inspecting media")?;
                                        NewItemTarget::AtY(crate::drawing::row_y(row))
                                    } else {
                                        NewItemTarget::Automatic
                                    };
                                    let preview = import::preview(
                                        &candidate,
                                        info.duration,
                                        info.video_streams,
                                        info.audio_streams,
                                        pending.placement.start,
                                        target,
                                        pending.placement.collision,
                                    );
                                    let imported = import::apply(&mut candidate, &info, &preview);
                                    (imported, preview.end)
                                }
                                Target::Tracks(tracks) => {
                                    let keys = tracks.iter().map(|address| {
                                        selection_state::track_key(&candidate, address)
                                            .ok_or("import destination track was removed while inspecting media")
                                    }).collect::<Result<Vec<_>, _>>()?;
                                    let kind = keys.first().expect("validated import tracks").kind;
                                    let indices = keys.iter().map(|key| key.track_index).collect::<Vec<_>>();
                                    let imported = if kind == TrackKind::Caption {
                                        import::apply_vtt_cues_to_tracks(
                                            &mut candidate,
                                            &info.caption_cues,
                                            &indices,
                                            pending.placement.start,
                                        )?
                                    } else {
                                        import::apply_media_to_tracks(
                                            &mut candidate,
                                            &info,
                                            kind,
                                            &indices,
                                            pending.placement.start,
                                        )?
                                    };
                                    let step = candidate.frame_step();
                                    let start = pending.placement.start.max(Time::ZERO).snapped(step);
                                    let end = start.saturating_add(info.duration)
                                        .snapped(step).max(start.saturating_add(step));
                                    (imported, end)
                                }
                            };
                            project::commit_edit_checked(&candidate, "import-media")?;
                            let duration = candidate.duration();
                            *project = candidate;
                            for next in self
                                .pending
                                .iter_mut()
                                .take_while(|next| next.batch == pending.batch)
                            {
                                next.placement.start = end;
                            }

                            Ok((imported, duration))
                        })()
                        .map_err(|error: String| format!("{}: {error}", pending.path.display())),
                    );
                }
                event => {
                    let pending = self.pending.pop_front().expect("pending import exists");
                    let error = match event {
                        TryNext::Event(Event::Failed(error)) => error.to_string(),
                        TryNext::Event(Event::Cancelled) => "media inspection was cancelled".into(),
                        TryNext::Closed => "media inspection worker stopped unexpectedly".into(),
                        _ => unreachable!("handled nonterminal event"),
                    };
                    return Some(Err(format!("{}: {error}", pending.path.display())));
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
