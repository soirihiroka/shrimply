use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value, json};
use shrimply_math_core::{Fraction, time_from_frame};
use shrimply_media_import::video_dimensions;
use shrimply_project_document::project::{
    AudioTransition, Background, BackgroundGenerator, CanvasSize, Color, PROJECT_FORMAT_VERSION,
    SolidColor, Time, Transform, TransitionSide, VideoItem, VideoSampleMethod, VisualSource,
    VisualTransition, default_playback_speed, fraction_denominator, fraction_numerator,
    native_playback_fps,
};
use shrimply_property_model::timeline_value::TimelineValue;

pub use shrimply_project_document::project::InterchangeDocument as ImportResult;

pub fn from_file(
    path: &Path,
    canvas_size: CanvasSize,
    fps: Fraction,
) -> Result<ImportResult, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let document: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    from_value(&document, path, canvas_size, fps)
}

pub fn from_value(
    document: &Value,
    source_path: &Path,
    canvas_size: CanvasSize,
    fps: Fraction,
) -> Result<ImportResult, String> {
    require_schema(document, "Timeline.1")?;
    let tracks = document
        .get("tracks")
        .ok_or_else(|| "OTIO timeline is missing tracks".to_string())?;
    require_schema(tracks, "Stack.1")?;
    let tracks = array_field(tracks, "children", "OTIO track stack")?;
    let base = source_path.parent().unwrap_or_else(|| Path::new("."));
    let mut warnings = Vec::new();
    let mut video_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    let frame_step = time_from_frame(1, fps).ok_or_else(|| "invalid project FPS".to_string())?;

    for (index, track) in tracks.iter().enumerate() {
        require_schema(track, "Track.1")?;
        let enabled = track
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let children = array_field(track, "children", "OTIO track")?;
        match track.get("kind").and_then(Value::as_str) {
            Some("Video") => video_tracks.push(json!({
                "enabled": enabled,
                "items": import_video_items(children, canvas_size, base, frame_step, &mut warnings)?,
            })),
            Some("Audio") => audio_tracks.push(json!({
                "enabled": enabled,
                "gain_db": 0.0,
                "items": import_audio_items(children, base, frame_step, &mut warnings)?,
            })),
            kind => warnings.push(format!(
                "track {} has unsupported kind {}; it was skipped",
                index + 1,
                kind.unwrap_or("unknown")
            )),
        }
    }

    // OTIO and Shrimply both store visual tracks bottom-to-top.
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            source_path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Imported OTIO".to_string());
    Ok(ImportResult {
        project: json!({
            "format_version": PROJECT_FORMAT_VERSION,
            "name": name,
            "fps": fraction_value(fps),
            "canvas_size": canvas_size,
            "caption_tracks": [],
            "visual_tracks": video_tracks,
            "audio_tracks": audio_tracks,
        }),
        warnings,
    })
}

fn import_video_items(
    children: &[Value],
    canvas_size: CanvasSize,
    base: &Path,
    frame_step: Time,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut cursor = Time::ZERO;
    let mut pending_intro: Option<Time> = None;
    for child in children {
        match schema(child) {
            Some("Gap.1") => {
                cursor = cursor
                    .saturating_add(item_duration(child)?)
                    .snapped(frame_step)
            }
            Some("Clip.2") => {
                let duration = item_duration(child)?;
                let end = cursor
                    .saturating_add(duration)
                    .snapped(frame_step)
                    .max(cursor.saturating_add(frame_step));
                if child.get("enabled").and_then(Value::as_bool) == Some(false) {
                    warnings.push(format!(
                        "disabled clip {} was preserved as a gap",
                        item_name(child)
                    ));
                } else {
                    warn_clip_annotations(child, warnings);
                    if let Some(mut item) =
                        visual_clip(child, cursor, end, canvas_size, base, warnings)?
                    {
                        if let Some(duration) = pending_intro.take() {
                            set_transition(
                                &mut item,
                                "intro",
                                &VisualTransition::new(
                                    TransitionSide::Intro,
                                    duration.min(end.saturating_sub(cursor)),
                                    canvas_size,
                                ),
                            )?;
                        }
                        items.push(item);
                    }
                }
                cursor = end;
            }
            Some("Transition.1") => {
                let (intro, outro) = transition_offsets(child)?;
                let transition_type = child
                    .get("transition_type")
                    .and_then(Value::as_str)
                    .unwrap_or("SMPTE_Dissolve");
                if !transition_type.contains("Dissolve") {
                    warnings.push(format!(
                        "unsupported visual transition {transition_type} was skipped"
                    ));
                    continue;
                }
                if let Some(item) = items.last_mut() {
                    let duration = value_item_duration(item)?;
                    set_transition(
                        item,
                        "outro",
                        &VisualTransition::new(
                            TransitionSide::Outro,
                            outro.min(duration),
                            canvas_size,
                        ),
                    )?;
                }
                pending_intro = Some(intro);
            }
            other => {
                preserve_unknown_duration(child, &mut cursor)?;
                cursor = cursor.snapped(frame_step);
                warnings.push(format!(
                    "unsupported OTIO visual item {} was preserved as a gap",
                    other.unwrap_or("without a schema")
                ));
            }
        }
    }
    Ok(items)
}

fn import_audio_items(
    children: &[Value],
    base: &Path,
    frame_step: Time,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut cursor = Time::ZERO;
    let mut pending_intro: Option<Time> = None;
    for child in children {
        match schema(child) {
            Some("Gap.1") => {
                cursor = cursor
                    .saturating_add(item_duration(child)?)
                    .snapped(frame_step)
            }
            Some("Clip.2") => {
                let duration = item_duration(child)?;
                let end = cursor
                    .saturating_add(duration)
                    .snapped(frame_step)
                    .max(cursor.saturating_add(frame_step));
                if child.get("enabled").and_then(Value::as_bool) == Some(false) {
                    warnings.push(format!(
                        "disabled clip {} was preserved as a gap",
                        item_name(child)
                    ));
                } else {
                    warn_clip_annotations(child, warnings);
                    if let Some(mut item) = audio_clip(child, cursor, end, base, warnings)? {
                        if let Some(duration) = pending_intro.take() {
                            set_transition(
                                &mut item,
                                "intro",
                                &AudioTransition::new(
                                    TransitionSide::Intro,
                                    duration.min(end.saturating_sub(cursor)),
                                ),
                            )?;
                        }
                        items.push(item);
                    }
                }
                cursor = end;
            }
            Some("Transition.1") => {
                let (intro, outro) = transition_offsets(child)?;
                if let Some(item) = items.last_mut() {
                    let duration = value_item_duration(item)?;
                    set_transition(
                        item,
                        "outro",
                        &AudioTransition::new(TransitionSide::Outro, outro.min(duration)),
                    )?;
                }
                pending_intro = Some(intro);
            }
            other => {
                preserve_unknown_duration(child, &mut cursor)?;
                cursor = cursor.snapped(frame_step);
                warnings.push(format!(
                    "unsupported OTIO audio item {} was preserved as a gap",
                    other.unwrap_or("without a schema")
                ));
            }
        }
    }
    Ok(items)
}

fn visual_clip(
    clip: &Value,
    start: Time,
    end: Time,
    canvas_size: CanvasSize,
    base: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<Value>, String> {
    let reference = active_reference(clip)?;
    match schema(reference) {
        Some("ExternalReference.1") => {
            external_visual(clip, reference, start, end, canvas_size, base, warnings).map(Some)
        }
        Some("GeneratorReference.1") => {
            solid_color_item(clip, reference, start, end, canvas_size, warnings)
        }
        Some("MissingReference.1") => {
            warnings.push(format!(
                "{} has a missing media reference and was preserved as a gap",
                item_name(clip)
            ));
            Ok(None)
        }
        other => {
            warnings.push(format!(
                "{} uses unsupported media reference {} and was preserved as a gap",
                item_name(clip),
                other.unwrap_or("without a schema")
            ));
            Ok(None)
        }
    }
}

fn audio_clip(
    clip: &Value,
    start: Time,
    end: Time,
    base: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<Value>, String> {
    let reference = active_reference(clip)?;
    if schema(reference) == Some("MissingReference.1") {
        warnings.push(format!(
            "{} has a missing media reference and was preserved as a gap",
            item_name(clip)
        ));
        return Ok(None);
    }
    if schema(reference) != Some("ExternalReference.1") {
        warnings.push(format!(
            "{} uses an unsupported audio reference and was preserved as a gap",
            item_name(clip)
        ));
        return Ok(None);
    }
    let path = media_path(reference, base)?;
    warn_missing_media(&path, warnings);
    Ok(Some(json!({
        "start": time_value(start)?,
        "end": time_value(end)?,
        "time_offset": time_value(source_offset(clip)?)?,
        "source_duration": time_value(available_duration(reference).unwrap_or_else(|| end.saturating_sub(start)))?,
        "playback_speed": fraction_value(default_playback_speed()),
        "repeat_strategy": "hold",
        "speed_method": "preserve_pitch",
        "track_id": 0,
        "transitions": { "intro": null, "outro": null },
        "file": path,
    })))
}

fn external_visual(
    clip: &Value,
    reference: &Value,
    start: Time,
    end: Time,
    canvas_size: CanvasSize,
    base: &Path,
    warnings: &mut Vec<String>,
) -> Result<Value, String> {
    let path = media_path(reference, base)?;
    warn_missing_media(&path, warnings);
    let (content, dimensions) =
        match shrimply_media_import::image_reference(&path, canvas_size, end.saturating_sub(start))
        {
            Some((content, size)) => (content, size.ok()),
            None => (VisualSource::Media, video_dimensions(&path, 0)),
        };
    let transform = dimensions
        .map(|size| Transform::natural_size(canvas_size, size.x, size.y))
        .unwrap_or_else(|| Transform::fill(canvas_size));
    let mut item = VideoItem::background_item(canvas_size, start, end);
    if matches!(&content, VisualSource::LayeredImage(_)) {
        item.sample_method = TimelineValue::new_const(VideoSampleMethod::Nearest);
    }
    let mut item = serde_json::to_value(item)
        .map_err(|error| format!("could not create visual item: {error}"))?;
    let fields = object_mut(&mut item, "visual item")?;
    fields.insert("time_offset".into(), time_value(source_offset(clip)?)?);
    fields.insert(
        "source_duration".into(),
        time_value(available_duration(reference).unwrap_or_else(|| end.saturating_sub(start)))?,
    );
    fields.insert(
        "playback_fps".into(),
        fraction_value(available_rate(reference).unwrap_or_else(native_playback_fps)),
    );
    fields.insert(
        "transform".into(),
        serde_json::to_value(&transform)
            .map_err(|error| format!("could not encode visual transform: {error}"))?,
    );
    fields.insert(
        "default_transform".into(),
        serde_json::to_value(&transform)
            .map_err(|error| format!("could not encode default visual transform: {error}"))?,
    );
    let dimensions = dimensions.unwrap_or_default();
    fields.insert("source_width".into(), dimensions.x.into());
    fields.insert("source_height".into(), dimensions.y.into());
    fields.insert(
        "content".into(),
        serde_json::to_value(content)
            .map_err(|error| format!("could not encode visual source: {error}"))?,
    );
    fields.insert(
        "file".into(),
        Value::String(path.to_string_lossy().into_owned()),
    );
    fields.insert("track_id".into(), 0.into());
    Ok(item)
}

fn solid_color_item(
    clip: &Value,
    reference: &Value,
    start: Time,
    end: Time,
    canvas_size: CanvasSize,
    warnings: &mut Vec<String>,
) -> Result<Option<Value>, String> {
    let kind = reference.get("generator_kind").and_then(Value::as_str);
    if kind != Some("kdenlive:SolidColor") {
        warnings.push(format!(
            "{} uses unsupported generator {} and was preserved as a gap",
            item_name(clip),
            kind.unwrap_or("unknown")
        ));
        return Ok(None);
    }
    let color = reference
        .pointer("/parameters/kdenlive/color")
        .and_then(Value::as_str)
        .and_then(parse_kdenlive_color)
        .unwrap_or(Color::<u8>::BLACK);
    let mut item = VideoItem::background_item(canvas_size, start, end);
    item.source_duration = end.saturating_sub(start);
    item.content = VisualSource::Background(Box::new(Background {
        generator: BackgroundGenerator::SolidColor(Box::new(SolidColor {
            color: TimelineValue::new_const(color),
        })),
    }));
    serde_json::to_value(item)
        .map(Some)
        .map_err(|error| format!("could not create solid-color item: {error}"))
}

fn active_reference(clip: &Value) -> Result<&Value, String> {
    let key = clip
        .get("active_media_reference_key")
        .and_then(Value::as_str)
        .unwrap_or("DEFAULT_MEDIA");
    object_field(clip, "media_references", "clip")?
        .get(key)
        .ok_or_else(|| {
            format!(
                "{} is missing active media reference {key}",
                item_name(clip)
            )
        })
}

fn media_path(reference: &Value, base: &Path) -> Result<PathBuf, String> {
    let target = reference
        .get("target_url")
        .and_then(Value::as_str)
        .ok_or_else(|| "external reference is missing target_url".to_string())?;
    shrimply_media_import::interchange_path(base, target)
}

fn warn_missing_media(path: &Path, warnings: &mut Vec<String>) {
    if !path.exists() {
        warnings.push(format!("media {} does not exist", path.display()));
    }
}

fn source_offset(clip: &Value) -> Result<Time, String> {
    rational_time(
        clip.pointer("/source_range/start_time")
            .ok_or_else(|| format!("{} is missing source start time", item_name(clip)))?,
    )
}

fn available_duration(reference: &Value) -> Option<Time> {
    rational_time(reference.pointer("/available_range/duration")?).ok()
}

fn available_rate(reference: &Value) -> Option<Fraction> {
    positive_fraction(reference.pointer("/available_range/duration/rate")?).ok()
}

fn item_duration(item: &Value) -> Result<Time, String> {
    rational_time(
        item.pointer("/source_range/duration")
            .ok_or_else(|| format!("{} is missing source duration", item_name(item)))?,
    )
}

fn value_item_duration(item: &Value) -> Result<Time, String> {
    let start: Time = serde_json::from_value(
        item.get("start")
            .cloned()
            .ok_or_else(|| "imported item is missing start".to_string())?,
    )
    .map_err(|error| format!("could not read imported item start: {error}"))?;
    let end: Time = serde_json::from_value(
        item.get("end")
            .cloned()
            .ok_or_else(|| "imported item is missing end".to_string())?,
    )
    .map_err(|error| format!("could not read imported item end: {error}"))?;
    Ok(end.saturating_sub(start))
}

fn transition_offsets(transition: &Value) -> Result<(Time, Time), String> {
    // OTIO in_offset overlaps the previous item; out_offset overlaps the next item.
    let next_intro = rational_time(
        transition
            .get("out_offset")
            .ok_or_else(|| "OTIO transition is missing out_offset".to_string())?,
    )?;
    let previous_outro = rational_time(
        transition
            .get("in_offset")
            .ok_or_else(|| "OTIO transition is missing in_offset".to_string())?,
    )?;
    Ok((next_intro, previous_outro))
}

fn set_transition<T: serde::Serialize>(
    item: &mut Value,
    side: &str,
    transition: &T,
) -> Result<(), String> {
    let transitions = object_mut(item, "imported item")?
        .entry("transitions")
        .or_insert_with(|| json!({ "intro": null, "outro": null }));
    object_mut(transitions, "item transitions")?.insert(
        side.to_string(),
        serde_json::to_value(transition)
            .map_err(|error| format!("could not encode transition: {error}"))?,
    );
    Ok(())
}

fn rational_time(value: &Value) -> Result<Time, String> {
    let value_number = value
        .get("value")
        .ok_or_else(|| "rational time is missing value".to_string())?;
    let rate_number = value
        .get("rate")
        .ok_or_else(|| "rational time is missing rate".to_string())?;
    Ok(Time {
        seconds: fraction_number(value_number)? / positive_fraction(rate_number)?,
    })
}

fn positive_fraction(value: &Value) -> Result<Fraction, String> {
    let fraction = fraction_number(value)?;
    if fraction <= Fraction::from(0u32) {
        return Err("rational time rate must be positive".to_string());
    }
    Ok(fraction)
}

fn fraction_number(value: &Value) -> Result<Fraction, String> {
    let number = value
        .as_number()
        .ok_or_else(|| "OTIO time component is not a number".to_string())?;
    parse_fraction(number)
}

fn parse_fraction(number: &Number) -> Result<Fraction, String> {
    number
        .to_string()
        .parse()
        .map_err(|error| format!("invalid OTIO number {number}: {error}"))
}

fn preserve_unknown_duration(item: &Value, cursor: &mut Time) -> Result<(), String> {
    if item.pointer("/source_range/duration").is_some() {
        *cursor = cursor.saturating_add(item_duration(item)?);
    }
    Ok(())
}

fn time_value(time: Time) -> Result<Value, String> {
    serde_json::to_value(time).map_err(|error| format!("could not encode time: {error}"))
}

fn fraction_value(value: Fraction) -> Value {
    json!({
        "numerator": fraction_numerator(value),
        "denominator": fraction_denominator(value),
    })
}

fn warn_clip_annotations(clip: &Value, warnings: &mut Vec<String>) {
    for (field, label) in [("effects", "effects"), ("markers", "markers")] {
        if clip
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
        {
            warnings.push(format!(
                "{} contains OTIO {label} that were not imported",
                item_name(clip)
            ));
        }
    }
}

fn parse_kdenlive_color(value: &str) -> Option<Color<u8>> {
    let raw = u32::from_str_radix(value.trim_start_matches("0x"), 16).ok()?;
    Some(Color::<u8>::from_rgba(
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
        (raw >> 24) as u8,
    ))
}

fn item_name(item: &Value) -> &str {
    item.get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("unnamed clip")
}

fn schema(value: &Value) -> Option<&str> {
    value.get("OTIO_SCHEMA").and_then(Value::as_str)
}

fn require_schema(value: &Value, expected: &str) -> Result<(), String> {
    let actual = schema(value).unwrap_or("missing");
    if actual != expected {
        return Err(format!("expected OTIO schema {expected}, found {actual}"));
    }
    Ok(())
}

fn object_field<'a>(
    value: &'a Value,
    field: &str,
    label: &str,
) -> Result<&'a Map<String, Value>, String> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{label} is missing object field {field}"))
}

fn object_mut<'a>(value: &'a mut Value, label: &str) -> Result<&'a mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| format!("{label} is not a JSON object"))
}

fn array_field<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a [Value], String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{label} is missing array field {field}"))
}
