use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use ffmpeg_next::{format::Pixel, frame};
use shrimply_project::project::{ItemAddress, Project, Time};
use shrimply_video_modifiers::{ModifierEffect, RasterModifierEffect};
use uuid::Uuid;

use crate::{
    compositor::{EXPORT_ASSETS_LOADING, VideoExportRenderer},
    modifiers::transparent_fill::{
        AnalysisFrame, TransparentFillMaskCache, analysis_cache_key, analysis_frames, encode_mask,
        render_input_project,
    },
};

const MAXIMUM_MASK_WORKERS: usize = 6;
const MAX_RETAINED_TERMINAL_JOBS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Missing,
    Running { completed: u64, total: u64 },
    Complete,
    Cancelled,
    Failed(String),
}

struct Job {
    run_id: RunId,
    status: Status,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct Registry {
    active: HashMap<AnalysisIdentity, Job>,
    terminal: HashMap<AnalysisIdentity, (RunId, Status)>,
    terminal_order: VecDeque<(AnalysisIdentity, RunId)>,
}

impl Registry {
    fn insert_terminal(&mut self, identity: AnalysisIdentity, run_id: RunId, status: Status) {
        self.terminal.insert(identity.clone(), (run_id, status));
        self.terminal_order.push_back((identity, run_id));
        let terminal = &self.terminal;
        self.terminal_order.retain(|(identity, run_id)| {
            terminal
                .get(identity)
                .is_some_and(|(current, _)| current == run_id)
        });
        while self.terminal.len() > MAX_RETAINED_TERMINAL_JOBS {
            let (identity, run_id) = self
                .terminal_order
                .pop_front()
                .expect("terminal analysis order must track every retained job");
            if self
                .terminal
                .get(&identity)
                .is_some_and(|(current, _)| *current == run_id)
            {
                self.terminal.remove(&identity);
            }
        }
    }

    fn terminal_status(&mut self, identity: &AnalysisIdentity) -> Option<Status> {
        let (run_id, status) = self.terminal.get(identity)?.clone();
        self.terminal_order.retain(|(stored, _)| stored != identity);
        self.terminal_order.push_back((identity.clone(), run_id));
        Some(status)
    }

    fn terminal_status_for_run(&mut self, run_id: RunId) -> Option<Status> {
        let (identity, status) =
            self.terminal
                .iter()
                .find_map(|(identity, (stored, status))| {
                    (*stored == run_id).then(|| (identity.clone(), status.clone()))
                })?;
        self.terminal_order
            .retain(|(stored, _)| stored != &identity);
        self.terminal_order.push_back((identity, run_id));
        Some(status)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AnalysisIdentity {
    address: ItemAddress,
    modifier_id: Uuid,
    prompt_signature: u64,
    cache_key: String,
}

#[derive(Clone)]
pub struct PreparedStatus {
    identity: AnalysisIdentity,
    width: u32,
    height: u32,
    frames: u64,
}

struct Input {
    project: Project,
    address: ItemAddress,
    identity: AnalysisIdentity,
    frames: Vec<AnalysisFrame>,
    points: Vec<shrimply_video_modifiers::transparent_fill::TransparentFillPoint>,
    tolerance: shrimply_core::timeline_value::TimelineValue<f32>,
    maximum_gap: u32,
}

struct MaskJob {
    frame_index: u64,
    rgba: Vec<u8>,
    seeds: Vec<(u32, u32)>,
    tolerance: f32,
}

struct MaskResult {
    frame_index: u64,
    mask: Result<(Vec<u8>, Vec<u8>), String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunId(Uuid);

static JOBS: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));

pub fn analyze(
    project: Project,
    address: &ItemAddress,
    modifier_id: Uuid,
) -> Result<RunId, String> {
    let input = prepare(project, address, modifier_id)?;
    let total = u64::try_from(input.frames.len())
        .map_err(|_| "transparent fill frame count is too large")?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let identity = input.identity.clone();
    let run_id = RunId(Uuid::new_v4());
    {
        let mut jobs = JOBS
            .lock()
            .expect("transparent fill analysis job lock is poisoned");
        if jobs
            .active
            .keys()
            .any(|running| running.cache_key == identity.cache_key)
        {
            return Err("transparent fill analysis is already running".to_string());
        }
        jobs.terminal.remove(&identity);
        jobs.active.insert(
            identity.clone(),
            Job {
                run_id,
                status: Status::Running {
                    completed: 0,
                    total,
                },
                cancelled: cancelled.clone(),
            },
        );
    }
    let worker_identity = identity.clone();
    let spawn = thread::Builder::new()
        .name(format!("transparent-fill-analysis-{modifier_id}"))
        .spawn(move || {
            let result = analyze_inner(input, run_id, &cancelled);
            let mut jobs = JOBS
                .lock()
                .expect("transparent fill analysis job lock is poisoned");
            let Some(job) = jobs.active.get(&worker_identity) else {
                return;
            };
            if job.run_id != run_id {
                return;
            }
            let status = match result {
                Ok(()) if cancelled.load(Ordering::Acquire) => Status::Cancelled,
                Ok(()) => Status::Complete,
                Err(_) if cancelled.load(Ordering::Acquire) => Status::Cancelled,
                Err(error) => Status::Failed(error),
            };
            jobs.active.remove(&worker_identity);
            jobs.insert_terminal(worker_identity, run_id, status);
        });
    if let Err(error) = spawn {
        let mut jobs = JOBS
            .lock()
            .expect("transparent fill analysis job lock is poisoned");
        if jobs
            .active
            .get(&identity)
            .is_some_and(|job| job.run_id == run_id)
        {
            jobs.active.remove(&identity);
        }
        return Err(format!("spawn transparent fill analysis: {error}"));
    }
    Ok(run_id)
}

pub fn active_run_prepared(prepared: &PreparedStatus) -> Option<RunId> {
    JOBS.lock()
        .expect("transparent fill analysis job lock is poisoned")
        .active
        .get(&prepared.identity)
        .filter(|job| matches!(job.status, Status::Running { .. }))
        .map(|job| job.run_id)
}

pub fn cancel(run_id: RunId) -> bool {
    let mut jobs = JOBS
        .lock()
        .expect("transparent fill analysis job lock is poisoned");
    let Some(identity) = jobs.active.iter().find_map(|(identity, job)| {
        (job.run_id == run_id && matches!(job.status, Status::Running { .. }))
            .then(|| identity.clone())
    }) else {
        return false;
    };
    let job = jobs
        .active
        .remove(&identity)
        .expect("located transparent fill analysis job must exist");
    job.cancelled.store(true, Ordering::Release);
    jobs.insert_terminal(identity, run_id, Status::Cancelled);
    true
}

pub fn status(project: &Project, address: &ItemAddress, modifier_id: Uuid) -> Status {
    let Ok(prepared) = prepare_status(project, address, modifier_id) else {
        return Status::Missing;
    };
    status_prepared(&prepared)
}

pub fn status_prepared(prepared: &PreparedStatus) -> Status {
    {
        let mut jobs = JOBS
            .lock()
            .expect("transparent fill analysis job lock is poisoned");
        if let Some(status) = jobs
            .active
            .get(&prepared.identity)
            .map(|job| job.status.clone())
        {
            return status;
        }
        if let Some(status) = jobs.terminal_status(&prepared.identity) {
            return status;
        }
    }
    if TransparentFillMaskCache::shared().analysis_complete(
        &prepared.identity.cache_key,
        prepared.width,
        prepared.height,
        prepared.frames,
    ) {
        Status::Complete
    } else {
        Status::Missing
    }
}

pub fn status_for_run(run_id: RunId) -> Status {
    let mut jobs = JOBS
        .lock()
        .expect("transparent fill analysis job lock is poisoned");
    let active = jobs
        .active
        .values()
        .find(|job| job.run_id == run_id)
        .map(|job| job.status.clone());
    active
        .or_else(|| jobs.terminal_status_for_run(run_id))
        .unwrap_or(Status::Missing)
}

fn prepare(project: Project, address: &ItemAddress, modifier_id: Uuid) -> Result<Input, String> {
    let item = project
        .video_item(address)
        .ok_or_else(|| "transparent fill item no longer exists".to_string())?;
    let modifier_index = item
        .modifiers
        .iter()
        .position(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| "transparent fill modifier no longer exists".to_string())?;
    let ModifierEffect::Raster(effect) = &item.modifiers[modifier_index].effect else {
        return Err("selected modifier is not Transparent Fill".to_string());
    };
    let RasterModifierEffect::TransparentFill(fill) = &**effect else {
        return Err("selected modifier is not Transparent Fill".to_string());
    };
    if fill.points.is_empty() {
        return Err("add at least one transparent fill point before analyzing".to_string());
    }
    let frames = analysis_frames(&project, address)?;
    let signature = fill.prompt_signature();
    let points = fill.points.clone();
    let tolerance = fill.tolerance.clone();
    let maximum_gap = fill.maximum_gap;
    let project = render_input_project(&project, address, modifier_index)?;
    let cache_key = analysis_cache_key(&project, address, modifier_id, signature);
    Ok(Input {
        project,
        identity: AnalysisIdentity {
            address: address.clone(),
            modifier_id,
            prompt_signature: signature,
            cache_key,
        },
        address: address.clone(),
        frames,
        points,
        tolerance,
        maximum_gap,
    })
}

pub fn prepare_status(
    project: &Project,
    address: &ItemAddress,
    modifier_id: Uuid,
) -> Result<PreparedStatus, String> {
    let item = project
        .video_item(address)
        .ok_or_else(|| "transparent fill item no longer exists".to_string())?;
    let (index, modifier) = item
        .modifiers
        .iter()
        .enumerate()
        .find(|(_, modifier)| modifier.id == modifier_id)
        .ok_or_else(|| "transparent fill modifier no longer exists".to_string())?;
    let ModifierEffect::Raster(effect) = &modifier.effect else {
        return Err("selected modifier is not Transparent Fill".to_string());
    };
    let RasterModifierEffect::TransparentFill(fill) = &**effect else {
        return Err("selected modifier is not Transparent Fill".to_string());
    };
    let prompt_signature = fill.prompt_signature();
    let frames = u64::try_from(analysis_frames(project, address)?.len())
        .map_err(|_| "transparent fill frame count is too large")?;
    let render_project = render_input_project(project, address, index)?;
    Ok(PreparedStatus {
        identity: AnalysisIdentity {
            address: address.clone(),
            modifier_id,
            prompt_signature,
            cache_key: analysis_cache_key(&render_project, address, modifier_id, prompt_signature),
        },
        width: project.canvas_size.width,
        height: project.canvas_size.height,
        frames,
    })
}

fn analyze_inner(input: Input, run_id: RunId, cancelled: &AtomicBool) -> Result<(), String> {
    let cache = TransparentFillMaskCache::shared();
    let staging_key = format!("{}:run:{}", input.identity.cache_key, run_id.0);
    cache.begin_analysis(&staging_key)?;
    let result = (|| {
        let width = input.project.canvas_size.width.max(1);
        let height = input.project.canvas_size.height.max(1);
        let total = u64::try_from(input.frames.len())
            .map_err(|_| "transparent fill frame count is too large")?;
        let mut renderer = VideoExportRenderer::new(48_000)?;
        let item = input
            .project
            .video_item(&input.address)
            .ok_or("transparent fill source item disappeared")?;
        let worker_count = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .saturating_sub(1)
            .clamp(1, MAXIMUM_MASK_WORKERS)
            .min(usize::try_from(total).unwrap_or(usize::MAX).max(1));
        let (job_sender, job_receiver) = mpsc::sync_channel::<MaskJob>(worker_count);
        let job_receiver = Mutex::new(job_receiver);
        let (result_sender, result_receiver) = mpsc::channel::<MaskResult>();
        thread::scope(|scope| -> Result<(), String> {
            for _ in 0..worker_count {
                let result_sender = result_sender.clone();
                let job_receiver = &job_receiver;
                scope.spawn(move || {
                    loop {
                        let job = match job_receiver
                            .lock()
                            .expect("transparent fill worker queue lock is poisoned")
                            .recv()
                        {
                            Ok(job) => job,
                            Err(_) => break,
                        };
                        let mask = if cancelled.load(Ordering::Acquire) {
                            Err("transparent fill analysis cancelled".to_string())
                        } else {
                            shrimply_math_color::transparent_fill_mask(
                                &job.rgba,
                                width,
                                height,
                                &job.seeds,
                                job.tolerance,
                                input.maximum_gap,
                            )
                            .and_then(|mask| {
                                let png = encode_mask(&mask, width, height)?;
                                Ok((mask, png))
                            })
                        };
                        if result_sender
                            .send(MaskResult {
                                frame_index: job.frame_index,
                                mask,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
            drop(result_sender);

            let mut submitted = 0_u64;
            let mut completed = 0_u64;
            let store_result = |result: MaskResult, completed: u64| -> Result<u64, String> {
                let (mask, png) = result.mask?;
                cache.insert_staged_encoded(
                    &staging_key,
                    i64::try_from(result.frame_index)
                        .map_err(|_| "transparent fill frame is too large")?,
                    &mask,
                    png,
                )?;
                let completed = completed + 1;
                update_progress(&input.identity, run_id, completed, total);
                Ok(completed)
            };

            for sample in &input.frames {
                if cancelled.load(Ordering::Acquire) {
                    return Err("transparent fill analysis cancelled".to_string());
                }
                let composited = loop {
                    match renderer.render_transparent_fill_input(
                        &input.project,
                        sample.timeline_position,
                        &input.address,
                    ) {
                        Ok(frame) => break frame,
                        Err(error) if error == EXPORT_ASSETS_LOADING => {
                            if cancelled.load(Ordering::Acquire) {
                                return Err("transparent fill analysis cancelled".to_string());
                            }
                            thread::yield_now();
                        }
                        Err(error) => return Err(error),
                    }
                };
                let mut output = frame::Video::new(Pixel::RGBA, width, height);
                renderer.copy_to_rgba_frame(composited, &mut output)?;
                let row_bytes = width as usize * 4;
                let stride = output.stride(0);
                let mut rgba = Vec::with_capacity(row_bytes * height as usize);
                for row in output.data(0).chunks_exact(stride).take(height as usize) {
                    rgba.extend_from_slice(&row[..row_bytes]);
                }
                let local_time =
                    shrimply_project::project::generated_item_time(item, sample.sequence_position)
                        .unwrap_or(Time::ZERO);
                let seeds = input
                    .points
                    .iter()
                    .map(|point| {
                        let point = point
                            .position
                            .value_at(local_time)
                            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
                        (
                            (point.x * width.saturating_sub(1) as f32).round() as u32,
                            (point.y * height.saturating_sub(1) as f32).round() as u32,
                        )
                    })
                    .collect();
                job_sender
                    .send(MaskJob {
                        frame_index: sample.cache_index,
                        rgba,
                        seeds,
                        tolerance: input.tolerance.value_at(local_time),
                    })
                    .map_err(|_| "transparent fill mask workers stopped unexpectedly")?;
                submitted += 1;
                while let Ok(result) = result_receiver.try_recv() {
                    completed = store_result(result, completed)?;
                }
            }
            drop(job_sender);
            while completed < submitted {
                completed = store_result(
                    result_receiver
                        .recv()
                        .map_err(|_| "transparent fill mask workers stopped unexpectedly")?,
                    completed,
                )?;
            }
            Ok(())
        })?;
        let mut jobs = JOBS
            .lock()
            .expect("transparent fill analysis job lock is poisoned");
        let is_current = jobs
            .active
            .get(&input.identity)
            .is_some_and(|job| job.run_id == run_id && !job.cancelled.load(Ordering::Acquire));
        if !is_current {
            return Err("transparent fill analysis cancelled".to_string());
        }
        cache.publish_analysis(
            &staging_key,
            &input.identity.cache_key,
            width,
            height,
            total,
        )?;
        jobs.active
            .get_mut(&input.identity)
            .expect("current transparent fill analysis job must exist")
            .status = Status::Complete;
        Ok(())
    })();
    if result.is_err() {
        cache.abort_analysis(&staging_key);
    }
    result
}

fn update_progress(identity: &AnalysisIdentity, run_id: RunId, completed: u64, total: u64) {
    let mut jobs = JOBS
        .lock()
        .expect("transparent fill analysis job lock is poisoned");
    if let Some(job) = jobs.active.get_mut(identity)
        && job.run_id == run_id
        && !job.cancelled.load(Ordering::Acquire)
    {
        job.status = Status::Running { completed, total };
    }
}
