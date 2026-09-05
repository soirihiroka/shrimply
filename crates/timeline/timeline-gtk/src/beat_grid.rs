use crate::audio::beat::{self, BeatMap, BeatState, MarkerKind};
use crate::project::{Project, Time, fraction_as_f64, playback_speed_or_default};

use super::items::{TrackKind, row_for_track};
use super::renderer::{Align2, Color, FontId, Stroke, TimelinePainter, vec2};
use super::{RULER_HEIGHT, TimelineViewState, row_screen_y, time_to_x, timeline_x};

const BEAT_MIN_SPACING_PX: f64 = 12.0;
const BAR_MIN_SPACING_PX: f64 = 6.0;

#[derive(Clone, Copy)]
struct GridMarker {
    time: Time,
    kind: MarkerKind,
    track_index: usize,
}

pub(super) fn snap_targets(
    project: &Project,
    beats: &BeatMap,
    view: TimelineViewState,
) -> Vec<Time> {
    markers(project, beats, view, None)
        .into_iter()
        .map(|marker| marker.time)
        .collect()
}

pub(super) fn draw(
    painter: &TimelinePainter,
    project: &Project,
    beats: &BeatMap,
    view: TimelineViewState,
    timeline_width: f64,
    content_height: f64,
) {
    let visible_start = Time::from_seconds_f64(view.scroll_seconds.max(0.0));
    let visible_end = Time::from_seconds_f64(
        view.scroll_seconds + timeline_width.max(0.0) * view.seconds_per_pixel,
    );
    let markers = markers(project, beats, view, Some((visible_start, visible_end)));
    let left = timeline_x();
    let right = left + timeline_width;
    for marker in markers {
        let x = time_to_x(marker.time.as_secs_f64(), left, view);
        if x < left || x > right {
            continue;
        }
        match marker.kind {
            MarkerKind::Bar => {
                painter.line_segment(
                    [
                        vec2(x as f32, RULER_HEIGHT as f32),
                        vec2(x as f32, content_height as f32),
                    ],
                    Stroke::new(1.5, Color::new(0.32, 0.90, 0.48, 0.72)),
                );
                if let Some(row) = row_for_track(project, TrackKind::Audio, marker.track_index) {
                    painter.circle_filled(
                        vec2(x as f32, (row_screen_y(row, view) + 4.0) as f32),
                        2.5,
                        Color::new(0.42, 0.98, 0.56, 1.0),
                    );
                }
            }
            MarkerKind::Beat => draw_dashed_line(
                painter,
                x,
                RULER_HEIGHT,
                content_height,
                Stroke::new(1.0, Color::new(0.32, 0.90, 0.48, 0.46)),
            ),
        }
    }
    draw_statuses(painter, project, beats, view, left, right);
}

fn markers(
    project: &Project,
    beats: &BeatMap,
    view: TimelineViewState,
    visible_range: Option<(Time, Time)>,
) -> Vec<GridMarker> {
    let mut markers = Vec::new();
    let frame_step = shrimply_math_core::time_from_frame(1, project.fps)
        .expect("project frame rate must be positive");
    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        for item in &track.items {
            if !item.beat_detection {
                continue;
            }
            let Some(BeatState::Ready(analysis)) = beats.get(&item.id) else {
                continue;
            };
            let speed = fraction_as_f64(playback_speed_or_default(item.playback_speed)).abs();
            if speed <= f64::EPSILON || analysis.sample_rate == 0 {
                continue;
            }
            let beat_spacing_px = analysis.period_frames as f64
                / f64::from(analysis.sample_rate)
                / speed
                / view.seconds_per_pixel;
            let show_beats = beat_spacing_px >= BEAT_MIN_SPACING_PX;
            let show_bars =
                analysis.bar_phase.is_some() && beat_spacing_px * 4.0 >= BAR_MIN_SPACING_PX;
            let (visible_start, visible_end) = visible_range.unwrap_or((item.start, item.end));
            for marker in beat::timeline_markers(item, analysis, visible_start, visible_end) {
                if marker.kind == MarkerKind::Beat && !show_beats
                    || marker.kind == MarkerKind::Bar && !show_bars
                {
                    continue;
                }
                markers.push(GridMarker {
                    time: marker.time.snapped(frame_step),
                    kind: marker.kind,
                    track_index,
                });
            }
        }
    }
    markers.sort_by(|left, right| {
        left.time
            .cmp(&right.time)
            .then_with(|| marker_priority(right.kind).cmp(&marker_priority(left.kind)))
    });
    markers.dedup_by(|right, left| {
        if right.time == left.time && right.track_index == left.track_index {
            if marker_priority(right.kind) > marker_priority(left.kind) {
                left.kind = right.kind;
            }
            true
        } else {
            false
        }
    });
    markers
}

fn marker_priority(kind: MarkerKind) -> u8 {
    match kind {
        MarkerKind::Beat => 0,
        MarkerKind::Bar => 1,
    }
}

fn draw_dashed_line(painter: &TimelinePainter, x: f64, start_y: f64, end_y: f64, stroke: Stroke) {
    let mut y = start_y;
    while y < end_y {
        let segment_end = (y + 4.0).min(end_y);
        painter.line_segment(
            [vec2(x as f32, y as f32), vec2(x as f32, segment_end as f32)],
            stroke,
        );
        y += 8.0;
    }
}

fn draw_statuses(
    painter: &TimelinePainter,
    project: &Project,
    beats: &BeatMap,
    view: TimelineViewState,
    left: f64,
    right: f64,
) {
    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        let Some(row) = row_for_track(project, TrackKind::Audio, track_index) else {
            continue;
        };
        let y = row_screen_y(row, view) + 5.0;
        for item in track.items.iter().filter(|item| item.beat_detection) {
            let x = time_to_x(item.start.as_secs_f64(), left, view) + 6.0;
            if x < left || x > right {
                continue;
            }
            let Some((label, color)) = beats.get(&item.id).and_then(|state| match state {
                BeatState::Loading => Some((
                    "Analyzing beats…".to_string(),
                    Color::new(0.70, 0.88, 0.74, 1.0),
                )),
                BeatState::LowConfidence => Some((
                    "No reliable beat detected".to_string(),
                    Color::new(1.0, 0.70, 0.24, 1.0),
                )),
                BeatState::Failed(error) => Some((
                    format!("Beat analysis failed: {error}"),
                    Color::new(1.0, 0.42, 0.34, 1.0),
                )),
                BeatState::Ready(_) => None,
            }) else {
                continue;
            };
            painter.text(
                vec2(x as f32, y as f32),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(10.0),
                color,
            );
        }
    }
}
