use super::{
    DEFAULT_AUDIO_BITRATE_KBPS, DEFAULT_AUDIO_SAMPLE_RATE, DEFAULT_CONSTANT_QP,
    DEFAULT_VIDEO_BITRATE_KBPS, Encoders, VideoEncoder, frame_rates,
};
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_project_document::project::Project;

fn field(id: &str, label: &str, group: &str, widget: Value, value: Value) -> Value {
    let mut field = widget;
    field["id"] = json!(id);
    field["label"] = json!(text(label));
    field["group"] = json!(text(group));
    field["value"] = value;
    field["enabled"] = json!(true);
    field["visible"] = json!(true);
    field
}

pub(super) fn fields(project: &Project, encoders: &Encoders) -> Vec<Value> {
    let backend = encoders.form_encoder();
    let nvenc = backend.is_nvenc();
    let media = backend.is_mediacodec();
    let rates: Vec<_> = frame_rates(project.fps)
        .into_iter()
        .map(|(value, label)| json!({"value":value.to_string(),"label":label}))
        .collect();
    let rate_controls: Vec<_> = super::rate_controls(backend)
        .iter()
        .map(|(value, label)| json!({"value":value,"label":text(label)}))
        .collect();
    let audio: Vec<_> = crate::video::available_audio_encoders()
        .map(|(value, label)| json!({"value":value,"label":text(label)}))
        .collect();
    let mut fields = vec![
        field(
            "video_codec",
            "Export format",
            "Video format",
            json!({"widget":"choice","type":"string","choices":encoders.formats().iter().map(|encoder| { let format = encoder.format(); json!({"value":format.value,"label":text(format.label)}) }).collect::<Vec<_>>()}),
            json!(
                encoders
                    .preferred(super::ExportVideoCodec::H265)
                    .format()
                    .value
            ),
        ),
        field(
            "container",
            "Container",
            "Video format",
            json!({"widget":"choice","type":"string","choices":[{"value":"mp4","label":"MP4"},{"value":"mkv","label":"MKV"}]}),
            json!("mp4"),
        ),
        field(
            "rate_control",
            "Rate Control",
            "Encoder settings",
            json!({"widget":"choice","type":"string","choices":rate_controls}),
            json!(if media {
                "variable_bitrate"
            } else {
                "constant_qp"
            }),
        ),
        field(
            "bitrate_kbps",
            "Video Bitrate",
            "Encoder settings",
            json!({"widget":"number","type":"integer","range":[50,250000,50],"subtitle":text("Kbps for CBR/VBR")}),
            json!(DEFAULT_VIDEO_BITRATE_KBPS),
        ),
        field(
            "constant_qp",
            "Constant QP",
            "Encoder settings",
            json!({"widget":"number","type":"integer","range":[0,51,1],"subtitle":text("Lower values are higher quality")}),
            json!(DEFAULT_CONSTANT_QP),
        ),
        field(
            "keyframe_interval_seconds",
            "Keyframe Interval",
            "Encoder settings",
            json!({"widget":"number","type":"integer","range":[0,10,1],"subtitle":text("Seconds; 0 lets the encoder choose")}),
            json!(0),
        ),
        field(
            "b_frames",
            "B Frames",
            "Encoder settings",
            json!({"widget":"number","type":"integer","range":[0,if backend.is_vaapi() {2} else {16},1]}),
            json!(if media { 0 } else { 2 }),
        ),
        field(
            "fps",
            "Frame rate",
            "Output",
            json!({"widget":"choice","type":"string","choices":rates}),
            json!(project.fps.to_string()),
        ),
        field(
            "background_alpha",
            "Background Alpha",
            "Output",
            json!({"widget":"number","type":"integer","range":[0,255,1],"subtitle":text("Values below 128 are transparent in GIF output")}),
            json!(0),
        ),
        field(
            "audio_encoder",
            "Audio Encoder",
            "Audio",
            json!({"widget":"choice","type":"string","choices":audio}),
            json!(super::ExportAudioEncoder::default()),
        ),
        field(
            "audio_sample_rate",
            "Audio Sample Rate",
            "Audio",
            json!({"widget":"choice","type":"integer","choices":[{"value":"44100","label":"44100"},{"value":"48000","label":"48000"},{"value":"96000","label":"96000"}]}),
            json!(DEFAULT_AUDIO_SAMPLE_RATE),
        ),
        field(
            "audio_bitrate_kbps",
            "Audio Bitrate",
            "Audio",
            json!({"widget":"number","type":"integer","range":[32,512,8],"subtitle":text("Kbps")}),
            json!(DEFAULT_AUDIO_BITRATE_KBPS),
        ),
    ];
    if !media {
        fields.push(field("max_bitrate_kbps", "Max Video Bitrate", "Encoder settings", json!({"widget":"number","type":"integer","range":[50,250000,50],"subtitle":text("Kbps for VBR")}), json!(DEFAULT_VIDEO_BITRATE_KBPS)));
    }
    if !media && backend.format().hardware {
        fields.push(field("profile", "Profile", "Encoder settings", json!({"widget":"choice","type":"string","choices":[{"value":"main","label":"Main"},{"value":"main10","label":"Main10"}]}), json!("main")));
    }
    if nvenc || matches!(backend, VideoEncoder::X264 | VideoEncoder::X265) {
        fields.push(field("target_quality", "Target Quality", "Encoder settings", json!({"widget":"number","type":"integer","range":[1,51,1],"subtitle":text("Lower values are higher quality")}), json!(20)));
    }
    if nvenc {
        fields.extend([
            field("preset", "Preset", "Encoder settings", json!({"widget":"choice","type":"string","choices":[
                {"value":"p1","label":text("P1: Fastest (Lowest Quality)")},{"value":"p2","label":text("P2: Faster (Lower Quality)")},{"value":"p3","label":text("P3: Fast (Low Quality)")},{"value":"p4","label":text("P4: Medium (Medium Quality)")},{"value":"p5","label":text("P5: Slow (Good Quality)")},{"value":"p6","label":text("P6: Slower (Better Quality)")},{"value":"p7","label":text("P7: Slowest (Best Quality)")}
            ]}), json!("p6")),
            field("tuning", "Tuning", "Encoder settings", json!({"widget":"choice","type":"string","choices":[{"value":"ultra_high_quality","label":text("Ultra High Quality")},{"value":"high_quality","label":text("High Quality")},{"value":"low_latency","label":text("Low Latency")},{"value":"ultra_low_latency","label":text("Ultra Low Latency")}]}), json!("high_quality")),
            field("multipass", "Multi Pass", "Encoder settings", json!({"widget":"choice","type":"string","choices":[{"value":"single_pass","label":text("Single Pass")},{"value":"quarter_resolution","label":text("Two Passes (Quarter Resolution)")},{"value":"full_resolution","label":text("Two Passes (Full Resolution)")}]}), json!("quarter_resolution")),
            field("look_ahead", "Look-ahead", "Encoder settings", json!({"widget":"toggle","type":"boolean"}), json!(true)),
            field("adaptive_quantization", "Adaptive Quantization", "Encoder settings", json!({"widget":"toggle","type":"boolean"}), json!(true)),
            field("b_frame_as_reference", "B Frame as Reference", "Encoder settings", json!({"widget":"toggle","type":"boolean"}), json!(false)),
        ]);
    }
    const ORDER: &[&str] = &[
        "video_codec",
        "container",
        "rate_control",
        "bitrate_kbps",
        "max_bitrate_kbps",
        "target_quality",
        "constant_qp",
        "keyframe_interval_seconds",
        "preset",
        "tuning",
        "multipass",
        "profile",
        "look_ahead",
        "adaptive_quantization",
        "b_frames",
        "b_frame_as_reference",
        "fps",
        "background_alpha",
        "audio_encoder",
        "audio_sample_rate",
        "audio_bitrate_kbps",
    ];
    fields.sort_by_key(|field| {
        ORDER
            .iter()
            .position(|id| field["id"] == *id)
            .expect("ordered export field")
    });
    fields
}

pub fn schema(project: &Project, encoders: &Encoders, input: Option<&Value>) -> Value {
    let mut fields = fields(project, encoders);
    let defaults: serde_json::Map<String, Value> = fields
        .iter()
        .map(|field| {
            (
                field["id"].as_str().expect("field id").to_owned(),
                field["value"].clone(),
            )
        })
        .collect();
    let mut values = Value::Object(defaults.clone());
    if let Some(input) = input.and_then(Value::as_object) {
        for (key, value) in input {
            if values.get(key).is_some() {
                values[key] = value.clone();
            }
        }
    }
    if let Some(selected) = values["fps"].as_str()
        && let Some(field) = fields.iter_mut().find(|field| field["id"] == "fps")
    {
        let choices = field["choices"].as_array_mut().expect("frame rate choices");
        if !choices.iter().any(|choice| choice["value"] == selected) {
            let label = selected
                .parse::<shrimply_math_core::Fraction>()
                .ok()
                .filter(|rate| {
                    shrimply_math_core::fraction_is_finite(*rate)
                        && *rate > shrimply_math_core::Fraction::from(0u8)
                })
                .map(|rate| {
                    frame_rates(rate)
                        .into_iter()
                        .find(|(value, _)| *value == rate)
                        .expect("selected exact frame rate")
                        .1
                })
                .unwrap_or_else(|| selected.to_owned());
            choices.push(json!({"value":selected,"label":label}));
        }
    }
    let selected = values["video_codec"]
        .as_str()
        .and_then(|value| encoders.resolve(value))
        .unwrap_or_else(|| encoders.default_encoder());
    let format = selected.format();
    values["video_codec"] = json!(format.value);
    let gif = selected == VideoEncoder::Gif;
    let software = !format.hardware;
    let rate = values["rate_control"].as_str().unwrap_or_default();
    for field in &mut fields {
        let id = field["id"].as_str().expect("field id").to_owned();
        field["value"] = values[&id].clone();
        field["visible"] = json!(match id.as_str() {
            "container" => !gif,
            "background_alpha" => gif,
            "bitrate_kbps" =>
                !gif && matches!(
                    rate,
                    "constant_bitrate" | "variable_bitrate" | "variable_bitrate_target_quality"
                ),
            "max_bitrate_kbps" =>
                !gif && matches!(rate, "variable_bitrate" | "variable_bitrate_target_quality"),
            "target_quality" => !gif && rate == "variable_bitrate_target_quality",
            "constant_qp" => !gif && rate == "constant_qp",
            "tuning" => !gif && !software && rate != "lossless",
            "preset"
            | "multipass"
            | "profile"
            | "look_ahead"
            | "adaptive_quantization"
            | "b_frame_as_reference" => !gif && !software,
            "video_codec" | "fps" => true,
            _ => !gif,
        });
    }
    let groups: Vec<_> = ["Video format","Encoder settings","Output","Audio"].into_iter().map(|group| {
        let label = text(group);
        json!({"label":label,"visible":fields.iter().any(|field|field["group"] == label.as_ref() && field["visible"] == true)})
    }).collect();
    json!({"title":text("Export Video"),"submit_label":text("Export"),"cancel_label":text("Cancel"),
        "filename":crate::output::default_filename(project,"mp4"),"defaults":defaults,"values":values,"groups":groups,"fields":fields})
}

pub(super) fn normalize(
    project: &Project,
    encoders: &Encoders,
    input: &Value,
) -> Result<(VideoEncoder, Value), String> {
    let fields = fields(project, encoders);
    let mut values = serde_json::Map::new();
    for field in &fields {
        values.insert(
            field["id"].as_str().expect("field id").to_owned(),
            field["value"].clone(),
        );
    }
    let input = input
        .get("values")
        .unwrap_or(input)
        .as_object()
        .ok_or("Export settings must be an object")?;
    let video_encoder = match input.get("video_codec") {
        Some(value) => value
            .as_str()
            .and_then(|value| encoders.resolve(value))
            .ok_or("Selected video format is unavailable")?,
        None => encoders.default_encoder(),
    };
    for (id, value) in input {
        if matches!(id.as_str(), "op" | "project_path" | "video_codec") {
            continue;
        }
        let field = fields
            .iter()
            .find(|field| field["id"] == id.as_str())
            .ok_or_else(|| format!("This encoder does not support export field {id}"))?;
        let label = field["label"].as_str().expect("field label");
        let value = match field["type"].as_str().expect("field type") {
            "integer" => json!(
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                    .ok_or_else(|| format!("{label} must be a whole number"))?
            ),
            "boolean" => json!(
                value
                    .as_bool()
                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                    .ok_or_else(|| format!("{label} requires a boolean"))?
            ),
            "string" => json!(
                value
                    .as_str()
                    .ok_or_else(|| format!("{label} requires text"))?
            ),
            _ => unreachable!("export field type"),
        };
        if let Some(range) = field.get("range") {
            let number = value.as_u64().expect("integer field");
            if number < range[0].as_u64().expect("minimum")
                || number > range[1].as_u64().expect("maximum")
            {
                return Err(format!("{label} is outside its supported range"));
            }
        }
        if id != "fps"
            && let Some(choices) = field.get("choices").and_then(Value::as_array)
        {
            let selected = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            if !choices.iter().any(|choice| choice["value"] == selected) {
                return Err(format!("{label} is unavailable"));
            }
        }
        values.insert(id.clone(), value);
    }
    Ok((video_encoder, Value::Object(values)))
}
