//! Unpaced production-worker throughput: each requested frame must finish on the GPU.
use crate::{
    CompositeAccuracy,
    worker::{
        PlaybackRenderEvent, PlaybackRenderObserver, VideoCommand, VideoCommandSender, VideoEvent,
    },
};
use shrimply_math_core::{Fraction, frame_rate_from_duration, percentile, time_from_frame};
use shrimply_project_document::Project;
use std::{
    collections::VecDeque,
    fmt::Write,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

const WORKER_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Report<F> {
    pub text: String,
    pub frame: F,
}

enum Event<F> {
    Submitted(shrimply_math_core::Time),
    Render(VideoEvent<F>),
}

/// Real native clips, kept in memory: benchmark fixtures never edit project files.
pub fn scene(mut project: Project, kind: &str) -> Result<Project, String> {
    use shrimply_project_document::{Time, VideoItem, VisualTrack};
    use shrimply_property_model::timeline_value::{
        TimelineBase, TimelineCurveKeyframe, TimelineValue,
    };
    let (animated, kind) = kind
        .strip_prefix("animated-")
        .map_or((false, kind), |kind| (true, kind));
    let (video, text, shape) = match kind {
        "project" => return Ok(project),
        "text" => (false, true, false),
        "shape" => (false, false, true),
        "video-text" => (true, true, false),
        "video-shape" => (true, false, true),
        "text-shape" => (false, true, true),
        _ => {
            return Err(
                "Scene must be project, text, shape, video-text, video-shape or text-shape".into(),
            );
        }
    };
    let end = project.duration();
    project.audio_tracks.clear();
    project.caption_tracks.clear();
    if !video {
        project.video_tracks.clear();
    }
    let first_generated = project.video_tracks.len();
    if text {
        project.video_tracks.push(VisualTrack {
            items: vec![VideoItem::text_item(project.canvas_size, Time::ZERO, end)],
            ..Default::default()
        });
    }
    if shape {
        project.video_tracks.push(VisualTrack {
            items: vec![VideoItem::shape_item(project.canvas_size, Time::ZERO, end)],
            ..Default::default()
        });
    }
    for (index, track) in project.video_tracks[first_generated..]
        .iter_mut()
        .enumerate()
    {
        let item = &mut track.items[0];
        if text && shape {
            item.transform.position = TimelineValue::new_const(glam::vec2(
                (index + 1) as f32 * project.canvas_size.width as f32 / 3.0,
                project.canvas_size.height as f32 / 2.0,
            ));
        }
        if animated {
            item.transform.rotation_degrees.base = TimelineBase::Keyframes(
                [(Time::ZERO, 0.0), (end, 360.0)]
                    .into_iter()
                    .map(|(time, value)| TimelineCurveKeyframe {
                        id: uuid::Uuid::new_v4(),
                        time,
                        value,
                        interpolation_to_next: shrimply_project_document::Interpolation::Linear,
                    })
                    .collect(),
            );
        }
    }
    project.ensure_ids();
    project.validate()?;
    Ok(project)
}

pub fn run<F: Send + 'static>(
    project: Project,
    count: u64,
    spawn: impl FnOnce(Project, PlaybackRenderObserver) -> (VideoCommandSender, Receiver<VideoEvent<F>>),
    complete: impl Fn(&F) -> Result<(), String>,
) -> Result<Report<F>, String> {
    if count < 2 {
        return Err("Benchmark needs at least two frames".into());
    }
    let fps = project.fps;
    let last = time_from_frame(count - 1, fps).ok_or("Invalid frame position")?;
    if last >= project.duration() {
        return Err("Benchmark exceeds project duration".into());
    }
    let (sender, events) = mpsc::channel();
    let submitted_tx = sender.clone();
    let observer = Arc::new(move |event| {
        if let PlaybackRenderEvent::Submitted { position, .. } = event {
            let _ = submitted_tx.send(Event::Submitted(position));
        }
    });
    let (commands, rendered) = spawn(project, observer);
    let forwarding = std::thread::spawn(move || {
        for event in rendered {
            if sender.send(Event::Render(event)).is_err() {
                break;
            }
        }
    });
    let result = (|| {
        let mut report = String::new();
        let mut completed_frame = None;
        let mut submitted = VecDeque::new();
        for pass in ["Cold", "Warm"] {
            shrimply_profiling::clear();
            let started = Instant::now();
            let mut samples = Vec::new();
            let mut first_frame = Duration::ZERO;
            let send_frame = |index| {
                let position = time_from_frame(index, fps).ok_or("Invalid frame position")?;
                commands
                    .send(VideoCommand::Render {
                        position,
                        accuracy: CompositeAccuracy::CONTINUOUS_FULLY_ACCURATE,
                    })
                    .map_err(|e| e.to_string())
            };
            let mut frame_started = Instant::now();
            send_frame(0)?;
            for index in 0..count {
                let position = time_from_frame(index, fps).ok_or("Invalid frame position")?;
                loop {
                    let position_submitted = if let Some(position) = submitted.pop_front() {
                        position
                    } else {
                        match events
                            .recv_timeout(WORKER_TIMEOUT)
                            .map_err(|error| format!("{pass} frame {index} submission: {error}"))?
                        {
                            Event::Submitted(position) => position,
                            Event::Render(VideoEvent::Error(error)) => return Err(error),
                            Event::Render(VideoEvent::Clear { .. }) => {
                                return Err("No visible benchmark frame".into());
                            }
                            _ => continue,
                        }
                    };
                    if position_submitted == position {
                        break;
                    }
                }
                // One queued successor overlaps source/CPU preparation with the current GPU frame.
                let next_started = Instant::now();
                if index + 1 < count {
                    send_frame(index + 1)?;
                }
                loop {
                    let event = match events
                        .recv_timeout(WORKER_TIMEOUT)
                        .map_err(|e| format!("{pass} frame {index}: {e}"))?
                    {
                        Event::Submitted(position) => {
                            submitted.push_back(position);
                            continue;
                        }
                        Event::Render(event) => event,
                    };
                    match event {
                        VideoEvent::Error(error) => return Err(error),
                        VideoEvent::Frame {
                            position: rendered,
                            frame,
                            settled: true,
                            ..
                        } if rendered == position => {
                            complete(&frame)?;
                            completed_frame = Some(frame);
                            break;
                        }
                        VideoEvent::Clear {
                            position: rendered, ..
                        } if rendered == position => {
                            return Err(format!("No visible frame at {}", position.as_label()));
                        }
                        _ => {}
                    }
                }
                if index == 0 {
                    first_frame = started.elapsed();
                } else {
                    samples.push(frame_started.elapsed().as_nanos());
                }
                frame_started = next_started;
            }
            let elapsed = started.elapsed();
            let steady = elapsed.saturating_sub(first_frame);
            samples.sort_unstable();
            let rate = frame_rate_from_duration(elapsed).ok_or("Empty benchmark duration")?
                * Fraction::from(count);
            let steady_rate = frame_rate_from_duration(steady).ok_or("Empty steady duration")?
                * Fraction::from(count - 1);
            writeln!(report, "pipeline pass={pass} frames={count} elapsed_ns={} fps={rate} steady_fps={steady_rate} first_frame_ns={} p50_ns={} p95_ns={} max_ns={}",
                elapsed.as_nanos(), first_frame.as_nanos(), percentile(&samples, 50), percentile(&samples, 95), samples.last().expect("frame samples")).expect("write benchmark report");
            writeln!(
                report,
                "profile pass={pass} {}",
                shrimply_profiling::report_json()
            )
            .expect("write profile");
        }
        Ok(Report {
            text: report,
            frame: completed_frame.expect("completed benchmark frame"),
        })
    })();
    // Stop on errors as well; never leave a benchmark renderer running in the app.
    let _ = commands.send(VideoCommand::Stop);
    drop(commands);
    let mut result = result;
    loop {
        match events.recv_timeout(WORKER_TIMEOUT) {
            Ok(Event::Render(VideoEvent::Error(error))) => result = Err(error),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(error) => return Err(format!("GPU worker shutdown: {error}")),
        }
    }
    forwarding
        .join()
        .expect("benchmark event forwarding panicked");
    result
}
