use hashbrown::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub use shrimply_math_color::{Color, LayerBlendMode};
pub use shrimply_math_core::{
    Fraction, GenericFraction, Sign, Time, fraction_denominator, fraction_numerator,
};
use shrimply_math_core::{nonnegative_frame_index, time_from_frame};
pub use shrimply_math_geometry::{
    EllipseSegment, Rect, arrow_vertices, cross_vertices, ellipse_segment, fit_vertices,
    regular_polygon_vertices, star_vertices,
};
use shrimply_project_core::{AudioClipTransitionCurve, CanvasSize, TransitionSide};
pub use shrimply_render_core::math::*;

pub fn background_noise_epoch(position: Time, interval_seconds: f32) -> u32 {
    const MIN_INTERVAL_SECONDS: f32 = 0.001;
    let interval_seconds = interval_seconds.max(MIN_INTERVAL_SECONDS);
    if interval_seconds.is_infinite() {
        return 0;
    }
    let rate =
        Fraction::from(1u8) / shrimply_math_core::fraction_from_f64(f64::from(interval_seconds));
    nonnegative_frame_index(position.max(Time::ZERO), rate)
        .unwrap_or(u64::MAX)
        .min(u64::from(u32::MAX)) as u32
}

pub fn gib_to_bytes(gib: Fraction) -> u64 {
    let GenericFraction::Rational(sign, ratio) = gib else {
        return 0;
    };
    if sign == Sign::Minus {
        return 0;
    }
    u128::from(*ratio.numer())
        .saturating_mul(u128::from(1024_u64.pow(3)))
        .checked_div(u128::from(*ratio.denom()).max(1))
        .unwrap_or(0)
        .min(u128::from(u64::MAX)) as u64
}

type VolumeResolver = dyn Fn(&[usize]) -> VolumeValue + Send + Sync;

#[derive(Clone, Debug)]
pub enum VolumeValue {
    Ready(f32),
    Pending,
    Failed(String),
}

#[derive(Clone)]
pub struct FrameVolumeMixer {
    track_count: usize,
    resolver: Arc<VolumeResolver>,
    deferred_resolver: Option<Arc<VolumeResolver>>,
    resolved: Arc<Mutex<HashMap<Vec<usize>, VolumeValue>>>,
    pending: Arc<AtomicBool>,
    frame: Arc<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeSelectionError {
    Duplicate(usize),
    OutOfRange { index: usize, track_count: usize },
}

impl std::fmt::Display for VolumeSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(index) => {
                write!(formatter, "audio track {index} was selected more than once")
            }
            Self::OutOfRange { index, track_count } => write!(
                formatter,
                "audio track {index} is out of range for {track_count} tracks"
            ),
        }
    }
}

impl std::error::Error for VolumeSelectionError {}

impl FrameVolumeMixer {
    pub fn resolving(
        track_count: usize,
        resolver: impl Fn(&[usize]) -> f32 + Send + Sync + 'static,
    ) -> Self {
        Self {
            track_count,
            resolver: Arc::new(move |indices| VolumeValue::Ready(resolver(indices))),
            deferred_resolver: None,
            resolved: Default::default(),
            pending: Default::default(),
            frame: Arc::new(()),
        }
    }

    pub fn with_deferred_resolver(
        mut self,
        resolver: impl Fn(&[usize]) -> VolumeValue + Send + Sync + 'static,
    ) -> Self {
        self.deferred_resolver = Some(Arc::new(resolver));
        self
    }

    /// A nonblocking view shares completed values with the render worker. Each
    /// preparation gets its own pending flag, so incomplete results cannot become
    /// accepted geometry and retries can observe newly completed queries.
    pub fn for_preparation(&self) -> Self {
        Self {
            resolver: self
                .deferred_resolver
                .clone()
                .unwrap_or_else(|| self.resolver.clone()),
            pending: Default::default(),
            ..self.clone()
        }
    }

    pub fn pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed)
    }

    pub fn failures(&self) -> Vec<String> {
        self.resolved
            .lock()
            .expect("frame volume cache mutex poisoned")
            .values()
            .filter_map(|value| match value {
                VolumeValue::Failed(error) => Some(error.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn silent(track_count: usize) -> Self {
        Self::resolving(track_count, |_| 0.0)
    }

    pub fn all(&self) -> VolumeValue {
        self.resolve((0..self.track_count).collect())
    }

    pub fn selected(&self, indices: &[usize]) -> Result<VolumeValue, VolumeSelectionError> {
        let mut selected = HashSet::with_capacity(indices.len());
        for &index in indices {
            if index >= self.track_count {
                return Err(VolumeSelectionError::OutOfRange {
                    index,
                    track_count: self.track_count,
                });
            }
            if !selected.insert(index) {
                return Err(VolumeSelectionError::Duplicate(index));
            }
        }

        let mut indices = indices.to_vec();
        indices.sort_unstable();
        Ok(self.resolve(indices))
    }

    fn resolve(&self, indices: Vec<usize>) -> VolumeValue {
        if let Some(volume) = self
            .resolved
            .lock()
            .expect("frame volume cache mutex poisoned")
            .get(&indices)
            .cloned()
        {
            return volume;
        }

        let volume = (self.resolver)(&indices);
        if matches!(volume, VolumeValue::Pending) {
            self.pending.store(true, Ordering::Relaxed);
        } else {
            self.resolved
                .lock()
                .expect("frame volume cache mutex poisoned")
                .insert(indices, volume.clone());
        }
        volume
    }

    pub fn same_frame(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
    }
}

impl std::fmt::Debug for FrameVolumeMixer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameVolumeMixer")
            .field("track_count", &self.track_count)
            .finish_non_exhaustive()
    }
}

impl Default for FrameVolumeMixer {
    fn default() -> Self {
        Self::silent(0)
    }
}

pub fn time_midpoint(start: Time, end: Time) -> Time {
    Time {
        seconds: start.seconds + (end.seconds - start.seconds) / 2,
    }
}

#[derive(Clone, Copy)]
pub struct PolygonCorner {
    pub entry: glam::Vec2,
    pub exit: glam::Vec2,
    pub conic_weight: f32,
}

pub fn polygon_corners(
    vertices: &[glam::Vec2],
    corner_radius: f32,
    circular: bool,
) -> Option<Vec<PolygonCorner>> {
    if vertices.len() < 3 {
        return None;
    }
    vertices
        .iter()
        .enumerate()
        .map(|(index, &corner)| {
            let previous = vertices[(index + vertices.len() - 1) % vertices.len()];
            let next = vertices[(index + 1) % vertices.len()];
            let previous_edge = previous - corner;
            let next_edge = next - corner;
            let previous_length = previous_edge.length();
            let next_length = next_edge.length();
            if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
                return None;
            }
            let half_angle = corner_half_angle(previous_edge, next_edge);
            let distance = if circular {
                let tangent = half_angle.tan();
                if tangent <= f32::EPSILON {
                    return None;
                }
                corner_radius / tangent
            } else {
                corner_radius
            }
            .min(previous_length * 0.45)
            .min(next_length * 0.45);
            if distance <= f32::EPSILON {
                return None;
            }
            Some(PolygonCorner {
                entry: corner + previous_edge * (distance / previous_length),
                exit: corner + next_edge * (distance / next_length),
                conic_weight: if circular {
                    half_angle.cos().max(f32::EPSILON)
                } else {
                    1.0
                },
            })
        })
        .collect()
}

pub fn corner_conic_weight(previous_edge: glam::Vec2, next_edge: glam::Vec2) -> f32 {
    corner_half_angle(previous_edge, next_edge)
        .cos()
        .max(f32::EPSILON)
}

fn corner_half_angle(previous_edge: glam::Vec2, next_edge: glam::Vec2) -> f32 {
    (previous_edge.dot(next_edge) / (previous_edge.length() * next_edge.length()))
        .clamp(-1.0, 1.0)
        .acos()
        * 0.5
}

pub fn milliseconds_f32_to_nanoseconds(milliseconds: f32) -> u64 {
    (milliseconds.max(0.0) * 1_000_000.0).round() as u64
}

pub fn playback_speed_scale_position(speed: f64) -> f64 {
    speed.clamp(0.25, 4.0).log2()
}

pub fn playback_speed_from_scale_position(position: f64) -> f64 {
    2.0_f64.powf(position)
}

pub fn is_pixel_aligned_translation(matrix: glam::Mat3) -> bool {
    const EPSILON: f32 = 0.000_1;
    let values = matrix.to_cols_array();
    (values[0] - 1.0).abs() <= EPSILON
        && values[1].abs() <= EPSILON
        && values[2].abs() <= EPSILON
        && values[3].abs() <= EPSILON
        && (values[4] - 1.0).abs() <= EPSILON
        && values[5].abs() <= EPSILON
        && (values[6] - values[6].round()).abs() <= EPSILON
        && (values[7] - values[7].round()).abs() <= EPSILON
        && (values[8] - 1.0).abs() <= EPSILON
}

pub fn frames_per_second_milli(frames: u64, elapsed: std::time::Duration) -> u64 {
    if frames == 0 || elapsed.is_zero() {
        return 0;
    }
    (u128::from(frames) * 1_000_000_000_000 / elapsed.as_nanos()).min(u128::from(u64::MAX)) as u64
}

pub fn duration_for_frames_at_millifps(frames: u64, fps_milli: u64) -> std::time::Duration {
    if fps_milli == 0 {
        return std::time::Duration::ZERO;
    }
    let nanoseconds =
        (u128::from(frames) * 1_000_000_000_000 / u128::from(fps_milli)).min(u128::from(u64::MAX));
    std::time::Duration::from_nanos(nanoseconds as u64)
}

pub fn motion_blur_sample_positions(
    position: Time,
    item_start: Time,
    item_end: Time,
    fps: Fraction,
    shutter_angle_degrees: u32,
    shutter_phase_degrees: i32,
    samples: u32,
) -> Vec<Time> {
    let fps_numerator = fraction_numerator(fps);
    let fps_denominator = fraction_denominator(fps);
    if fps_numerator <= 0 || fps_denominator <= 0 || samples == 0 {
        return vec![position.clamp(item_start, item_end)];
    }

    let frame_duration = Time::from_fraction(fps_denominator, fps_numerator).seconds;
    let shutter_start = position.seconds
        + frame_duration * Time::from_fraction(i64::from(shutter_phase_degrees), 360).seconds;
    let shutter_duration =
        frame_duration * Time::from_fraction(i64::from(shutter_angle_degrees), 360).seconds;
    let denominator = i64::from(samples) * 2;
    (0..samples)
        .map(|sample| {
            let midpoint = Time::from_fraction(i64::from(sample) * 2 + 1, denominator).seconds;
            Time {
                seconds: shutter_start + shutter_duration * midpoint,
            }
            .clamp(item_start, item_end)
        })
        .collect()
}

pub fn timeline_sample_frame_spans(
    position: Time,
    fps: Fraction,
    sample_rate: u32,
    count: usize,
) -> Option<Vec<(u64, u64)>> {
    let first_frame = nonnegative_frame_index(position, fps)?;
    Some(
        (0..count)
            .map(|offset| {
                let frame = first_frame.saturating_add(offset as u64);
                (
                    time_from_frame(frame, fps)
                        .expect("valid frame rate must produce a sample span")
                        .as_sample_frame(sample_rate),
                    time_from_frame(frame.saturating_add(1), fps)
                        .expect("valid frame rate must produce a sample span")
                        .as_sample_frame(sample_rate),
                )
            })
            .collect(),
    )
}

pub fn peak_amplitude(samples: &[f32]) -> f32 {
    samples
        .iter()
        .filter(|sample| sample.is_finite())
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        .clamp(0.0, 1.0)
}

pub fn lagged_transition_progress(
    progress: f32,
    index: usize,
    count: usize,
    lag_ratio: f32,
) -> f32 {
    let full_length = count.saturating_sub(1) as f32 * lag_ratio + 1.0;
    progress * full_length - index as f32 * lag_ratio
}

pub fn drawing_stroke_progresses(
    progress: f32,
    lengths: &[f32],
    length_weight: f32,
    overlap: f32,
) -> Vec<f32> {
    if lengths.is_empty() {
        return Vec::new();
    }
    let length_weight = length_weight.clamp(0.0, 1.0);
    let overlap = overlap.clamp(-1.0, 1.0);
    let mean_length = lengths.iter().copied().sum::<f32>() / lengths.len() as f32;
    let durations: Vec<_> = lengths
        .iter()
        .map(|length| {
            let proportional = if mean_length > f32::EPSILON {
                length.max(0.0) / mean_length
            } else {
                1.0
            };
            lerp(1.0, proportional, length_weight).max(f32::EPSILON)
        })
        .collect();
    let mut starts = Vec::with_capacity(durations.len());
    starts.push(0.0);
    for duration in &durations[..durations.len() - 1] {
        starts.push(starts.last().copied().unwrap_or(0.0) + duration * (1.0 - overlap));
    }
    let schedule_duration = starts
        .iter()
        .zip(&durations)
        .map(|(start, duration)| start + duration)
        .fold(f32::EPSILON, f32::max);
    let position = progress.clamp(0.0, 1.0) * schedule_duration;
    starts
        .into_iter()
        .zip(durations)
        .map(|(start, duration)| ((position - start) / duration).clamp(0.0, 1.0))
        .collect()
}

pub fn closest_cyclic_shift(from: &[glam::Vec2], to: &[glam::Vec2]) -> usize {
    if from.len() != to.len() || from.is_empty() {
        return 0;
    }
    let center =
        |points: &[glam::Vec2]| points.iter().copied().sum::<glam::Vec2>() / points.len() as f32;
    let from_center = center(from);
    let to_center = center(to);
    (0..to.len())
        .min_by(|left, right| {
            let error = |shift: usize| {
                from.iter()
                    .enumerate()
                    .map(|(index, from)| {
                        let delta =
                            (*from - from_center) - (to[(index + shift) % to.len()] - to_center);
                        delta.length_squared()
                    })
                    .sum::<f32>()
            };
            error(*left).total_cmp(&error(*right))
        })
        .unwrap_or(0)
}

pub fn shaky_path_seed(seed: u32, evolution: f32) -> u32 {
    let mut value = u64::from(seed) ^ u64::from(evolution.round().to_bits()).rotate_left(32);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (value ^ (value >> 31)) as u32
}

pub fn facet_transform(
    progress: f32,
    index: usize,
    count: usize,
    extent: f32,
) -> (glam::Vec2, f32, f32, f32) {
    let progress = progress.clamp(0.0, 1.0);
    let away = (1.0 - progress).powi(2);
    let direction = 218.0 + index as f32 * 137.5;
    let distance = extent * (0.24 + 0.035 * (index % 3) as f32) * away;
    let offset = polar_degrees(distance, direction);
    let middle = count.saturating_sub(1) as f32 * 0.5;
    let rotation =
        ((index as f32 - middle) * 2.8 + if index.is_multiple_of(2) { -7.0 } else { 7.0 }) * away;
    let scale = 0.78 + 0.22 * progress;
    let fade = (progress / 0.3).clamp(0.0, 1.0);
    let opacity = fade * fade * (3.0 - 2.0 * fade);
    (offset, rotation, scale, opacity)
}

pub fn coalesce_pool(progress: f32, index: usize) -> (glam::Vec2, f32) {
    let seeds = [
        glam::Vec2::new(0.27, 0.38),
        glam::Vec2::new(0.7, 0.32),
        glam::Vec2::new(0.52, 0.72),
        glam::Vec2::new(0.34, 0.68),
        glam::Vec2::new(0.76, 0.63),
    ];
    let delay = index.min(seeds.len() - 1) as f32 * 0.07;
    let local = ((progress.clamp(0.0, 1.0) - delay) / (1.0 - delay)).clamp(0.0, 1.0);
    let growth = shrimply_interpolation::Interpolation::SineInOut.value(f64::from(local)) as f32;
    let center = seeds[index.min(seeds.len() - 1)].lerp(glam::Vec2::splat(0.5), growth * 0.12);
    let wobble = 1.0
        + 0.07 * (progress * std::f32::consts::TAU + index as f32 * 2.1).sin() * (1.0 - progress);
    (center, growth * 1.05 * wobble)
}

pub fn vector_reveal_opacity(progress: f32) -> f32 {
    let progress = (progress / 0.38).clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

pub fn transition_accent(progress: f32) -> f32 {
    (std::f32::consts::PI * progress.clamp(0.0, 1.0))
        .sin()
        .max(0.0)
}

pub fn seed_at_frequency(elapsed: Time, frequency: u32) -> u32 {
    ((u128::from(elapsed.as_nonnegative_nanos()) * u128::from(frequency)) / 1_000_000_000)
        .min(u128::from(u32::MAX)) as u32
}

pub fn origami_mesh_vertices(
    width: u32,
    height: u32,
    grid: u32,
    visibility: f32,
    depth: f32,
    direction_degrees: f32,
) -> Vec<f32> {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let grid = grid.clamp(2, 6);
    let fold = 1.0 - visibility.clamp(0.0, 1.0);
    let angle = fold * std::f32::consts::FRAC_PI_2 * 0.9;
    let compress_x = angle.cos().max(0.12);
    let compress_y = (angle * 0.52).cos().max(0.62);
    let cell = (width / grid as f32).min(height / grid as f32);
    let height_scale = angle.sin() * cell * depth.clamp(0.0, 1.0) * 0.9;
    let direction = polar_degrees(1.0, direction_degrees);
    let canvas_center = glam::Vec2::new(width, height) * 0.5;
    let center = canvas_center + direction * fold * width.min(height) * 0.055;
    let projection = direction * 0.42 + glam::Vec2::new(-direction.y, direction.x) * 0.14;
    let mut vertices = Vec::with_capacity(((grid + 1) * (grid + 1) * 3) as usize);
    for row in 0..=grid {
        for column in 0..=grid {
            let source = glam::Vec2::new(
                column as f32 / grid as f32 * width,
                row as f32 / grid as f32 * height,
            );
            let local = source - canvas_center;
            let ridge = if (column + row) & 1 == 0 { 1.0 } else { -1.0 };
            let z = ridge * height_scale;
            let projected = center
                + glam::Vec2::new(local.x * compress_x, local.y * compress_y)
                + projection * z;
            vertices.extend([projected.x, projected.y, z]);
        }
    }
    vertices
}

pub fn audio_transition_gain(
    item_start: Time,
    item_end: Time,
    transitions: impl IntoIterator<Item = (TransitionSide, Time, shrimply_interpolation::Interpolation)>,
    position: Time,
) -> f32 {
    for (side, duration, interpolation) in transitions {
        let Some(progress) = transition_progress(item_start, item_end, duration, side, position)
        else {
            continue;
        };
        let eased = interpolation.value(f64::from(progress)) as f32;
        return match side {
            TransitionSide::Intro => eased,
            TransitionSide::Outro => 1.0 - eased,
        }
        .clamp(0.0, 1.0);
    }
    1.0
}

pub fn clip_transition_half_duration(duration: Time) -> Time {
    Time {
        seconds: duration.seconds / Fraction::from(2_u8),
    }
}

pub fn default_clip_transition_duration(left_duration: Time, right_duration: Time) -> Time {
    Time::from_seconds(1).min(clip_transition_half_duration(
        left_duration.min(right_duration),
    ))
}

pub fn maximum_clip_transition_duration(
    left_duration: Time,
    right_duration: Time,
    left_intro: Option<Time>,
    right_outro: Option<Time>,
) -> Time {
    let double = Fraction::from(2_u8);
    clip_transition_half_duration(left_duration.min(right_duration))
        .min(Time {
            seconds: left_duration
                .saturating_sub(left_intro.unwrap_or(Time::ZERO))
                .seconds
                * double,
        })
        .min(Time {
            seconds: right_duration
                .saturating_sub(right_outro.unwrap_or(Time::ZERO))
                .seconds
                * double,
        })
}

pub fn minimum_clip_transition_item_duration(
    duration: Time,
    opposite_transition: Option<Time>,
) -> Time {
    Time {
        seconds: duration.seconds * Fraction::from(2_u8),
    }
    .max(
        opposite_transition
            .unwrap_or(Time::ZERO)
            .saturating_add(clip_transition_half_duration(duration)),
    )
}

pub fn clip_transition_bounds(cut: Time, duration: Time) -> (Time, Time) {
    let half = clip_transition_half_duration(duration);
    (cut.saturating_sub(half), cut.saturating_add(half))
}

pub fn clip_transition_progress(cut: Time, duration: Time, position: Time) -> Option<f32> {
    if duration <= Time::ZERO {
        return None;
    }
    let (start, end) = clip_transition_bounds(cut, duration);
    if position < start || position > end {
        return None;
    }
    Some((position.signed_sub(start).as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0) as f32)
}

pub fn clip_transition_travel_distance(canvas_size: CanvasSize, direction_degrees: f32) -> f32 {
    let direction = polar_degrees(1.0, direction_degrees).abs();
    let horizontal = if direction.x > f32::EPSILON {
        canvas_size.width.max(1) as f32 / direction.x
    } else {
        f32::INFINITY
    };
    let vertical = if direction.y > f32::EPSILON {
        canvas_size.height.max(1) as f32 / direction.y
    } else {
        f32::INFINITY
    };
    horizontal.min(vertical)
}

pub fn audio_clip_transition_gains(curve: AudioClipTransitionCurve, progress: f32) -> (f32, f32) {
    let progress = progress.clamp(0.0, 1.0);
    match curve {
        AudioClipTransitionCurve::EqualPower => {
            let angle = progress * std::f32::consts::FRAC_PI_2;
            (angle.cos(), angle.sin())
        }
        AudioClipTransitionCurve::Linear => (1.0 - progress, progress),
    }
}

pub fn audio_decay_feedback(delay_seconds: f32, decay_seconds: f32) -> f32 {
    10.0_f32.powf(-3.0 * delay_seconds / decay_seconds.max(f32::EPSILON))
}

pub fn audio_smoothing_coefficient(seconds: f32, sample_rate: u32) -> f32 {
    1.0 - (-1.0 / (seconds.max(f32::EPSILON) * sample_rate.max(1) as f32)).exp()
}

pub fn audio_lowpass_coefficient(cutoff_hz: f32, sample_rate: u32) -> f32 {
    1.0 - (-std::f32::consts::TAU * cutoff_hz / sample_rate.max(1) as f32).exp()
}

pub fn audio_geometric_lerp(start: f32, end: f32, amount: f32) -> f32 {
    start * (end / start).powf(amount.clamp(0.0, 1.0))
}

pub fn audio_stereo_width(left: f32, right: f32, width: f32) -> (f32, f32) {
    let middle = (left + right) * 0.5;
    let side = (left - right) * 0.5 * width;
    (middle + side, middle - side)
}

pub fn audio_tremolo_gain(time: Time, rate_hz: f32, depth: f32) -> f32 {
    let phase = std::f64::consts::TAU * f64::from(rate_hz) * time.as_secs_f64();
    let oscillator = (phase.sin() as f32 + 1.0) * 0.5;
    1.0 - depth + oscillator * depth
}

pub fn audio_quantize_sample(sample: f32, resolution_bits: f32) -> f32 {
    let levels = 2.0_f32.powf(resolution_bits.round()) - 1.0;
    ((sample.clamp(-1.0, 1.0) + 1.0) * 0.5 * levels).round() / levels * 2.0 - 1.0
}

pub fn audio_chorus_delay_ms(delay_ms: f32, depth_ms: f32, phase: f32) -> f32 {
    delay_ms + depth_ms * 0.5 * phase.sin()
}

pub const AUDIO_ROOM_REFLECTIONS: usize = 6;
const AUDIO_ROOM_LENGTH_MIN_M: f32 = 6.0;
const AUDIO_ROOM_LENGTH_MAX_M: f32 = 12.0;
const AUDIO_ROOM_WIDTH_MIN_M: f32 = 3.0;
const AUDIO_ROOM_WIDTH_MAX_M: f32 = 8.0;
const AUDIO_ROOM_HEIGHT_MIN_M: f32 = 2.4;
const AUDIO_ROOM_HEIGHT_MAX_M: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioRoomReflection {
    pub delay_seconds: f32,
    pub relative_gain: f32,
}

pub fn audio_room_reflections(
    distance_m: f32,
    room_size: f32,
) -> [AudioRoomReflection; AUDIO_ROOM_REFLECTIONS] {
    const SPEED_OF_SOUND_MPS: f32 = 343.0;
    const MIN_DISTANCE_M: f32 = 0.2;
    const MAX_DISTANCE_M: f32 = 5.0;
    const WALL_MARGIN_M: f32 = 0.5;
    const SOURCE_HEIGHT_M: f32 = 1.4;

    let distance = distance_m.clamp(MIN_DISTANCE_M, MAX_DISTANCE_M);
    let [length, width, height] = audio_room_dimensions(room_size);
    let microphone = [
        WALL_MARGIN_M,
        width * 0.5,
        SOURCE_HEIGHT_M.min(height * 0.5),
    ];
    let source = [microphone[0] + distance, microphone[1], microphone[2]];
    let images = [
        [-source[0], source[1], source[2]],
        [2.0 * length - source[0], source[1], source[2]],
        [source[0], -source[1], source[2]],
        [source[0], 2.0 * width - source[1], source[2]],
        [source[0], source[1], -source[2]],
        [source[0], source[1], 2.0 * height - source[2]],
    ];
    images.map(|image| {
        let path = ((image[0] - microphone[0]).powi(2)
            + (image[1] - microphone[1]).powi(2)
            + (image[2] - microphone[2]).powi(2))
        .sqrt()
        .max(distance);
        AudioRoomReflection {
            delay_seconds: (path - distance) / SPEED_OF_SOUND_MPS,
            relative_gain: distance / path,
        }
    })
}

pub fn audio_room_decay_seconds(room_size: f32, absorption: f32) -> f32 {
    const SABINE_METRIC: f32 = 0.161;
    const MIN_ABSORPTION: f32 = 0.15;
    const MAX_ABSORPTION: f32 = 0.85;
    const MIN_DECAY_SECONDS: f32 = 0.15;
    const MAX_DECAY_SECONDS: f32 = 2.5;

    let [length, width, height] = audio_room_dimensions(room_size);
    let volume = length * width * height;
    let surface = 2.0 * (length * width + length * height + width * height);
    let absorption =
        MIN_ABSORPTION + absorption.clamp(0.0, 1.0) * (MAX_ABSORPTION - MIN_ABSORPTION);
    (SABINE_METRIC * volume / (surface * absorption)).clamp(MIN_DECAY_SECONDS, MAX_DECAY_SECONDS)
}

fn audio_room_dimensions(room_size: f32) -> [f32; 3] {
    let size = room_size.clamp(0.0, 1.0);
    [
        AUDIO_ROOM_LENGTH_MIN_M + (AUDIO_ROOM_LENGTH_MAX_M - AUDIO_ROOM_LENGTH_MIN_M) * size,
        AUDIO_ROOM_WIDTH_MIN_M + (AUDIO_ROOM_WIDTH_MAX_M - AUDIO_ROOM_WIDTH_MIN_M) * size,
        AUDIO_ROOM_HEIGHT_MIN_M + (AUDIO_ROOM_HEIGHT_MAX_M - AUDIO_ROOM_HEIGHT_MIN_M) * size,
    ]
}

pub fn audio_proximity_amount(distance_cm: f32) -> f32 {
    const MIN_DISTANCE_CM: f32 = 3.0;
    const MAX_DISTANCE_CM: f32 = 100.0;
    let distance = distance_cm.clamp(MIN_DISTANCE_CM, MAX_DISTANCE_CM);
    (MAX_DISTANCE_CM / distance).ln() / (MAX_DISTANCE_CM / MIN_DISTANCE_CM).ln()
}

pub fn audio_soft_clip(sample: f32, amount: f32) -> f32 {
    let drive = amount.clamp(0.0, 1.0);
    if drive <= f32::EPSILON {
        sample
    } else {
        (sample * drive).tanh() / drive
    }
}

pub fn audio_soft_ceiling(sample: f32, ceiling: f32) -> f32 {
    const KNEE_RATIO: f32 = 0.8;
    let ceiling = ceiling.max(f32::EPSILON);
    let knee = ceiling * KNEE_RATIO;
    let magnitude = sample.abs();
    if magnitude <= knee {
        sample
    } else {
        sample.signum() * (knee + (ceiling - knee) * ((magnitude - knee) / (ceiling - knee)).tanh())
    }
}

const PINK_NOISE_OCTAVES: u32 = 16;
const BROWN_NOISE_OCTAVES: u32 = 12;

pub fn audio_sine_sample(phase: f32) -> f32 {
    (std::f32::consts::TAU * phase).sin()
}

pub fn audio_triangle_sample(phase: f32) -> f32 {
    1.0 - 4.0 * (phase.fract() - 0.5).abs()
}

pub fn audio_sawtooth_sample(phase: f32, phase_step: f32) -> f32 {
    let phase = phase.fract();
    2.0 * phase - 1.0 - audio_poly_blep(phase, phase_step)
}

pub fn audio_square_pulse_sample(phase: f32, phase_step: f32, pulse_width: f32) -> f32 {
    let phase = phase.fract();
    let pulse_width = pulse_width.clamp(0.01, 0.99);
    let mut sample = if phase < pulse_width { 1.0 } else { -1.0 };
    sample += audio_poly_blep(phase, phase_step);
    sample -= audio_poly_blep((phase - pulse_width).rem_euclid(1.0), phase_step);
    sample
}

pub fn audio_white_noise_sample(seed: u32, sample_index: i64) -> f32 {
    hash_noise(seed, sample_index, 0)
}

pub fn audio_pink_noise_sample(seed: u32, sample_index: i64) -> f32 {
    let sum = (0..PINK_NOISE_OCTAVES)
        .map(|octave| hash_noise(seed, sample_index.div_euclid(1_i64 << octave), octave + 1))
        .sum::<f32>();
    (sum / (PINK_NOISE_OCTAVES as f32).sqrt()).clamp(-1.0, 1.0)
}

pub fn audio_brown_noise_sample(seed: u32, sample_index: i64) -> f32 {
    let mut sum = 0.0;
    let mut weight_sum = 0.0;
    for octave in 0..BROWN_NOISE_OCTAVES {
        let period = 1_i64 << octave;
        let left = sample_index.div_euclid(period);
        let progress = sample_index.rem_euclid(period) as f32 / period as f32;
        let progress = progress * progress * (3.0 - 2.0 * progress);
        let weight = period as f32;
        let left_value = hash_noise(seed, left, octave + 1);
        let right_value = hash_noise(seed, left + 1, octave + 1);
        sum += (left_value + (right_value - left_value) * progress) * weight;
        weight_sum += weight;
    }
    (sum / weight_sum).clamp(-1.0, 1.0)
}

fn audio_poly_blep(phase: f32, phase_step: f32) -> f32 {
    let phase_step = phase_step.clamp(f32::EPSILON, 1.0);
    if phase < phase_step {
        let value = phase / phase_step;
        value + value - value * value - 1.0
    } else if phase > 1.0 - phase_step {
        let value = (phase - 1.0) / phase_step;
        value * value + value + value + 1.0
    } else {
        0.0
    }
}

fn hash_noise(seed: u32, sample_index: i64, stream: u32) -> f32 {
    let mut value = u64::from_ne_bytes(sample_index.to_ne_bytes())
        ^ u64::from(seed)
        ^ u64::from(stream).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    let unit = ((value ^ (value >> 31)) >> 40) as f32 / ((1_u32 << 24) - 1) as f32;
    unit * 2.0 - 1.0
}

pub fn transition_progress(
    item_start: Time,
    item_end: Time,
    duration: Time,
    side: TransitionSide,
    position: Time,
) -> Option<f32> {
    let duration = duration.as_nanos_i128();
    if duration <= 0 {
        return None;
    }
    let elapsed = match side {
        TransitionSide::Intro => position.as_nanos_i128() - item_start.as_nanos_i128(),
        TransitionSide::Outro => position.as_nanos_i128() - (item_end.as_nanos_i128() - duration),
    };
    (elapsed >= 0 && elapsed <= duration)
        .then_some((elapsed as f64 / duration as f64).clamp(0.0, 1.0) as f32)
}
