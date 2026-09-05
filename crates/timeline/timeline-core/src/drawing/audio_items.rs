use super::super::*;
use super::decoration::*;

pub(in crate::drawing) fn draw_audio_item(
    draw: &TimelineDraw<'_>,
    item: &crate::project::AudioItem,
    waveform_start: Time,
    y: f64,
    selected: bool,
) {
    let mut waveform_columns = vec![0.0; draw.timeline_width.ceil() as usize];
    draw_audio_item_into(
        draw,
        item,
        waveform_start,
        y,
        selected,
        true,
        &mut waveform_columns,
    );
    draw_waveform_columns(
        draw.painter,
        &waveform_columns,
        draw.timeline_x,
        y,
        Color::GREEN1,
    );
}

pub(in crate::drawing) fn draw_audio_item_into(
    draw: &TimelineDraw<'_>,
    item: &crate::project::AudioItem,
    waveform_start: Time,
    y: f64,
    selected: bool,
    detailed: bool,
    waveform_columns: &mut [f32],
) {
    let key = waveform::audio_key(item);
    let waveform = match draw.waveforms.get(&key) {
        Some(Some(waveform)) => WaveformState::Loaded(Some(waveform)),
        Some(None) => WaveformState::Loaded(None),
        None => WaveformState::Loading,
    };
    draw_audio_item_with_waveform(
        draw,
        item,
        waveform_start,
        y,
        waveform,
        Color::GREEN5.alpha_multiply(0.55),
        Color::GREEN1,
        selected,
        Color::GREEN1,
        detailed,
        waveform_columns,
    );
}

pub(in crate::drawing) fn draw_live_recording_item(
    draw: &TimelineDraw<'_>,
    recording: &LiveRecordingDraw,
    y: f64,
) {
    let mut waveform_columns = vec![0.0; draw.timeline_width.ceil() as usize];
    draw_audio_item_with_waveform(
        draw,
        &recording.item,
        recording.item.start,
        y,
        WaveformState::Loaded(Some(&recording.waveform)),
        Color::RED5.alpha_multiply(0.65),
        Color::RED1,
        false,
        Color::RED1,
        true,
        &mut waveform_columns,
    );
    draw_waveform_columns(
        draw.painter,
        &waveform_columns,
        draw.timeline_x,
        y,
        Color::RED1,
    );
}

#[allow(clippy::too_many_arguments)]
pub(in crate::drawing) fn draw_audio_item_with_waveform(
    draw: &TimelineDraw<'_>,
    item: &crate::project::AudioItem,
    waveform_start: Time,
    y: f64,
    waveform: WaveformState<'_>,
    fill_color: Color,
    waveform_color: Color,
    selected: bool,
    selected_border_color: Color,
    detailed: bool,
    waveform_columns: &mut [f32],
) {
    let (clip_x, clip_width) = item_rect(item.start, item.end, draw.timeline_x, draw.view);
    if clip_width <= 0.0 || draw.timeline_width <= 0.0 {
        return;
    }

    let clip_end_x = clip_x + clip_width;
    let visible_left = draw.timeline_x;
    let visible_right = draw.timeline_x + draw.timeline_width;
    let visible_clip_x = clip_x.max(visible_left);
    let visible_clip_right = clip_end_x.min(visible_right);
    let visible_clip_width = (visible_clip_right - visible_clip_x).max(0.0);
    if visible_clip_width <= 0.0 || clip_end_x <= visible_left || clip_x >= visible_right {
        return;
    }

    if detailed {
        let marker = if matches!(&item.source, crate::project::AudioSource::Generator(_)) {
            NaturalEndMarker {
                start: item.start,
                end: item.end,
                position: None,
                repeat_interval: None,
                real_start: None,
                real_end: None,
            }
        } else {
            let real_span = media_real_span(
                item.start,
                item.time_offset,
                item.source_duration,
                item.playback_speed,
                item.repeat_strategy,
            );
            NaturalEndMarker {
                start: item.start,
                end: item.end,
                position: media_item_natural_end_position(
                    item.start,
                    item.time_offset,
                    item.source_duration,
                    item.playback_speed,
                    item.repeat_strategy,
                ),
                repeat_interval: media_natural_end_interval(
                    item.source_duration,
                    item.playback_speed,
                    item.repeat_strategy,
                ),
                real_start: real_span.map(|(start, _)| start),
                real_end: real_span.map(|(_, end)| end),
            }
        };
        draw_timed_item_box(
            draw.painter,
            TimedItemBox {
                marker,
                bounds: rect(clip_x, y, clip_width, TRACK_HEIGHT),
                fill: fill_color,
                timeline_x: draw.timeline_x,
                view: draw.view,
                selected,
                selected_border_color,
            },
        );
        draw_natural_end_marker(
            draw.painter,
            marker,
            draw.timeline_x,
            y,
            draw.view,
            selected_border_color,
        );
    }

    draw_audio_item_waveform(
        draw.painter,
        waveform,
        item,
        waveform_start,
        clip_x,
        clip_width,
        visible_clip_x,
        visible_clip_width,
        y,
        draw.waveform_chunks_per_second,
        waveform_color,
        draw.animation_seconds,
        detailed,
        draw.timeline_x,
        waveform_columns,
    );
}

pub(in crate::drawing) fn preview_audio_item(
    source: &crate::project::AudioItem,
    start: Time,
    end: Time,
    mode: PreviewTimeMode,
) -> crate::project::AudioItem {
    let mut preview = source.clone();
    if matches!(mode, PreviewTimeMode::Resize) {
        preview.time_offset = preview_media_time_offset(
            source.time_offset,
            source.start,
            start,
            source.playback_speed,
            source.repeat_strategy,
            source.source_duration,
        );
    }
    preview.start = start;
    preview.end = end;
    preview
}

pub(in crate::drawing) fn preview_media_time_offset(
    offset: Time,
    old_start: Time,
    new_start: Time,
    playback_speed: Fraction,
    _repeat_strategy: RepeatStrategy,
    _source_duration: Time,
) -> Time {
    offset.saturating_add(scaled_time_delta(
        Time {
            seconds: new_start.seconds - old_start.seconds,
        },
        playback_speed,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::drawing) fn draw_audio_item_waveform(
    painter: &TimelinePainter,
    waveform: WaveformState<'_>,
    item: &crate::project::AudioItem,
    waveform_start: Time,
    item_clip_x: f64,
    item_clip_width: f64,
    clip_x: f64,
    clip_width: f64,
    y: f64,
    waveform_chunks_per_second: u32,
    color: Color,
    animation_seconds: f64,
    detailed: bool,
    timeline_x: f64,
    columns: &mut [f32],
) {
    let baseline_y = waveform_baseline_y(y);
    let waveform = match waveform {
        WaveformState::Loading if !detailed => {
            fill_waveform_columns(columns, timeline_x, clip_x, clip_width, 1.0);
            return;
        }
        WaveformState::Loading => {
            draw_loading_waveform(painter, clip_x, clip_width, y, animation_seconds);
            return;
        }
        WaveformState::Loaded(Some(waveform)) => waveform,
        WaveformState::Loaded(None) if !detailed => {
            fill_waveform_columns(columns, timeline_x, clip_x, clip_width, 1.0);
            return;
        }
        WaveformState::Loaded(None) => {
            draw_empty_waveform(painter, clip_x, clip_width, baseline_y, color);
            return;
        }
    };

    if waveform.peaks.is_empty() || waveform.max_peak == 0 && !waveform.has_pending() {
        if detailed {
            draw_empty_waveform(painter, clip_x, clip_width, baseline_y, color);
        } else {
            fill_waveform_columns(columns, timeline_x, clip_x, clip_width, 1.0);
        }
        return;
    }

    let max_height = TRACK_HEIGHT - 6.0;
    let first_column = ((clip_x - timeline_x).floor().max(0.0) as usize).min(columns.len());
    let end_column =
        ((clip_x + clip_width - timeline_x).ceil().max(0.0) as usize).min(columns.len());
    if first_column >= end_column {
        return;
    }
    let item_duration_nanos = item.end.as_nanos_i128() - item.start.as_nanos_i128();
    for (column, height) in columns
        .iter_mut()
        .enumerate()
        .take(end_column)
        .skip(first_column)
    {
        let bar_left = (timeline_x + column as f64).max(clip_x);
        let bar_right = (timeline_x + column as f64 + 1.0).min(clip_x + clip_width);
        let left_ratio = ((bar_left - item_clip_x) / item_clip_width).clamp(0.0, 1.0);
        let right_ratio = ((bar_right - item_clip_x) / item_clip_width).clamp(0.0, 1.0);
        let left_time = Time::from_nanos_i128(
            item.start.as_nanos_i128() + (item_duration_nanos as f64 * left_ratio).round() as i128,
        );
        let right_time = Time::from_nanos_i128(
            item.start.as_nanos_i128() + (item_duration_nanos as f64 * right_ratio).round() as i128,
        );
        let Some(peak) = waveform_peak_for_timeline_range(
            waveform,
            waveform_start,
            left_time,
            right_time,
            waveform_chunks_per_second,
        ) else {
            if detailed {
                draw_loading_waveform_bar(
                    painter,
                    bar_left,
                    bar_right - bar_left,
                    item_clip_x,
                    y,
                    animation_seconds,
                );
            } else {
                *height = (*height).max(1.0);
            }
            continue;
        };
        let midpoint = Time::from_nanos_i128(
            left_time.as_nanos_i128()
                + (right_time.as_nanos_i128() - left_time.as_nanos_i128()) / 2,
        );
        let normalized = absolute_waveform_height(peak * f64::from(item.transition_gain(midpoint)));
        if normalized == 0.0 {
            continue;
        }
        *height = (*height).max((normalized.sqrt() * max_height).max(1.0) as f32);
    }
}

pub(in crate::drawing) fn fill_waveform_columns(
    columns: &mut [f32],
    timeline_x: f64,
    clip_x: f64,
    clip_width: f64,
    height: f32,
) {
    let first = ((clip_x - timeline_x).floor().max(0.0) as usize).min(columns.len());
    let end = ((clip_x + clip_width - timeline_x).ceil().max(0.0) as usize).min(columns.len());
    for column in &mut columns[first..end] {
        *column = (*column).max(height);
    }
}

pub(in crate::drawing) fn draw_waveform_columns(
    painter: &TimelinePainter,
    columns: &[f32],
    timeline_x: f64,
    y: f64,
    color: Color,
) {
    let baseline_y = waveform_baseline_y(y) as f32;
    let mut segments = Vec::with_capacity(columns.len() * 2);
    for (column, height) in columns.iter().copied().enumerate() {
        if height <= 0.0 {
            continue;
        }
        let x = (timeline_x + column as f64 + 0.5) as f32;
        segments.extend([vec2(x, baseline_y - height), vec2(x, baseline_y)]);
    }
    painter.line_segments(&segments, Stroke::new(1.0, color));
}

pub(in crate::drawing) fn waveform_peak_for_timeline_range(
    waveform: &waveform::Waveform,
    waveform_start: Time,
    timeline_start: Time,
    timeline_end: Time,
    waveform_chunks_per_second: u32,
) -> Option<f64> {
    let item_start = timeline_start.saturating_sub(waveform_start);
    let item_end = timeline_end.saturating_sub(waveform_start);
    let start_index = item_start.as_secs_f64().max(0.0) * f64::from(waveform_chunks_per_second);
    let end_index = item_end.as_secs_f64().max(0.0) * f64::from(waveform_chunks_per_second);
    let left = start_index.min(end_index);
    let right = start_index.max(end_index);
    waveform.range(left, right)
}

pub(in crate::drawing) fn absolute_waveform_height(peak: f64) -> f64 {
    (peak / u8::MAX as f64).clamp(0.0, 1.0)
}

pub(in crate::drawing) fn waveform_baseline_y(y: f64) -> f64 {
    y + TRACK_HEIGHT - 3.0
}

pub(in crate::drawing) fn draw_empty_waveform(
    painter: &TimelinePainter,
    clip_x: f64,
    clip_width: f64,
    baseline_y: f64,
    color: Color,
) {
    painter.rect_filled(
        rect(
            clip_x + ITEM_PADDING_X,
            baseline_y,
            (clip_width - ITEM_PADDING_X * 2.0).max(1.0),
            1.0,
        ),
        0,
        color,
    );
}

pub(in crate::drawing) fn draw_loading_waveform(
    painter: &TimelinePainter,
    clip_x: f64,
    clip_width: f64,
    y: f64,
    animation_seconds: f64,
) {
    let left = clip_x + ITEM_PADDING_X;
    let width = (clip_width - ITEM_PADDING_X * 2.0).max(1.0);
    let baseline_y = waveform_baseline_y(y);
    painter.rect_filled(
        rect(left, baseline_y, width, 1.0),
        0,
        Color::new(0.42, 0.58, 0.48, 1.0),
    );

    let bar_width = 4.0f64.min(width);
    let bar_spacing = 8.0;
    let bar_count = ((width / bar_spacing).ceil() as usize).clamp(8, 240);
    for index in 0..bar_count {
        let mut bar_x = left + index as f64 * bar_spacing;
        if bar_x > left + width - bar_width {
            bar_x = left + width - bar_width;
        }
        if bar_x > left + width {
            break;
        }
        draw_loading_waveform_bar(
            painter,
            bar_x.min(left + width - bar_width),
            bar_width,
            left,
            y,
            animation_seconds,
        );
    }
}

pub(in crate::drawing) fn draw_loading_waveform_bar(
    painter: &TimelinePainter,
    x: f64,
    width: f64,
    wave_origin: f64,
    y: f64,
    animation_seconds: f64,
) {
    let phase = ((x - wave_origin) / 34.0 - animation_seconds * 1.4) * std::f64::consts::TAU;
    let pulse = (phase.sin() + 1.0) * 0.5;
    let height = 3.0 + pulse * (TRACK_HEIGHT - 15.0).max(1.0);
    let baseline_y = waveform_baseline_y(y);
    painter.rect_filled(
        rect(x, baseline_y - height, width.max(1.0), height),
        0,
        Color::new(0.56, 0.78, 0.64, (0.32 + 0.68 * pulse) as f32),
    );
}

pub fn item_rect(start: Time, end: Time, x: f64, view: TimelineViewState) -> (f64, f64) {
    let item_x = time_to_x(start.as_secs_f64(), x, view);
    let item_width = ((end.as_secs_f64() - start.as_secs_f64()) / view.seconds_per_pixel).max(1.0);
    (item_x, item_width)
}
