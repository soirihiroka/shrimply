use super::*;
use shrimply_math_core::{fraction_floor_i64, fraction_rem_euclid};

pub fn default_playback_speed() -> Fraction {
    fraction_from_integer(1)
}

pub fn native_playback_fps() -> Fraction {
    FRACTION_ZERO
}

pub fn playback_speed_or_default(value: Fraction) -> Fraction {
    match value {
        GenericFraction::Rational(_, _) => value,
        _ => default_playback_speed(),
    }
}

pub fn playback_speed_is_zero(value: Fraction) -> bool {
    matches!(playback_speed_or_default(value), GenericFraction::Rational(_, ratio) if *ratio.numer() == 0)
}

pub fn playback_speed_is_negative(value: Fraction) -> bool {
    fraction_numerator(playback_speed_or_default(value)) < 0
}

pub fn scaled_time_delta(delta: Time, playback_speed: Fraction) -> Time {
    Time {
        seconds: delta.seconds * playback_speed_or_default(playback_speed),
    }
}

pub fn unscaled_time_delta(delta: Time, playback_speed: Fraction) -> Time {
    if playback_speed_is_zero(playback_speed) {
        return Time::ZERO;
    }
    Time {
        seconds: delta.seconds / playback_speed_or_default(playback_speed),
    }
}

pub fn video_source_time(item: &VideoItem, position: Time) -> Time {
    video_source_time_at(item, position).unwrap_or(item.time_offset)
}

pub fn video_source_time_at(item: &VideoItem, position: Time) -> Option<Time> {
    if playback_speed_is_zero(item.playback_speed) {
        return None;
    }
    repeat_source_time(
        Time::ZERO,
        item.source_duration,
        item.time_offset.saturating_add(scaled_time_delta(
            position.signed_sub(item.start),
            item.playback_speed,
        )),
        item.repeat_strategy,
    )
}

pub fn generated_source_time_at(item: &VideoItem, position: Time) -> Option<Time> {
    generated_source_time_with_duration(item, position, item.source_duration)
}

pub fn generated_source_time_with_duration(
    item: &VideoItem,
    position: Time,
    duration: Time,
) -> Option<Time> {
    if playback_speed_is_zero(item.playback_speed) {
        return None;
    }
    repeat_source_time(
        Time::ZERO,
        duration,
        item.animation_time_offset.saturating_add(scaled_time_delta(
            position.signed_sub(item.start),
            item.playback_speed,
        )),
        item.repeat_strategy,
    )
}

pub fn audio_source_time(item: &AudioItem, position: Time) -> Time {
    audio_source_time_at(item, position).unwrap_or(item.time_offset)
}

pub fn audio_source_time_at(item: &AudioItem, position: Time) -> Option<Time> {
    if playback_speed_is_zero(item.playback_speed) {
        return None;
    }
    repeat_source_time(
        Time::ZERO,
        item.source_duration,
        item.time_offset.saturating_add(scaled_time_delta(
            position.signed_sub(item.start),
            item.playback_speed,
        )),
        item.repeat_strategy,
    )
}

pub fn repeat_source_time(
    start: Time,
    end: Time,
    elapsed: Time,
    strategy: RepeatStrategy,
) -> Option<Time> {
    let span = end.seconds - start.seconds;
    if span == FRACTION_ZERO {
        return (strategy != RepeatStrategy::Empty).then_some(start);
    }
    repeat_span_time(
        start.seconds,
        end.seconds,
        start.seconds + elapsed.seconds,
        strategy,
    )
    .map(|seconds| Time { seconds })
}

pub fn generated_item_time(item: &VideoItem, position: Time) -> Option<Time> {
    let local = generated_item_animation_time(item, position);
    let Some((start, end)) = generated_item_keyframe_span(item) else {
        return Some(local);
    };
    repeat_local_time(start, end, local, item.repeat_strategy)
}

/// Unclamped animation time for authoring generated content at the playhead.
pub fn generated_item_animation_time(item: &VideoItem, position: Time) -> Time {
    position
        .signed_sub(item.start)
        .saturating_add(item.animation_time_offset)
}

pub fn generated_item_keyframe_span(item: &VideoItem) -> Option<(Time, Time)> {
    let content_span = match &item.content {
        VideoItemContent::Text(text) => keyframe_span_from_iter(
            [
                timeline_value_keyframe_span(&text.text),
                timeline_value_keyframe_span(&text.font_style),
                timeline_value_keyframe_span(&text.h_align),
                timeline_value_keyframe_span(&text.v_align),
                timeline_value_keyframe_span(&text.direction),
                timeline_value_keyframe_span(&text.font_size),
                timeline_value_keyframe_span(&text.font_weight),
                timeline_value_keyframe_span(&text.tracking),
                timeline_value_keyframe_span(&text.line_height),
                timeline_value_keyframe_span(&text.color),
                timeline_value_keyframe_span(&text.background_color),
                timeline_value_keyframe_span(&text.background_roundness),
                timeline_value_keyframe_span(&text.background_padding),
                timeline_value_keyframe_span(&text.outline_width),
                timeline_value_keyframe_span(&text.outline_color),
                timeline_value_keyframe_span(&text.shadow_color),
                timeline_value_keyframe_span(&text.shadow_distance),
                timeline_value_keyframe_span(&text.shadow_direction_degrees),
                timeline_value_keyframe_span(&text.shadow_width),
                timeline_value_keyframe_span(&text.shadow_blur),
            ]
            .into_iter(),
        ),
        VideoItemContent::Shape(shape) => keyframe_span_from_iter(
            [
                timeline_value_keyframe_span(&shape.shape),
                timeline_value_keyframe_span(&shape.rounding_strategy),
                timeline_value_keyframe_span(&shape.size),
                timeline_value_keyframe_span(&shape.star_points),
                timeline_value_keyframe_span(&shape.star_inner_radius_percent),
                timeline_value_keyframe_span(&shape.arrow_shaft_width_percent),
                timeline_value_keyframe_span(&shape.arrow_head_length_percent),
                timeline_value_keyframe_span(&shape.cross_arm_thickness_percent),
                timeline_value_keyframe_span(&shape.fill),
                timeline_value_keyframe_span(&shape.outline_color),
                timeline_value_keyframe_span(&shape.outline_width),
                timeline_value_keyframe_span(&shape.corner_radius),
                timeline_value_keyframe_span(&shape.shadow_color),
                timeline_value_keyframe_span(&shape.shadow_distance),
                timeline_value_keyframe_span(&shape.shadow_direction_degrees),
                timeline_value_keyframe_span(&shape.shadow_width),
                timeline_value_keyframe_span(&shape.shadow_blur),
            ]
            .into_iter(),
        ),
        VideoItemContent::Paint(paint) => paint::keyframe_span(paint),
        VideoItemContent::Obj(scene) => obj_scene_keyframe_span(scene),
        VideoItemContent::Gaussian(scene) => gaussian_scene_keyframe_span(scene),
        VideoItemContent::Media
        | VideoItemContent::Image
        | VideoItemContent::Gif
        | VideoItemContent::Svg
        | VideoItemContent::Pdf(_)
        | VideoItemContent::Manim(_)
        | VideoItemContent::Sanim(_)
        | VideoItemContent::Blender(_)
        | VideoItemContent::FoldedSequence(_) => None,
        VideoItemContent::Background(background) => background.generator.keyframe_span(),
        VideoItemContent::LayeredImage(image) => {
            keyframe_span_from_iter(image.layers.iter().map(|layer| {
                layer
                    .visibility
                    .as_ref()
                    .and_then(timeline_value_keyframe_span)
            }))
        }
    };
    let transform_span = if matches!(
        item.content,
        VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
    ) {
        None
    } else {
        keyframe_span_from_iter(
            [
                timeline_value_keyframe_span(&item.transform.position),
                timeline_value_keyframe_span(&item.transform.anchor),
                timeline_value_keyframe_span(&item.transform.scale),
                timeline_value_keyframe_span(&item.transform.shear),
                timeline_value_keyframe_span(&item.transform.rotation_degrees),
            ]
            .into_iter(),
        )
    };
    keyframe_span_from_iter(
        [
            transform_span,
            timeline_value_keyframe_span(&item.compositing.opacity),
            timeline_value_keyframe_span(&item.compositing.blend_mode),
            item.compositing
                .alpha_mask
                .as_ref()
                .filter(|mask| mask.enabled)
                .and_then(alpha_mask_keyframe_span),
            timeline_value_keyframe_span(&item.visibility),
            content_span,
            keyframe_span_from_iter(
                item.modifiers
                    .iter()
                    .filter(|modifier| modifier.enabled)
                    .map(|modifier| {
                        keyframe_span_from_iter(
                            [
                                modifier.effect.keyframe_span(),
                                modifier
                                    .alpha_mask
                                    .as_ref()
                                    .filter(|mask| mask.enabled)
                                    .and_then(alpha_mask_keyframe_span),
                            ]
                            .into_iter(),
                        )
                    }),
            ),
        ]
        .into_iter(),
    )
}

fn alpha_mask_keyframe_span(mask: &VisualAlphaMask) -> Option<(Time, Time)> {
    keyframe_span_from_iter(
        [
            timeline_value_keyframe_span(&mask.center),
            timeline_value_keyframe_span(&mask.size),
            timeline_value_keyframe_span(&mask.rotation_degrees),
            timeline_value_keyframe_span(&mask.feather),
            timeline_value_keyframe_span(&mask.rounding),
        ]
        .into_iter(),
    )
}

fn animated_vec3_keyframe_span(value: &AnimatedVec3) -> Option<(Time, Time)> {
    timeline_value_keyframe_span(value)
}

fn obj_scene_keyframe_span(scene: &ObjScene) -> Option<(Time, Time)> {
    keyframe_span_from_iter(
        [
            animated_vec3_keyframe_span(&scene.model.position),
            animated_vec3_keyframe_span(&scene.model.anchor),
            animated_vec3_keyframe_span(&scene.model.rotation_degrees),
            timeline_value_keyframe_span(&scene.model.rotation_order),
            animated_vec3_keyframe_span(&scene.model.scale),
            animated_vec3_keyframe_span(&scene.camera.position),
            animated_vec3_keyframe_span(&scene.camera.rotation_degrees),
            timeline_value_keyframe_span(&scene.camera.vertical_fov_degrees),
            timeline_value_keyframe_span(&scene.camera.orthographic_height),
            timeline_value_keyframe_span(&scene.camera.focus_distance),
            timeline_value_keyframe_span(&scene.camera.background_distance),
            timeline_value_keyframe_span(&scene.camera.f_stop),
            timeline_value_keyframe_span(&scene.camera.exposure_ev),
            timeline_value_keyframe_span(&scene.material.base_color),
            timeline_value_keyframe_span(&scene.material.metallic),
            timeline_value_keyframe_span(&scene.material.roughness),
            timeline_value_keyframe_span(&scene.material.subsurface),
            timeline_value_keyframe_span(&scene.material.clearcoat),
            timeline_value_keyframe_span(&scene.material.sheen),
            timeline_value_keyframe_span(&scene.material.transmission),
            timeline_value_keyframe_span(&scene.material.ior),
            timeline_value_keyframe_span(&scene.material.toon.bands),
            timeline_value_keyframe_span(&scene.material.toon.color_levels),
            timeline_value_keyframe_span(&scene.material.toon.shadow_color),
            timeline_value_keyframe_span(&scene.material.toon.shadow_strength),
            timeline_value_keyframe_span(&scene.material.toon.rim_color),
            timeline_value_keyframe_span(&scene.material.toon.rim_strength),
            timeline_value_keyframe_span(&scene.material.toon.rim_power),
            timeline_value_keyframe_span(&scene.material.toon.specular_size),
            timeline_value_keyframe_span(&scene.material.toon.specular_strength),
            timeline_value_keyframe_span(&scene.material.toon.outline.color),
            timeline_value_keyframe_span(&scene.material.toon.outline.width),
            timeline_value_keyframe_span(&scene.material.toon.outline.opacity),
            timeline_value_keyframe_span(&scene.material.toon.outline.depth_threshold),
            timeline_value_keyframe_span(&scene.material.toon.outline.normal_angle_degrees),
            timeline_value_keyframe_span(&scene.material.toon.outline.dog_inner_radius),
            timeline_value_keyframe_span(&scene.material.toon.outline.dog_radius_ratio),
            timeline_value_keyframe_span(&scene.material.toon.outline.dog_threshold),
            timeline_value_keyframe_span(&scene.material.toon.outline.dog_sharpness),
            timeline_value_keyframe_span(&scene.material.toon.outline.offset_variation),
            timeline_value_keyframe_span(&scene.material.toon.outline.width_variation),
            timeline_value_keyframe_span(&scene.material.toon.outline.offset_frequency),
            timeline_value_keyframe_span(&scene.material.toon.outline.width_frequency),
            timeline_value_keyframe_span(&scene.material.toon.outline.aggressiveness),
            timeline_value_keyframe_span(&scene.material.toon.outline.noise_seed),
            timeline_value_keyframe_span(&scene.material.toon.outline.noise_evolution),
            timeline_value_keyframe_span(&scene.shadow_receiver.enabled),
            animated_vec3_keyframe_span(&scene.shadow_receiver.position),
            animated_vec3_keyframe_span(&scene.shadow_receiver.rotation_degrees),
            timeline_value_keyframe_span(&scene.shadow_receiver.opacity),
            animated_vec3_keyframe_span(&scene.environment.rotation_degrees),
            timeline_value_keyframe_span(&scene.environment.intensity),
        ]
        .into_iter(),
    )
}

fn gaussian_scene_keyframe_span(scene: &shrimply_3dgs_core::GaussianScene) -> Option<(Time, Time)> {
    keyframe_span_from_iter(
        [
            animated_vec3_keyframe_span(&scene.model.position),
            animated_vec3_keyframe_span(&scene.model.anchor),
            animated_vec3_keyframe_span(&scene.model.rotation_degrees),
            timeline_value_keyframe_span(&scene.model.rotation_order),
            animated_vec3_keyframe_span(&scene.model.scale),
            animated_vec3_keyframe_span(&scene.camera.position),
            animated_vec3_keyframe_span(&scene.camera.rotation_degrees),
            timeline_value_keyframe_span(&scene.camera.vertical_fov_degrees),
            timeline_value_keyframe_span(&scene.camera.orthographic_height),
            timeline_value_keyframe_span(&scene.camera.focus_distance),
            timeline_value_keyframe_span(&scene.camera.f_stop),
            timeline_value_keyframe_span(&scene.camera.exposure_ev),
        ]
        .into_iter(),
    )
}

pub fn generated_item_natural_end_position(item: &VideoItem) -> Option<Time> {
    let (_, end) = generated_item_keyframe_span(item)?;
    let delta = next_natural_end_delta(end, item.animation_time_offset, item.repeat_strategy)?;
    if delta <= Time::ZERO {
        return None;
    }
    Some(item.start.saturating_add(delta))
}

pub fn generated_item_natural_span(item: &VideoItem) -> Option<(Time, Time)> {
    let end = generated_item_natural_end_position(item)?;
    Some((
        end.saturating_sub(generated_item_keyframe_span(item)?.1),
        end,
    ))
}

pub fn media_real_span(
    start: Time,
    time_offset: Time,
    source_duration: Time,
    playback_speed: Fraction,
    repeat_strategy: RepeatStrategy,
) -> Option<(Time, Time)> {
    let end = media_item_natural_end_position(
        start,
        time_offset,
        source_duration,
        playback_speed,
        repeat_strategy,
    )?;
    Some((
        end.saturating_sub(
            unscaled_time_delta(source_duration, playback_speed).abs_diff(Time::ZERO),
        ),
        end,
    ))
}

pub fn media_natural_end_interval(
    source_duration: Time,
    playback_speed: Fraction,
    repeat_strategy: RepeatStrategy,
) -> Option<Time> {
    matches!(
        repeat_strategy,
        RepeatStrategy::Repeat | RepeatStrategy::PingPong
    )
    .then(|| unscaled_time_delta(source_duration, playback_speed).abs_diff(Time::ZERO))
    .filter(|interval| *interval > Time::ZERO)
}

pub fn video_natural_end_interval(item: &VideoItem) -> Option<Time> {
    if item.repeats_keyframes()
        && matches!(
            item.repeat_strategy,
            RepeatStrategy::Repeat | RepeatStrategy::PingPong
        )
    {
        generated_item_keyframe_span(item).map(|(_, end)| end)
    } else {
        media_natural_end_interval(
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )
    }
}

pub fn media_item_natural_end_position(
    start: Time,
    time_offset: Time,
    source_duration: Time,
    playback_speed: Fraction,
    repeat_strategy: RepeatStrategy,
) -> Option<Time> {
    let speed = playback_speed_or_default(playback_speed);
    if playback_speed_is_zero(speed) {
        return None;
    }
    let delta = if playback_speed_is_negative(speed) {
        previous_natural_start_delta(source_duration, time_offset, repeat_strategy)?
    } else {
        next_natural_end_delta(source_duration, time_offset, repeat_strategy)?
    };
    let timeline_delta = unscaled_time_delta(delta, speed);
    (timeline_delta > Time::ZERO).then(|| start.saturating_add(timeline_delta))
}

fn next_natural_end_delta(
    natural_duration: Time,
    offset: Time,
    repeat_strategy: RepeatStrategy,
) -> Option<Time> {
    let duration = natural_duration.seconds;
    if duration <= FRACTION_ZERO {
        return None;
    }
    let offset = offset.seconds;
    let target = if offset < duration {
        duration
    } else {
        match repeat_strategy {
            RepeatStrategy::Repeat | RepeatStrategy::PingPong => {
                duration
                    * fraction_from_integer(fraction_floor_i64(offset / duration)?.checked_add(1)?)
            }
            RepeatStrategy::Hold | RepeatStrategy::Empty => return None,
        }
    };
    let delta = target - offset;
    (delta > FRACTION_ZERO).then_some(Time { seconds: delta })
}

fn previous_natural_start_delta(
    natural_duration: Time,
    offset: Time,
    repeat_strategy: RepeatStrategy,
) -> Option<Time> {
    let duration = natural_duration.seconds;
    if duration <= FRACTION_ZERO {
        return None;
    }
    let offset = offset.seconds;
    let target = match repeat_strategy {
        RepeatStrategy::Repeat | RepeatStrategy::PingPong => {
            let boundary = duration * fraction_from_integer(fraction_floor_i64(offset / duration)?);
            if boundary == offset {
                boundary - duration
            } else {
                boundary
            }
        }
        RepeatStrategy::Hold | RepeatStrategy::Empty if offset > FRACTION_ZERO => FRACTION_ZERO,
        RepeatStrategy::Hold | RepeatStrategy::Empty => return None,
    };
    let delta = target - offset;
    (delta < FRACTION_ZERO).then_some(Time { seconds: delta })
}

fn repeat_local_time(start: Time, end: Time, time: Time, strategy: RepeatStrategy) -> Option<Time> {
    repeat_span_time(start.seconds, end.seconds, time.seconds, strategy)
        .map(|seconds| Time { seconds })
}

fn repeat_span_time(
    start: Fraction,
    end: Fraction,
    time: Fraction,
    strategy: RepeatStrategy,
) -> Option<Fraction> {
    if end <= start {
        return (strategy != RepeatStrategy::Empty).then_some(start);
    }
    if time >= start && time < end {
        return Some(time);
    }
    let span = end - start;
    match strategy {
        RepeatStrategy::Repeat => Some(start + fraction_rem_euclid(time - start, span)?),
        RepeatStrategy::PingPong => {
            let cycle = span * fraction_from_integer(2);
            let offset = fraction_rem_euclid(time - start, cycle)?;
            if offset <= span {
                Some(start + offset)
            } else {
                Some(end - (offset - span))
            }
        }
        RepeatStrategy::Hold => Some(if time < start { start } else { end }),
        RepeatStrategy::Empty => None,
    }
}

pub(super) fn keyframe_span_from_iter(
    spans: impl Iterator<Item = Option<(Time, Time)>>,
) -> Option<(Time, Time)> {
    let mut end = None::<Time>;
    for (_, span_end) in spans.flatten() {
        end = Some(end.map_or(span_end, |current| current.max(span_end)));
    }
    let end = end?;
    (end > Time::ZERO).then_some((Time::ZERO, end))
}

pub(super) fn timeline_value_keyframe_span<T: TimelineValueType>(
    value: &TimelineValue<T>,
) -> Option<(Time, Time)> {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return None;
    };
    keyframe_times(keyframes.iter().map(TimelineKeyframe::time))
}

fn keyframe_times(times: impl Iterator<Item = Time>) -> Option<(Time, Time)> {
    let mut start = None::<Time>;
    let mut end = None::<Time>;
    for time in times {
        start = Some(start.map_or(time, |current| current.min(time)));
        end = Some(end.map_or(time, |current| current.max(time)));
    }
    let start = start?;
    let end = end?;
    (end > start).then_some((start, end))
}

pub fn retimed_end_preserving_source_out(
    start: Time,
    end: Time,
    old_speed: Fraction,
    new_speed: Fraction,
) -> Time {
    start.saturating_add(unscaled_time_delta(
        scaled_time_delta(end.saturating_sub(start), old_speed),
        new_speed,
    ))
}
