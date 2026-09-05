use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use ffmpeg_next::{format::Pixel, frame::Video};
use glam::{Quat, Vec3};
use hashbrown::HashMap;
use num_traits::ToPrimitive;
use rusqlite::{Connection, params};
use shrimply_3dgs::{
    COLMAP_TRACKING_MODEL, ColmapCameraModel, ColmapQuality, Projection, TrackingCameraSource,
};
use shrimply_math_geometry::{
    InterpolatedCameraMotion, NormalizedCameraPose, interpolate_camera_motion,
    relative_camera_poses, vertical_fov_degrees_from_focal_length,
};
use shrimply_project::project::{Project, Time, fraction_denominator, fraction_numerator};
use uuid::Uuid;

use crate::compositor::VideoExportRenderer;

const CACHE_DATABASE: &str = "cache/camera-reconstruction.sqlite";
const CACHE_VERSION: i64 = 5;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const ANALYSIS_AUDIO_SAMPLE_RATE: u32 = 48_000;
const MIN_VERTICAL_FOV_DEGREES: f64 = 1.0;
const MAX_PERSPECTIVE_FOV_DEGREES: f64 = 179.0;
const MAX_FISHEYE_FOV_DEGREES: f64 = 360.0;
const ANALYSIS_CANCELLED: &str = "camera analysis cancelled";

#[derive(Clone, Debug, PartialEq)]
pub enum AnalysisStatus {
    NotAnalyzed,
    OutOfDate,
    Queued,
    Loading,
    Analyzing {
        message: String,
        completed_frames: u64,
        total_frames: u64,
    },
    Cancelling,
    Cancelled,
    Ready {
        sample_count: usize,
    },
    Failed {
        error: String,
    },
    MissingSourceTrack,
    EmptySourceTrack,
}

#[derive(Clone, Copy, Debug)]
pub struct ReconstructedCameraSample {
    pub position: Vec3,
    pub rotation: Quat,
    pub projection: Projection,
    pub vertical_fov_degrees: f32,
}

pub fn apply_custom_camera_offset(
    mut camera: ReconstructedCameraSample,
    custom_position: Vec3,
    custom_rotation_degrees: Vec3,
) -> ReconstructedCameraSample {
    (camera.position, camera.rotation) = shrimply_math_geometry::apply_reconstructed_camera_motion(
        camera.position,
        camera.rotation,
        custom_position,
        custom_rotation_degrees,
    );
    camera
}

#[derive(Clone)]
struct TrackSample {
    time: Time,
    pose: NormalizedCameraPose,
    projection: Projection,
    vertical_fov_degrees: f64,
}

#[derive(Clone)]
struct CachedTrack {
    source: TrackingCameraSource,
    cache_version: i64,
    samples: Vec<TrackSample>,
}

#[derive(Clone)]
struct JobState {
    source: TrackingCameraSource,
    status: AnalysisStatus,
    analysis_id: Uuid,
    cancellation: Arc<AtomicBool>,
    compute_cancellation: shrimply_server_client::CancellationToken,
}

struct State {
    jobs: HashMap<Uuid, JobState>,
    tracks: HashMap<Uuid, CachedTrack>,
    database_error: Option<String>,
}

struct Job {
    project: Project,
    camera_item_id: Uuid,
    source: TrackingCameraSource,
    server_url: String,
    analysis_id: Uuid,
    cancellation: Arc<AtomicBool>,
    compute_cancellation: shrimply_server_client::CancellationToken,
}

struct Service {
    state: Arc<Mutex<State>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

pub fn analyze(
    project: Project,
    camera_item_id: Uuid,
    source: TrackingCameraSource,
    server_url: String,
) {
    let service = service();
    let compute_cancellation = match shrimply_server_client::CancellationToken::new(&server_url) {
        Ok(cancellation) => cancellation,
        Err(error) => {
            tracing::error!(%error, "could not prepare managed 3D tracking job");
            return;
        }
    };
    let analysis_id = compute_cancellation.job_id();
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut state = service
            .state
            .lock()
            .expect("camera reconstruction state poisoned");
        if state.jobs.get(&camera_item_id).is_some_and(|job| {
            matches!(
                job.status,
                AnalysisStatus::Queued
                    | AnalysisStatus::Loading
                    | AnalysisStatus::Analyzing { .. }
                    | AnalysisStatus::Cancelling
            )
        }) {
            return;
        }
        state.jobs.insert(
            camera_item_id,
            JobState {
                source: source.clone(),
                status: AnalysisStatus::Analyzing {
                    message: "Preparing frames…".to_string(),
                    completed_frames: 0,
                    total_frames: 0,
                },
                analysis_id,
                cancellation: cancellation.clone(),
                compute_cancellation: compute_cancellation.clone(),
            },
        );
    }
    let job = Job {
        project,
        camera_item_id,
        source,
        server_url,
        analysis_id,
        cancellation,
        compute_cancellation,
    };
    let state = service.state.clone();
    if let Err(error) = thread::Builder::new()
        .name(format!("camera-reconstruction-{analysis_id}"))
        .spawn(move || process_job(job, state))
        && let Some(current) = service
            .state
            .lock()
            .expect("camera reconstruction state poisoned")
            .jobs
            .get_mut(&camera_item_id)
            .filter(|current| current.analysis_id == analysis_id)
    {
        current.status = AnalysisStatus::Failed {
            error: format!("could not start camera analysis: {error}"),
        };
    }
}

pub fn cancel(camera_item_id: Uuid, source: &TrackingCameraSource) {
    let compute_cancellation = {
        let mut state = service()
            .state
            .lock()
            .expect("camera reconstruction state poisoned");
        let Some(job) = state.jobs.get_mut(&camera_item_id).filter(|job| {
            job.source == *source
                && matches!(
                    job.status,
                    AnalysisStatus::Queued
                        | AnalysisStatus::Loading
                        | AnalysisStatus::Analyzing { .. }
                )
        }) else {
            return;
        };
        job.cancellation.store(true, Ordering::Release);
        job.status = if matches!(job.status, AnalysisStatus::Queued) {
            AnalysisStatus::Cancelled
        } else {
            AnalysisStatus::Cancelling
        };
        job.compute_cancellation.clone()
    };
    compute_cancellation.cancel();
}

pub fn status(camera_item_id: Uuid, source: &TrackingCameraSource) -> AnalysisStatus {
    let state = service()
        .state
        .lock()
        .expect("camera reconstruction state poisoned");
    if let Some(job) = state
        .jobs
        .get(&camera_item_id)
        .filter(|job| job.source == *source)
    {
        return job.status.clone();
    }
    if let Some(track) = state.tracks.get(&camera_item_id) {
        if track.source == *source && track.cache_version == CACHE_VERSION {
            return AnalysisStatus::Ready {
                sample_count: track.samples.len(),
            };
        }
        return AnalysisStatus::OutOfDate;
    }
    if let Some(error) = &state.database_error {
        return AnalysisStatus::Failed {
            error: error.clone(),
        };
    }
    AnalysisStatus::NotAnalyzed
}

pub fn has_matching_cache(camera_item_id: Uuid, source: &TrackingCameraSource) -> bool {
    service()
        .state
        .lock()
        .expect("camera reconstruction state poisoned")
        .tracks
        .get(&camera_item_id)
        .is_some_and(|track| track.source == *source && track.cache_version == CACHE_VERSION)
}

pub fn sample(
    camera_item_id: Uuid,
    source: &TrackingCameraSource,
    local_time: Time,
) -> Option<ReconstructedCameraSample> {
    let state = service()
        .state
        .lock()
        .expect("camera reconstruction state poisoned");
    let track = state.tracks.get(&camera_item_id)?;
    if track.source != *source || track.cache_version != CACHE_VERSION {
        return None;
    }
    let first = track.samples.first()?;
    let motion = if local_time <= first.time {
        motion(first)
    } else if let Some(last) = track
        .samples
        .last()
        .filter(|sample| local_time >= sample.time)
    {
        motion(last)
    } else {
        let after_index = track
            .samples
            .partition_point(|sample| sample.time < local_time);
        let before = &track.samples[after_index - 1];
        let after = &track.samples[after_index];
        let progress = ((local_time.seconds - before.time.seconds)
            / (after.time.seconds - before.time.seconds))
            .to_f64()
            .unwrap_or(0.0);
        interpolate_camera_motion(motion(before), motion(after), progress)
    };
    let projection = track
        .samples
        .partition_point(|sample| sample.time <= local_time)
        .saturating_sub(1)
        .min(track.samples.len() - 1);
    Some(ReconstructedCameraSample {
        position: motion.position.as_vec3(),
        rotation: motion.rotation.as_quat().normalize(),
        projection: track.samples[projection].projection,
        vertical_fov_degrees: motion.vertical_fov_degrees as f32,
    })
}

fn motion(sample: &TrackSample) -> InterpolatedCameraMotion {
    InterpolatedCameraMotion {
        position: sample.pose.position,
        rotation: sample.pose.rotation,
        vertical_fov_degrees: sample.vertical_fov_degrees,
    }
}

fn service() -> &'static Service {
    SERVICE.get_or_init(|| {
        let (tracks, database_error) = match load_database() {
            Ok(tracks) => (tracks, None),
            Err(error) => (HashMap::new(), Some(error)),
        };
        let state = Arc::new(Mutex::new(State {
            jobs: HashMap::new(),
            tracks,
            database_error,
        }));
        Service { state }
    })
}

fn process_job(job: Job, state: Arc<Mutex<State>>) {
    {
        let mut state = state.lock().expect("camera reconstruction state poisoned");
        let Some(current) = state
            .jobs
            .get_mut(&job.camera_item_id)
            .filter(|current| current.analysis_id == job.analysis_id)
        else {
            return;
        };
        if job.cancellation.load(Ordering::Acquire) {
            current.status = AnalysisStatus::Cancelled;
            return;
        }
        current.status = AnalysisStatus::Analyzing {
            message: "Preparing frames…".to_string(),
            completed_frames: 0,
            total_frames: 0,
        };
    }
    let result = run_analysis(&job, &state);
    let mut state = state.lock().expect("camera reconstruction state poisoned");
    if state
        .jobs
        .get(&job.camera_item_id)
        .is_none_or(|current| current.analysis_id != job.analysis_id)
    {
        return;
    }
    if job.cancellation.load(Ordering::Acquire) {
        state
            .jobs
            .get_mut(&job.camera_item_id)
            .expect("camera analysis job disappeared")
            .status = AnalysisStatus::Cancelled;
        return;
    }
    match result {
        Ok(track) => {
            let sample_count = track.samples.len();
            if let Err(error) = store_track(job.camera_item_id, &track) {
                tracing::error!(
                    camera_item_id = %job.camera_item_id,
                    source_track_id = %job.source.track_id,
                    error,
                    "could not store camera reconstruction"
                );
                state
                    .jobs
                    .get_mut(&job.camera_item_id)
                    .expect("camera analysis job disappeared")
                    .status = AnalysisStatus::Failed { error };
                return;
            }
            state.database_error = None;
            state.tracks.insert(job.camera_item_id, track);
            state
                .jobs
                .get_mut(&job.camera_item_id)
                .expect("camera analysis job disappeared")
                .status = AnalysisStatus::Ready { sample_count };
        }
        Err(error) => {
            tracing::error!(
                camera_item_id = %job.camera_item_id,
                source_track_id = %job.source.track_id,
                error,
                "camera reconstruction failed"
            );
            let display_error = if error.starts_with("Compute server connection failed") {
                "Compute server connection failed".to_string()
            } else {
                error
            };
            state
                .jobs
                .get_mut(&job.camera_item_id)
                .expect("camera analysis job disappeared")
                .status = AnalysisStatus::Failed {
                error: display_error,
            };
        }
    }
}

fn run_analysis(job: &Job, state: &Mutex<State>) -> Result<CachedTrack, String> {
    if job.cancellation.load(Ordering::Acquire) {
        return Err(ANALYSIS_CANCELLED.to_string());
    }
    if !(1..=60).contains(&job.source.settings.analysis_fps) {
        return Err("analysis FPS must be between 1 and 60".to_string());
    }
    let (consuming_track_id, consuming_item) = job
        .project
        .video_tracks
        .iter()
        .find_map(|track| {
            track
                .items
                .iter()
                .find(|item| item.id == job.camera_item_id)
                .map(|item| (track.id, item))
        })
        .ok_or_else(|| "consuming 3D item is missing".to_string())?;
    if consuming_track_id == job.source.track_id {
        return Err("a 3D item cannot track its own visual track".to_string());
    }
    let source_track = job
        .project
        .video_tracks
        .iter()
        .find(|track| track.id == job.source.track_id)
        .ok_or_else(|| "source visual track is missing".to_string())?;
    let (analysis_position_start, analysis_position_end) = source_track
        .items
        .iter()
        .filter_map(|item| {
            let start = item.start.max(consuming_item.start);
            let end = item.end.min(consuming_item.end);
            let duration = end.signed_sub(start);
            (duration > Time::ZERO).then_some((duration, start, end))
        })
        .max_by_key(|(duration, _, _)| *duration)
        .map(|(_, start, end)| (start, end))
        .ok_or_else(|| "source visual track does not overlap the 3D item".to_string())?;
    let analysis_duration = analysis_position_end.signed_sub(analysis_position_start);
    let analysis_time_start = analysis_position_start
        .signed_sub(consuming_item.start)
        .saturating_add(consuming_item.animation_time_offset);
    let item_ids = source_track
        .items
        .iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    let mut frame_count = 0_u64;
    loop {
        let numerator = i64::try_from(frame_count)
            .map_err(|_| "camera analysis has too many frames".to_string())?;
        if Time::from_fraction(numerator, i64::from(job.source.settings.analysis_fps))
            >= analysis_duration
        {
            break;
        }
        frame_count += 1;
    }
    if frame_count < 2 {
        return Err("source visual track has fewer than two analysis frames".to_string());
    }
    let colmap = job.source.settings.model == COLMAP_TRACKING_MODEL;
    let request = shrimply_server_client::Tracking3dAnalysisRequest::new(
        job.source.settings.model.clone(),
        frame_count,
        colmap.then(|| {
            match job.source.settings.quality {
                ColmapQuality::Low => "low",
                ColmapQuality::Medium => "medium",
                ColmapQuality::High => "high",
                ColmapQuality::Extreme => "extreme",
            }
            .to_string()
        }),
        colmap.then(|| {
            match job.source.settings.camera_model {
                ColmapCameraModel::SimpleRadial => "simple_radial",
                ColmapCameraModel::Pinhole => "pinhole",
                ColmapCameraModel::OpenCv => "open_cv",
                ColmapCameraModel::OpenCvFisheye => "open_cv_fisheye",
                ColmapCameraModel::Equirectangular => "equirectangular",
            }
            .to_string()
        }),
    );
    let mut archive = tempfile::NamedTempFile::new()
        .map_err(|error| format!("could not create 3D tracking archive: {error}"))?;
    shrimply_server_client::write_tracking_3d_archive_header(archive.as_file_mut(), &request)?;
    let mut renderer = VideoExportRenderer::new(ANALYSIS_AUDIO_SAMPLE_RATE)
        .map_err(|error| format!("could not create source-track renderer: {error}"))?;
    let mut visible_frames = 0_u64;
    for sample_index in 0..frame_count {
        if job.cancellation.load(Ordering::Acquire) {
            return Err(ANALYSIS_CANCELLED.to_string());
        }
        let numerator = i64::try_from(sample_index)
            .map_err(|_| "camera analysis has too many frames".to_string())?;
        let normalized_time =
            Time::from_fraction(numerator, i64::from(job.source.settings.analysis_fps));
        let position = analysis_position_start.saturating_add(normalized_time);
        let gpu = renderer
            .render_items(&job.project, position, 0, &item_ids)
            .map_err(|error| format!("could not render source frame: {error}"))?;
        let mut rgba = Video::new(
            Pixel::RGBA,
            job.project.canvas_size.width,
            job.project.canvas_size.height,
        );
        renderer
            .copy_to_rgba_frame(gpu, &mut rgba)
            .map_err(|error| format!("could not copy source frame: {error}"))?;
        let visible = rgba.data(0).chunks(rgba.stride(0)).any(|row| {
            row[..job.project.canvas_size.width as usize * 4]
                .chunks_exact(4)
                .any(|pixel| pixel[3] != 0)
        });
        let jpeg = visible.then(|| encode_jpeg(&rgba)).transpose()?;
        visible_frames += u64::from(visible);
        shrimply_server_client::write_tracking_3d_archive_frame(
            archive.as_file_mut(),
            sample_index,
            jpeg.as_deref().unwrap_or_default(),
        )?;
        let mut state = state.lock().expect("camera reconstruction state poisoned");
        if let Some(current) = state
            .jobs
            .get_mut(&job.camera_item_id)
            .filter(|current| current.analysis_id == job.analysis_id)
            .filter(|current| matches!(current.status, AnalysisStatus::Analyzing { .. }))
        {
            current.status = AnalysisStatus::Analyzing {
                message: "Preparing frames…".to_string(),
                completed_frames: sample_index + 1,
                total_frames: frame_count,
            };
        }
    }
    if visible_frames < 2 {
        return Err("source visual track rendered fewer than two visible frames".to_string());
    }
    let mut reconstructed = Vec::new();
    let mut server_error = None;
    let mut result_count = None;
    if job.cancellation.load(Ordering::Acquire) {
        return Err(ANALYSIS_CANCELLED.to_string());
    }
    {
        let mut state = state.lock().expect("camera reconstruction state poisoned");
        if let Some(current) = state
            .jobs
            .get_mut(&job.camera_item_id)
            .filter(|current| current.analysis_id == job.analysis_id)
        {
            current.status = AnalysisStatus::Analyzing {
                message: "Sending request…".to_string(),
                completed_frames: 0,
                total_frames: 0,
            };
        }
    }
    shrimply_server_client::analyze_tracking_3d(
        &job.server_url,
        &job.compute_cancellation,
        archive.path(),
        |event| {
            if job.cancellation.load(Ordering::Acquire) {
                return false;
            }
            match event {
                shrimply_server_client::Tracking3dEvent::Queued { position } => {
                    let mut state = state.lock().expect("camera reconstruction state poisoned");
                    if let Some(current) = state
                        .jobs
                        .get_mut(&job.camera_item_id)
                        .filter(|current| current.analysis_id == job.analysis_id)
                    {
                        current.status = AnalysisStatus::Analyzing {
                            message: shrimply_server_client::queued_status(position),
                            completed_frames: 0,
                            total_frames: 0,
                        };
                    }
                    true
                }
                shrimply_server_client::Tracking3dEvent::Progress {
                    message,
                    completed_frames,
                    total_frames,
                } => {
                    let mut state = state.lock().expect("camera reconstruction state poisoned");
                    if let Some(current) = state
                        .jobs
                        .get_mut(&job.camera_item_id)
                        .filter(|current| current.analysis_id == job.analysis_id)
                        .filter(|_| !job.cancellation.load(Ordering::Acquire))
                    {
                        current.status = if message.starts_with("Loading ") {
                            AnalysisStatus::Loading
                        } else {
                            AnalysisStatus::Analyzing {
                                message,
                                completed_frames,
                                total_frames,
                            }
                        };
                    }
                    true
                }
                shrimply_server_client::Tracking3dEvent::Camera(camera) => {
                    reconstructed.push(camera);
                    true
                }
                shrimply_server_client::Tracking3dEvent::Result { camera_count } => {
                    result_count = Some(camera_count);
                    true
                }
                shrimply_server_client::Tracking3dEvent::Error { code, message } => {
                    server_error = Some(format!("{code}: {message}"));
                    false
                }
            }
        },
    )?;
    if job.cancellation.load(Ordering::Acquire) {
        return Err(ANALYSIS_CANCELLED.to_string());
    }
    if let Some(error) = server_error {
        return Err(error);
    }
    if result_count != Some(reconstructed.len() as u64) {
        return Err("3D tracking server returned an inconsistent camera count".to_string());
    }
    reconstructed.sort_by_key(|camera| camera.frame_index);
    let poses = reconstructed
        .iter()
        .map(|camera| camera.pose)
        .collect::<Vec<_>>();
    let normalized = relative_camera_poses(&poses)?;
    let mut samples = Vec::with_capacity(reconstructed.len());
    for (camera, pose) in reconstructed.into_iter().zip(normalized) {
        if camera.frame_index >= frame_count {
            return Err("3D tracking server returned an unknown frame index".to_string());
        }
        let numerator = i64::try_from(camera.frame_index)
            .map_err(|_| "3D tracking frame index is too large".to_string())?;
        let time = analysis_time_start.saturating_add(Time::from_fraction(
            numerator,
            i64::from(job.source.settings.analysis_fps),
        ));
        let (projection, vertical_fov_degrees) = match camera.projection.as_str() {
            "perspective" => (
                Projection::Perspective,
                checked_fov(
                    camera.image_height,
                    camera.focal_y,
                    MAX_PERSPECTIVE_FOV_DEGREES,
                )?,
            ),
            "fisheye" => (
                Projection::Fisheye,
                checked_fov(camera.image_height, camera.focal_y, MAX_FISHEYE_FOV_DEGREES)?,
            ),
            "equirectangular" if camera.focal_y.is_none() => (Projection::Equirectangular, 180.0),
            _ => return Err("3D tracking server returned an invalid projection".to_string()),
        };
        samples.push(validate_sample(TrackSample {
            time,
            pose,
            projection,
            vertical_fov_degrees,
        })?);
    }
    samples.sort_by_key(|sample| sample.time);
    if samples.len() < 2 {
        return Err("3D tracking produced fewer than two cameras".to_string());
    }
    let track = CachedTrack {
        source: job.source.clone(),
        cache_version: CACHE_VERSION,
        samples,
    };
    Ok(track)
}

fn checked_fov(height: u32, focal_y: Option<f64>, maximum: f64) -> Result<f64, String> {
    let focal_y =
        focal_y.ok_or_else(|| "3D tracking camera has no vertical focal length".to_string())?;
    let fov = vertical_fov_degrees_from_focal_length(height, focal_y)?;
    if (MIN_VERTICAL_FOV_DEGREES..=maximum).contains(&fov) {
        Ok(fov)
    } else {
        Err(format!(
            "3D tracking vertical FOV {fov:.2} is outside renderer limits"
        ))
    }
}

fn validate_sample(sample: TrackSample) -> Result<TrackSample, String> {
    let rotation_length = sample.pose.rotation.length_squared();
    if sample.pose.position.is_finite()
        && sample.pose.rotation.is_finite()
        && rotation_length.is_finite()
        && (rotation_length - 1.0).abs() <= 1.0e-6
        && sample.vertical_fov_degrees.is_finite()
        && sample.vertical_fov_degrees > 0.0
    {
        Ok(sample)
    } else {
        Err("camera analysis produced an invalid sample".to_string())
    }
}

fn encode_jpeg(frame: &Video) -> Result<Vec<u8>, String> {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in frame.data(0).chunks(frame.stride(0)).take(height) {
        for pixel in row[..width * 4].chunks_exact(4) {
            let alpha = u16::from(pixel[3]);
            pixels.extend_from_slice(&[
                (u16::from(pixel[0]) * alpha / 255) as u8,
                (u16::from(pixel[1]) * alpha / 255) as u8,
                (u16::from(pixel[2]) * alpha / 255) as u8,
                255,
            ]);
        }
    }
    let image = skia_safe::images::raster_from_data(
        &skia_safe::ImageInfo::new(
            (frame.width() as i32, frame.height() as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Opaque,
            None,
        ),
        skia_safe::Data::new_copy(&pixels),
        width * 4,
    )
    .ok_or_else(|| "could not create 3D tracking proxy image".to_string())?;
    image
        .encode(None, skia_safe::EncodedImageFormat::JPEG, Some(95))
        .map(|data| data.as_bytes().to_vec())
        .ok_or_else(|| "could not encode 3D tracking proxy JPEG".to_string())
}

fn open_database() -> Result<Connection, String> {
    let path = Path::new(CACHE_DATABASE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create camera cache directory: {error}"))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("could not open camera reconstruction cache: {error}"))?;
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(|error| format!("could not configure camera cache timeout: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS camera_analyses (
               camera_item_id TEXT PRIMARY KEY,
               source_track_id TEXT NOT NULL,
               settings_json TEXT NOT NULL,
               cache_version INTEGER NOT NULL,
               sample_count INTEGER NOT NULL,
               updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS camera_samples (
               camera_item_id TEXT NOT NULL,
               sample_index INTEGER NOT NULL,
               time_numerator INTEGER NOT NULL,
               time_denominator INTEGER NOT NULL,
               position_x REAL NOT NULL,
               position_y REAL NOT NULL,
               position_z REAL NOT NULL,
               rotation_x REAL NOT NULL,
               rotation_y REAL NOT NULL,
               rotation_z REAL NOT NULL,
               rotation_w REAL NOT NULL,
               projection TEXT NOT NULL,
               vertical_fov_degrees REAL NOT NULL,
               PRIMARY KEY (camera_item_id, sample_index),
               FOREIGN KEY (camera_item_id) REFERENCES camera_analyses(camera_item_id) ON DELETE CASCADE
             );",
        )
        .map_err(|error| format!("could not initialize camera reconstruction cache: {error}"))?;
    Ok(connection)
}

fn load_database() -> Result<HashMap<Uuid, CachedTrack>, String> {
    let connection = open_database()?;
    let analyses = {
        let mut query = connection
            .prepare(
                "SELECT camera_item_id, source_track_id, settings_json, cache_version, sample_count
                 FROM camera_analyses",
            )
            .map_err(|error| format!("could not read camera cache schema: {error}"))?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| format!("could not query camera cache: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not decode camera cache metadata: {error}"))?
    };
    let mut tracks = HashMap::new();
    for (camera_id, source_id, settings, cache_version, sample_count) in analyses {
        let camera_item_id = Uuid::parse_str(&camera_id)
            .map_err(|error| format!("invalid camera cache item UUID: {error}"))?;
        let source = TrackingCameraSource {
            track_id: Uuid::parse_str(&source_id)
                .map_err(|error| format!("invalid camera cache source UUID: {error}"))?,
            settings: serde_json::from_str(&settings)
                .map_err(|error| format!("invalid camera cache settings: {error}"))?,
        };
        let mut query = connection
            .prepare(
                "SELECT time_numerator, time_denominator, position_x, position_y, position_z,
                        rotation_x, rotation_y, rotation_z, rotation_w, projection,
                        vertical_fov_degrees
                 FROM camera_samples WHERE camera_item_id = ?1 ORDER BY sample_index",
            )
            .map_err(|error| format!("could not read camera samples: {error}"))?;
        let samples = query
            .query_map([&camera_id], |row| {
                let denominator = row.get::<_, i64>(1)?;
                if denominator <= 0 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(TrackSample {
                    time: Time::from_fraction(row.get(0)?, denominator),
                    pose: NormalizedCameraPose {
                        position: glam::DVec3::new(row.get(2)?, row.get(3)?, row.get(4)?),
                        rotation: glam::DQuat::from_xyzw(
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ),
                    },
                    projection: parse_projection(&row.get::<_, String>(9)?)
                        .ok_or(rusqlite::Error::InvalidQuery)?,
                    vertical_fov_degrees: row.get(10)?,
                })
            })
            .map_err(|error| format!("could not query camera samples: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not decode camera samples: {error}"))?;
        if samples.len() != usize::try_from(sample_count).unwrap_or(usize::MAX) {
            return Err(format!(
                "camera cache sample count is inconsistent for {camera_item_id}"
            ));
        }
        let samples = samples
            .into_iter()
            .map(validate_sample)
            .collect::<Result<Vec<_>, _>>()?;
        tracks.insert(
            camera_item_id,
            CachedTrack {
                source,
                cache_version,
                samples,
            },
        );
    }
    Ok(tracks)
}

fn store_track(camera_item_id: Uuid, track: &CachedTrack) -> Result<(), String> {
    let mut connection = open_database()?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("could not begin camera cache transaction: {error}"))?;
    let settings = serde_json::to_string(&track.source.settings)
        .map_err(|error| format!("could not encode camera settings: {error}"))?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO camera_analyses
             (camera_item_id, source_track_id, settings_json, cache_version, sample_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)",
            params![
                camera_item_id.to_string(),
                track.source.track_id.to_string(),
                settings,
                track.cache_version,
                track.samples.len() as i64,
            ],
        )
        .map_err(|error| format!("could not replace camera cache metadata: {error}"))?;
    transaction
        .execute(
            "DELETE FROM camera_samples WHERE camera_item_id = ?1",
            [camera_item_id.to_string()],
        )
        .map_err(|error| format!("could not clear old camera samples: {error}"))?;
    for (index, sample) in track.samples.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO camera_samples
                 (camera_item_id, sample_index, time_numerator, time_denominator,
                  position_x, position_y, position_z, rotation_x, rotation_y, rotation_z,
                  rotation_w, projection, vertical_fov_degrees)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    camera_item_id.to_string(),
                    index as i64,
                    fraction_numerator(sample.time.seconds),
                    fraction_denominator(sample.time.seconds),
                    sample.pose.position.x,
                    sample.pose.position.y,
                    sample.pose.position.z,
                    sample.pose.rotation.x,
                    sample.pose.rotation.y,
                    sample.pose.rotation.z,
                    sample.pose.rotation.w,
                    projection_name(sample.projection),
                    sample.vertical_fov_degrees,
                ],
            )
            .map_err(|error| format!("could not insert camera sample: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("could not commit camera reconstruction cache: {error}"))
}

fn projection_name(projection: Projection) -> &'static str {
    match projection {
        Projection::Perspective => "perspective",
        Projection::Fisheye => "fisheye",
        Projection::Equirectangular => "equirectangular",
        Projection::Orthographic | Projection::Cylindrical => unreachable!(),
    }
}

fn parse_projection(value: &str) -> Option<Projection> {
    match value {
        "perspective" => Some(Projection::Perspective),
        "fisheye" => Some(Projection::Fisheye),
        "equirectangular" => Some(Projection::Equirectangular),
        _ => None,
    }
}
