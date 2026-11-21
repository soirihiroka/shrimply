use crate::{
    DragCollisionMode,
    external_content::OwnedFile,
    import,
    items::NewItemTarget,
    project::{self, Asset, AssetSnapshot, CanvasSize, Project, Time},
};
use shrimply_resource_pipeline::{CancelToken, Event, Subscription, TryNext};
use shrimply_timeline_edit::{TrackKey, TrackKind, selection_state};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

static NEXT_BATCH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BatchId(u64);

pub struct ImportStart {
    pub batch: BatchId,
    pub needs_remux: bool,
}

pub struct Completion {
    pub batch: BatchId,
    pub paths: Vec<PathBuf>,
    pub cancelled: bool,
    pub result: Result<(import::ImportResult, Time), String>,
}

/// Offered only after the remuxed file has been successfully imported.
/// Dropping the request keeps the original.
pub struct SourceDeletion {
    source: AssetSnapshot,
    output: AssetSnapshot,
}

impl SourceDeletion {
    pub fn source(&self) -> &std::path::Path {
        self.source.path()
    }

    pub fn delete(self) -> Result<(), String> {
        self.output.verify_current()?;
        self.source.verify_current()?;
        std::fs::remove_file(self.source.path())
            .map_err(|error| format!("Could not delete {}: {error}", self.source.path().display()))
    }
}

#[derive(Clone, Copy)]
pub struct Placement {
    pub start: Time,
    pub target: NewItemTarget,
    pub collision: DragCollisionMode,
}

/// Stable track address and exact snapped time captured when an external drop occurs.
pub struct Destination {
    target: Target,
    start: Time,
    collision: DragCollisionMode,
}

pub fn destination(project: &Project, placement: Placement) -> Result<Destination, String> {
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
    Ok(Destination {
        target: Target::Timeline(track),
        start: placement.start,
        collision: placement.collision,
    })
}

struct Inspection {
    path: PathBuf,
    subscription: Subscription<import::InspectionKey, (), import::MediaInfo>,
    info: Option<Arc<import::MediaInfo>>,
}

struct PendingBatch {
    batch: BatchId,
    target: Target,
    start: Time,
    collision: DragCollisionMode,
    phase: Phase,
}

enum Phase {
    Pending {
        sources: Vec<PathBuf>,
        canvas_size: CanvasSize,
        default_duration: Time,
    },
    Remuxing {
        sources: Vec<PathBuf>,
        job: RemuxJob,
        canvas_size: CanvasSize,
        default_duration: Time,
    },
    Inspecting {
        inspections: Vec<Inspection>,
        owned_files: Vec<OwnedFile>,
        source_deletions: Vec<SourceDeletion>,
    },
    Failed {
        sources: Vec<PathBuf>,
        error: String,
    },
}

struct RemuxJob {
    receiver: mpsc::Receiver<Result<PreparedFiles, String>>,
    cancellation: CancelToken,
}

impl Drop for RemuxJob {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

struct PreparedFiles {
    paths: Vec<PathBuf>,
    owned_files: Vec<OwnedFile>,
    source_deletions: Vec<SourceDeletion>,
}

#[derive(Clone)]
enum Target {
    Timeline(Option<project::TrackAddress>),
    Tracks(Vec<project::TrackAddress>),
}

/// Owns imports from confirmation through preparation and commit, in submission order.
#[derive(Default)]
pub struct ImportQueue {
    pending: VecDeque<PendingBatch>,
    completed: VecDeque<Completion>,
    source_deletions: VecDeque<SourceDeletion>,
}

impl Phase {
    fn new(paths: Vec<PathBuf>, canvas_size: CanvasSize, default_duration: Time) -> Self {
        if paths
            .iter()
            .any(|path| import::file_kind(path) == Some(import::FileKind::RemuxToMp4))
        {
            Self::Pending {
                sources: paths,
                canvas_size,
                default_duration,
            }
        } else {
            Self::inspect(
                PreparedFiles {
                    paths,
                    owned_files: Vec::new(),
                    source_deletions: Vec::new(),
                },
                canvas_size,
                default_duration,
            )
        }
    }

    fn inspect(prepared: PreparedFiles, canvas_size: CanvasSize, default_duration: Time) -> Self {
        Self::Inspecting {
            inspections: prepared
                .paths
                .into_iter()
                .map(|path| Inspection {
                    subscription: import::request_inspection(
                        path.clone(),
                        canvas_size,
                        default_duration,
                    ),
                    path,
                    info: None,
                })
                .collect(),
            owned_files: prepared.owned_files,
            source_deletions: prepared.source_deletions,
        }
    }

    fn poll(&mut self) -> Option<Result<(), String>> {
        match self {
            Self::Pending { .. } => return None,
            Self::Failed { error, .. } => return Some(Err(error.clone())),
            Self::Remuxing {
                job,
                canvas_size,
                default_duration,
                ..
            } => match job.receiver.try_recv() {
                Ok(Ok(prepared)) => {
                    *self = Self::inspect(prepared, *canvas_size, *default_duration);
                }
                Ok(Err(error)) => return Some(Err(error)),
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Some(Err("Media remux worker stopped unexpectedly".into()));
                }
            },
            Self::Inspecting { .. } => {}
        }
        let Self::Inspecting { inspections, .. } = self else {
            unreachable!("prepared import must be inspecting");
        };
        for inspection in inspections {
            if inspection.info.is_some() {
                continue;
            }
            loop {
                let error = match inspection.subscription.try_next() {
                    TryNext::Empty => return None,
                    TryNext::Event(Event::Progress(_)) => continue,
                    TryNext::Event(Event::Finished(info)) => {
                        if info.source.path() != inspection.path {
                            return Some(
                                Err("Media inspection returned a different source".into()),
                            );
                        }
                        inspection.info = Some(info);
                        break;
                    }
                    TryNext::Event(Event::Failed(error)) => error.to_string(),
                    TryNext::Event(Event::Cancelled) => "media inspection was cancelled".into(),
                    TryNext::Closed => "media inspection worker stopped unexpectedly".into(),
                };
                return Some(Err(format!("{}: {error}", inspection.path.display())));
            }
        }
        Some(Ok(()))
    }
}

impl ImportQueue {
    pub fn enqueue(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        placement: Placement,
        default_duration: Time,
    ) -> Result<ImportStart, String> {
        let batch = self.reserve_batch();
        self.enqueue_reserved(
            batch,
            paths.into_iter().collect(),
            project,
            placement,
            default_duration,
        )
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
    ) -> Result<ImportStart, String> {
        let destination = destination(project, placement)?;
        self.enqueue_captured(batch, paths, project, destination, default_duration)
    }

    pub fn enqueue_destination(
        &mut self,
        paths: Vec<PathBuf>,
        project: &Project,
        destination: Destination,
        default_duration: Time,
    ) -> Result<ImportStart, String> {
        let batch = self.reserve_batch();
        self.enqueue_captured(batch, paths, project, destination, default_duration)
    }

    fn enqueue_captured(
        &mut self,
        batch: BatchId,
        paths: Vec<PathBuf>,
        project: &Project,
        destination: Destination,
        default_duration: Time,
    ) -> Result<ImportStart, String> {
        crate::external_content::validate_dropped_files(&paths)?;
        let pending = PendingBatch {
            batch,
            target: destination.target,
            start: destination.start,
            collision: destination.collision,
            phase: Phase::new(paths, project.canvas_size, default_duration),
        };
        Ok(self.push(pending))
    }

    pub fn enqueue_tracks(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        project: &Project,
        keys: &[TrackKey],
        start: Time,
        default_duration: Time,
    ) -> Result<ImportStart, String> {
        let kind = keys.first().ok_or("no import tracks were selected")?.kind;
        if keys.iter().any(|key| key.kind != kind) {
            return Err("import tracks must have the same kind".into());
        }
        let mut tracks = Vec::new();
        for key in keys {
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
        }
        let batch = self.reserve_batch();
        let pending = PendingBatch {
            batch,
            target: Target::Tracks(tracks),
            start,
            collision: DragCollisionMode::NewTrack,
            phase: Phase::new(paths, project.canvas_size, default_duration),
        };
        Ok(self.push(pending))
    }

    fn push(&mut self, pending: PendingBatch) -> ImportStart {
        let needs_remux = matches!(pending.phase, Phase::Pending { .. });
        let batch = pending.batch;
        self.pending.push_back(pending);
        ImportStart { batch, needs_remux }
    }

    pub fn confirm_remux(&mut self, batch: BatchId, accepted: bool) -> Result<(), String> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.batch == batch)
            .ok_or("Remux request is no longer active")?;
        if !matches!(self.pending[index].phase, Phase::Pending { .. }) {
            return Err("Remux request has already been answered".into());
        }
        let mut pending = self.pending.remove(index).expect("pending remux exists");
        let Phase::Pending {
            sources,
            canvas_size,
            default_duration,
        } = pending.phase
        else {
            unreachable!("validated pending remux");
        };
        if !accepted {
            self.completed.push_back(Completion {
                batch,
                paths: sources,
                cancelled: true,
                result: Err("Media import was cancelled".into()),
            });
            return Ok(());
        }
        let paths = sources.clone();
        let cancellation = CancelToken::default();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("media-remux".into())
            .spawn(move || {
                let result = (|| {
                    let mut prepared = PreparedFiles {
                        paths: Vec::new(),
                        owned_files: Vec::new(),
                        source_deletions: Vec::new(),
                    };
                    for path in paths {
                        if worker_cancellation.is_cancelled() {
                            return Err("Media import was cancelled".into());
                        }
                        if import::file_kind(&path) == Some(import::FileKind::RemuxToMp4) {
                            let source = Asset::new(path.clone()).snapshot()?;
                            let output = import::remux_to_mp4(&path, worker_cancellation.clone())?;
                            source.verify_current()?;
                            prepared.source_deletions.push(SourceDeletion {
                                source,
                                output: Asset::new(output.path()).snapshot()?,
                            });
                            prepared.paths.push(output.path().to_path_buf());
                            prepared.owned_files.push(output);
                        } else {
                            prepared.paths.push(path);
                        }
                    }
                    Ok(prepared)
                })();
                let _ = sender.send(result);
            });
        pending.phase = match spawned {
            Ok(_) => Phase::Remuxing {
                sources,
                job: RemuxJob {
                    receiver,
                    cancellation,
                },
                canvas_size,
                default_duration,
            },
            Err(error) => Phase::Failed {
                sources,
                error: format!("Could not start media remux worker: {error}"),
            },
        };
        self.pending.insert(index, pending);
        Ok(())
    }

    pub fn poll(&mut self, project: &mut Project) -> Option<Completion> {
        if let Some(completion) = self.completed.pop_front() {
            return Some(completion);
        }
        let prepared = self.pending.front_mut()?.phase.poll()?;
        let PendingBatch {
            batch,
            target,
            mut start,
            collision,
            phase,
        } = self.pending.pop_front().expect("completed import exists");
        let (inspections, owned_files, source_deletions) = match phase {
            Phase::Inspecting {
                inspections,
                owned_files,
                source_deletions,
            } => (inspections, owned_files, source_deletions),
            Phase::Remuxing { sources, .. } | Phase::Failed { sources, .. } => {
                return Some(Completion {
                    batch,
                    paths: sources,
                    cancelled: false,
                    result: Err(prepared.expect_err("remux cannot complete before inspection")),
                });
            }
            Phase::Pending { .. } => unreachable!("unconfirmed import cannot complete"),
        };
        let paths = inspections
            .iter()
            .map(|inspection| inspection.path.clone())
            .collect();
        let result = prepared.and_then(|()| {
            let mut candidate = project.clone();
            let mut imported = import::ImportResult {
                selection: Vec::new(),
                video: false,
                audio: false,
                captions: false,
            };
            for inspection in inspections {
                let info = inspection
                    .info
                    .expect("completed inspection has media info");
                let (next, end) = apply_pending(&mut candidate, &target, start, collision, &info)
                    .map_err(|error| format!("{}: {error}", info.source.display()))?;
                imported.selection.extend(next.selection);
                imported.video |= next.video;
                imported.audio |= next.audio;
                imported.captions |= next.captions;
                start = end;
            }
            project::commit_edit_checked(&candidate, "import-media")?;
            let duration = candidate.duration();
            *project = candidate;
            for file in owned_files {
                file.keep();
            }
            self.source_deletions.extend(source_deletions);
            Ok((imported, duration))
        });
        Some(Completion {
            batch,
            paths,
            cancelled: false,
            result,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.completed.is_empty()
    }

    /// Temporary provider/download copies keep only sources referenced by the commit.
    /// External originals retain their separate, user-confirmed deletion request.
    pub(crate) fn finish_owned_sources(&mut self, files: Vec<OwnedFile>, paths: &[PathBuf]) {
        self.source_deletions
            .retain(|request| !files.iter().any(|file| file.path() == request.source()));
        for file in files {
            if paths.iter().any(|path| path == file.path()) {
                file.keep();
            }
        }
    }

    pub fn take_source_deletion(&mut self) -> Option<SourceDeletion> {
        self.source_deletions.pop_front()
    }
}

fn apply_pending(
    project: &mut Project,
    target: &Target,
    start: Time,
    collision: DragCollisionMode,
    info: &import::MediaInfo,
) -> Result<(import::ImportResult, Time), String> {
    info.snapshot.ensure_current()?;
    if info.video_streams == 0 && info.audio_streams == 0 && info.caption_cues.is_empty() {
        return Err("file contains no importable audio or video stream".into());
    }
    match target {
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
                start,
                target,
                collision,
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
                import::apply_vtt_cues_to_tracks(project, &info.caption_cues, &indices, start)?
            } else {
                import::apply_media_to_tracks(project, info, kind, &indices, start)?
            };
            let step = project.frame_step();
            let start = start.max(Time::ZERO).snapped(step);
            let end = start
                .saturating_add(info.duration)
                .snapped(step)
                .max(start.saturating_add(step));
            Ok((imported, end))
        }
    }
}
