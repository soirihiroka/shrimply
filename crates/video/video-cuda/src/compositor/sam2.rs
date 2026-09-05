use std::io::Write;

use super::*;

#[derive(Clone)]
pub(super) struct AnalysisJob {
    item_id: Uuid,
    pub(super) modifier_id: Uuid,
    pub(super) run_id: crate::sam2_analysis::RunId,
    prompt_signature: u64,
    cache_key: String,
    server_url: String,
    modifier: shrimply_video_modifiers::sam2::Sam2Modifier,
    start: Time,
    seed: Time,
    end: Time,
}

pub(super) fn pending_analysis(
    project: &Project,
    scheduled: &HashMap<Uuid, crate::sam2_analysis::RunId>,
) -> Option<AnalysisJob> {
    project
        .video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .flat_map(|item| {
            item.modifiers
                .iter()
                .enumerate()
                .filter_map(move |(modifier_index, modifier)| {
                    if !modifier.enabled {
                        return None;
                    }
                    let shrimply_video_modifiers::ModifierEffect::Raster(effect) = &modifier.effect
                    else {
                        return None;
                    };
                    let shrimply_video_modifiers::RasterModifierEffect::Sam2(sam2) = &**effect
                    else {
                        return None;
                    };
                    let prompt_signature = sam2.prompt_signature();
                    let (run_id, server_url) = crate::sam2_analysis::active_run(
                        modifier.id,
                        sam2.analysis_generation,
                        prompt_signature,
                    )?;
                    (sam2.analysis_generation > 0
                        && scheduled.get(&modifier.id).copied() != Some(run_id)
                        && (!sam2.points.is_empty() || sam2.box_prompt.is_some()))
                    .then_some(AnalysisJob {
                        item_id: item.id,
                        modifier_id: modifier.id,
                        run_id,
                        prompt_signature,
                        cache_key: crate::modifiers::sam2::cache_key(
                            project,
                            item,
                            modifier.id,
                            modifier_index,
                            sam2,
                        ),
                        server_url,
                        modifier: sam2.clone(),
                        start: item.start,
                        seed: sam2.seed_position.unwrap_or(item.start),
                        end: item.end,
                    })
                })
        })
        .next()
}

fn analysis_is_current(job: &AnalysisJob) -> bool {
    crate::sam2_analysis::is_current(job.modifier_id, job.run_id)
}

fn update_progress(
    job: &AnalysisJob,
    message: &str,
    completed_frames: u64,
    total_frames: u64,
) -> bool {
    crate::sam2_analysis::update(
        job.modifier_id,
        job.run_id,
        crate::sam2_analysis::Status::Running {
            message: message.to_string(),
            completed_frames,
            total_frames,
            prompt_signature: job.prompt_signature,
            server_url: job.server_url.clone(),
        },
    )
}

fn analyze_clip(
    project: &Project,
    job: &AnalysisJob,
    sessions: &mut RenderSessions,
    render_cache: &mut RenderCache,
    compositor: &mut CudaVideoCompositor,
    mask_cache: &crate::modifiers::sam2::Sam2MaskCache,
) -> Result<bool, String> {
    let first_frame = shrimply_math_core::frame_count(job.start, project.fps)
        .ok_or("project frame rate must be positive for SAM2 analysis")?;
    let end_frame = shrimply_math_core::frame_count(job.end, project.fps)
        .ok_or("project frame rate must be positive for SAM2 analysis")?;
    let total_frames = end_frame.saturating_sub(first_frame);
    if total_frames == 0 {
        return Ok(true);
    }
    let seed_frame = shrimply_math_core::frame_count(job.seed, project.fps)
        .ok_or("project frame rate must be positive for SAM2 analysis")?
        .clamp(first_frame, end_frame - 1);
    let seed_position = shrimply_math_core::time_from_frame(seed_frame, project.fps)
        .ok_or("project frame rate must be positive for SAM2 analysis")?;
    let prompt_time =
        shrimply_project::project::generated_item_time(job_item(project, job)?, seed_position)
            .unwrap_or(Time::ZERO);
    let request = shrimply_server_client::Sam2AnalysisRequest::new(
        match job.modifier.model {
            shrimply_video_modifiers::sam2::Sam2Model::Tiny => {
                shrimply_server_client::Sam2Model::Tiny
            }
            shrimply_video_modifiers::sam2::Sam2Model::Small => {
                shrimply_server_client::Sam2Model::Small
            }
            shrimply_video_modifiers::sam2::Sam2Model::BasePlus => {
                shrimply_server_client::Sam2Model::BasePlus
            }
            shrimply_video_modifiers::sam2::Sam2Model::Large => {
                shrimply_server_client::Sam2Model::Large
            }
        },
        total_frames,
        seed_frame - first_frame,
        job.modifier
            .points
            .iter()
            .map(|point| {
                let position = point
                    .position
                    .value_at(prompt_time)
                    .clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
                shrimply_server_client::Sam2Point {
                    position,
                    label: match point.label {
                        shrimply_video_modifiers::sam2::Sam2PointLabel::Foreground => 1,
                        shrimply_video_modifiers::sam2::Sam2PointLabel::Background => 0,
                    },
                }
            })
            .collect(),
        job.modifier
            .box_prompt
            .map(|box_prompt| shrimply_server_client::Sam2Box {
                minimum: box_prompt.min,
                maximum: box_prompt.max,
            }),
    );
    let mut archive = tempfile::NamedTempFile::new()
        .map_err(|error| format!("create SAM2 proxy archive: {error}"))?;
    shrimply_server_client::write_sam2_archive_header(archive.as_file_mut(), &request)?;

    if !update_progress(job, "Preparing frames…", 0, total_frames.saturating_mul(2)) {
        return Ok(false);
    }

    compositor.begin_sam2_analysis(job.modifier_id);
    let capture: Result<bool, String> = (|| {
        for (completed_frames, frame) in (first_frame..end_frame).enumerate() {
            if !analysis_is_current(job) {
                return Ok(false);
            }
            analyze_frame(project, job, frame, sessions, render_cache, compositor)?;
            let jpeg = compositor
                .take_sam2_proxy()
                .ok_or_else(|| "SAM2 modifier did not capture a proxy frame".to_string())?;
            shrimply_server_client::write_sam2_archive_frame(archive.as_file_mut(), &jpeg)?;
            if !update_progress(
                job,
                "Preparing frames…",
                completed_frames as u64 + 1,
                total_frames.saturating_mul(2),
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    compositor.end_sam2_analysis();
    if !capture? {
        return Ok(false);
    }
    archive
        .as_file_mut()
        .flush()
        .map_err(|error| format!("flush SAM2 proxy archive: {error}"))?;
    mask_cache.begin_analysis(&job.cache_key);
    let mut server_error = None;
    let mut result_frames = None;
    let cancellation = shrimply_server_client::CancellationToken::new(&job.server_url)?;
    if !crate::sam2_analysis::set_cancellation(job.modifier_id, job.run_id, cancellation.clone()) {
        return Ok(false);
    }
    if !crate::sam2_analysis::update(
        job.modifier_id,
        job.run_id,
        crate::sam2_analysis::Status::Running {
            message: "Sending request…".to_string(),
            completed_frames: total_frames,
            total_frames: total_frames.saturating_mul(2),
            prompt_signature: job.prompt_signature,
            server_url: job.server_url.clone(),
        },
    ) {
        cancellation.cancel();
        return Ok(false);
    }
    shrimply_server_client::analyze_sam2(
        &job.server_url,
        &cancellation,
        archive.path(),
        |event| {
            if !analysis_is_current(job) {
                return false;
            }
            match event {
                shrimply_server_client::Sam2Event::Queued { position } => {
                    crate::sam2_analysis::update(
                        job.modifier_id,
                        job.run_id,
                        crate::sam2_analysis::Status::Running {
                            message: shrimply_server_client::queued_status(position),
                            completed_frames: total_frames,
                            total_frames: total_frames.saturating_mul(2),
                            prompt_signature: job.prompt_signature,
                            server_url: job.server_url.clone(),
                        },
                    )
                }
                shrimply_server_client::Sam2Event::Progress {
                    message,
                    completed_frames,
                    ..
                } => update_progress(
                    job,
                    &message,
                    total_frames.saturating_add(completed_frames.min(total_frames)),
                    total_frames.saturating_mul(2),
                ),
                shrimply_server_client::Sam2Event::Mask { frame_index, mask } => {
                    if frame_index >= total_frames {
                        server_error = Some(format!(
                            "SAM2 server returned out-of-range frame {frame_index}"
                        ));
                        return false;
                    }
                    let Ok(frame) = i64::try_from(first_frame + frame_index) else {
                        server_error = Some("SAM2 frame index exceeds the cache range".to_string());
                        return false;
                    };
                    if let Err(error) = mask_cache.insert_staged(&job.cache_key, frame, &mask) {
                        server_error = Some(error);
                        return false;
                    }
                    true
                }
                shrimply_server_client::Sam2Event::Result { completed_frames } => {
                    result_frames = Some(completed_frames);
                    true
                }
                shrimply_server_client::Sam2Event::Error { code, message } => {
                    server_error = Some(format!("SAM2 server error {code}: {message}"));
                    false
                }
            }
        },
    )?;
    if let Some(error) = server_error {
        return Err(error);
    }
    if !analysis_is_current(job) {
        return Ok(false);
    }
    if result_frames != Some(total_frames) {
        return Err(format!(
            "SAM2 server returned {} masks; expected {total_frames}",
            result_frames.unwrap_or(0)
        ));
    }
    Ok(true)
}

fn job_item<'a>(project: &'a Project, job: &AnalysisJob) -> Result<&'a VideoItem, String> {
    project
        .video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == job.item_id)
        .ok_or_else(|| "SAM2 source item no longer exists".to_string())
}

fn analyze_frame(
    project: &Project,
    job: &AnalysisJob,
    frame: u64,
    sessions: &mut RenderSessions,
    render_cache: &mut RenderCache,
    compositor: &mut CudaVideoCompositor,
) -> Result<(), String> {
    let position = shrimply_math_core::time_from_frame(frame, project.fps)
        .ok_or("project frame rate must be positive for SAM2 analysis")?;
    let volume_revision = sessions.volume_revision;
    let audio_analysis = FrameAudioAnalysis {
        volume: sessions.volume.sample(project, position, volume_revision),
        mouth: sessions.mouth.sample(project, position, volume_revision),
    };
    let rendered = render_project_frame(
        project,
        position,
        sessions,
        render_cache,
        compositor,
        RenderMode::Preview {
            accuracy: CompositeAccuracy::FULLY_ACCURATE,
        },
        &audio_analysis,
        Some(std::slice::from_ref(&job.item_id)),
        None,
        false,
        None,
        None,
    );
    if !rendered.errors.is_empty() {
        return Err(format!(
            "SAM2 clip analysis failed at {}: {}",
            position.as_label(),
            rendered.errors.join("\n")
        ));
    }
    if rendered.loading {
        return Err(format!(
            "SAM2 clip analysis could not load the source frame at {}",
            position.as_label()
        ));
    }
    drop(rendered.frame);
    Ok(())
}

pub(super) fn spawn_analysis(
    project: &Project,
    job: AnalysisJob,
    event_tx: SyncSender<VideoEvent>,
    claim_waiter: &crate::sam2_analysis::ClaimWaiter,
) -> bool {
    let Some(claim) = crate::sam2_analysis::try_claim(job.modifier_id, job.run_id, claim_waiter)
    else {
        return false;
    };
    let project = project.clone();
    thread::Builder::new()
        .name(format!("sam2-analysis-{}", job.modifier_id))
        .spawn(move || {
            let _claim = claim;
            if !analysis_is_current(&job) {
                return;
            }
            let mask_cache = crate::modifiers::sam2::Sam2MaskCache::shared();
            if mask_cache.analysis_complete(&job.cache_key) {
                crate::sam2_analysis::update(
                    job.modifier_id,
                    job.run_id,
                    crate::sam2_analysis::Status::Complete {
                        prompt_signature: job.prompt_signature,
                    },
                );
                return;
            }
            let result = (|| {
                let mut sessions = RenderSessions::default();
                let mut render_cache = RenderCache::default();
                let mut compositor = CudaVideoCompositor::new()?;
                if !analysis_is_current(&job) {
                    return Ok(false);
                }
                analyze_clip(
                    &project,
                    &job,
                    &mut sessions,
                    &mut render_cache,
                    &mut compositor,
                    &mask_cache,
                )
            })();
            match result {
                Ok(true) => {
                    mask_cache.complete_analysis(&job.cache_key);
                    if !crate::sam2_analysis::update(
                        job.modifier_id,
                        job.run_id,
                        crate::sam2_analysis::Status::Complete {
                            prompt_signature: job.prompt_signature,
                        },
                    ) {
                        mask_cache.abort_analysis(&job.cache_key);
                    }
                }
                Ok(false) => mask_cache.abort_analysis(&job.cache_key),
                Err(error) => {
                    mask_cache.abort_analysis(&job.cache_key);
                    let display_error = if error.starts_with("Compute server connection failed") {
                        tracing::error!(%error, "SAM2 compute connection failed");
                        "Compute server connection failed".to_string()
                    } else {
                        error.clone()
                    };
                    if crate::sam2_analysis::update(
                        job.modifier_id,
                        job.run_id,
                        crate::sam2_analysis::Status::Failed(display_error),
                    ) {
                        let _ = event_tx.try_send(VideoEvent::Error(error));
                    }
                }
            }
            crate::sam2_analysis::clear_cancellation(job.modifier_id, job.run_id);
        })
        .expect("spawn SAM2 analysis worker");
    true
}
