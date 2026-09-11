use super::*;

const VISUAL_DURATION_UNITS_PER_SECOND: i64 = 10;
const GPU_MEMORY_UNITS_PER_GIB: i64 = 4;
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeDevicePresentation {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComputeServerPresentation {
    pub summary: String,
    pub version: String,
    pub version_detail: Option<String>,
    pub protocol: String,
    pub torch: String,
    pub cuda: String,
    pub devices: Vec<ComputeDevicePresentation>,
    pub selected_device: Option<usize>,
    pub jobs: String,
    pub reservations: String,
    pub workers: String,
    pub features: String,
}

pub fn present_compute_server(status: &ServerStatus) -> ComputeServerPresentation {
    let version = if status.server.git_short_hash.is_empty() {
        status.server.version.clone()
    } else {
        format!(
            "{} ({})",
            status.server.version, status.server.git_short_hash
        )
    };
    let localized_version =
        shrimply_i18n::text_args("Version %{version}", &[("version", version.clone())]);
    let summary = if status.status.eq_ignore_ascii_case("ok") {
        localized_version
    } else {
        format!("{localized_version} · {}", status.status.to_uppercase())
    };
    let devices = status
        .torch
        .devices
        .iter()
        .map(|device| ComputeDevicePresentation {
            id: device.id.clone(),
            label: device.total_memory_bytes.map_or_else(
                || device.name.clone(),
                |bytes| {
                    format!(
                        "{} · {} ({:.1} GiB)",
                        device.id.to_uppercase().replace(':', " "),
                        device.name,
                        bytes as f64 / BYTES_PER_GIB
                    )
                },
            ),
        })
        .collect::<Vec<_>>();
    let selected_device = status
        .torch
        .devices
        .iter()
        .position(|device| device.id == status.torch.selected_device);
    let workers = status
        .compute
        .workers
        .iter()
        .map(|worker| {
            let configuration = worker
                .configuration
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} · {}{} · {} ×{}",
                worker.service,
                worker.model,
                if configuration.is_empty() {
                    String::new()
                } else {
                    format!(" ({configuration})")
                },
                worker.state,
                worker.copies
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ComputeServerPresentation {
        summary,
        version,
        version_detail: (!status.server.git_hash.is_empty())
            .then(|| status.server.git_hash.clone()),
        protocol: format!("{}.{}", status.protocol.major, status.protocol.minor),
        torch: status.torch.version.clone(),
        cuda: match (&status.torch.cuda_runtime, status.torch.cuda_available) {
            (Some(runtime), true) => {
                shrimply_i18n::text_args("%{runtime} · Available", &[("runtime", runtime.clone())])
            }
            (None, true) => shrimply_i18n::text("Available").into_owned(),
            (_, false) => shrimply_i18n::text("Unavailable").into_owned(),
        },
        devices,
        selected_device,
        jobs: shrimply_i18n::text_args(
            "%{queued} queued · %{active} active",
            &[
                ("queued", status.compute.queued_jobs.to_string()),
                ("active", status.compute.active_jobs.to_string()),
            ],
        ),
        reservations: format!(
            "RAM {:.1} GiB · VRAM {:.1} GiB",
            status.compute.reserved_ram_bytes as f64 / BYTES_PER_GIB,
            status.compute.reserved_vram_bytes as f64 / BYTES_PER_GIB
        ),
        workers: if workers.is_empty() {
            shrimply_i18n::text("None").into_owned()
        } else {
            workers
        },
        features: if status.capabilities.is_empty() {
            shrimply_i18n::text("None").into_owned()
        } else {
            status.capabilities.join(", ")
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferenceId {
    CaptionFontSize,
    CaptionBackgroundColor,
    DefaultTextFontFamily,
    DefaultVisualDuration,
    TimelineSnapRadius,
    PreviewZoomPan,
    PreviewPadding,
    PreviewShadowSize,
    PreviewUpsampleMethod,
    PreviewDownsampleMethod,
    TemporalDecoderPoolSize,
    GpuHostMemory,
    BlenderBinary,
}

impl PreferenceId {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "caption-font-size" => Some(Self::CaptionFontSize),
            "caption-background-color" => Some(Self::CaptionBackgroundColor),
            "default-text-font-family" => Some(Self::DefaultTextFontFamily),
            "default-visual-duration" => Some(Self::DefaultVisualDuration),
            "timeline-snap-radius" => Some(Self::TimelineSnapRadius),
            "preview-zoom-pan" => Some(Self::PreviewZoomPan),
            "preview-padding" => Some(Self::PreviewPadding),
            "preview-shadow-size" => Some(Self::PreviewShadowSize),
            "preview-upsample-method" => Some(Self::PreviewUpsampleMethod),
            "preview-downsample-method" => Some(Self::PreviewDownsampleMethod),
            "temporal-decoder-pool-size" => Some(Self::TemporalDecoderPoolSize),
            "gpu-host-memory" => Some(Self::GpuHostMemory),
            "blender-binary" => Some(Self::BlenderBinary),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreferenceValue {
    Boolean(bool),
    Integer(i64),
    Text(String),
    Color(Color<u8>),
    FontFamily(FontFamily),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerRange {
    pub minimum: i64,
    pub maximum: i64,
    pub step: i64,
    pub scale: i64,
}

pub fn integer_range(id: PreferenceId) -> Option<IntegerRange> {
    let range = match id {
        PreferenceId::CaptionFontSize => IntegerRange {
            minimum: MIN_CAPTION_FONT_SIZE as i64,
            maximum: MAX_CAPTION_FONT_SIZE as i64,
            step: 1,
            scale: 1,
        },
        PreferenceId::DefaultVisualDuration => IntegerRange {
            minimum: shrimply_math_core::fraction_scaled_integer(
                MIN_VISUAL_DURATION.seconds,
                VISUAL_DURATION_UNITS_PER_SECOND,
            ),
            maximum: shrimply_math_core::fraction_scaled_integer(
                MAX_VISUAL_DURATION.seconds,
                VISUAL_DURATION_UNITS_PER_SECOND,
            ),
            step: 1,
            scale: VISUAL_DURATION_UNITS_PER_SECOND,
        },
        PreferenceId::TimelineSnapRadius => IntegerRange {
            minimum: i64::from(MIN_TIMELINE_SNAP_RADIUS_PX),
            maximum: i64::from(MAX_TIMELINE_SNAP_RADIUS_PX),
            step: 1,
            scale: 1,
        },
        PreferenceId::PreviewPadding => IntegerRange {
            minimum: 0,
            maximum: i64::from(MAX_PREVIEW_PADDING_PX),
            step: 1,
            scale: 1,
        },
        PreferenceId::PreviewShadowSize => IntegerRange {
            minimum: 0,
            maximum: i64::from(MAX_PREVIEW_SHADOW_SIZE_PX),
            step: 1,
            scale: 1,
        },
        PreferenceId::PreviewUpsampleMethod => IntegerRange {
            minimum: 0,
            maximum: 1,
            step: 1,
            scale: 1,
        },
        PreferenceId::PreviewDownsampleMethod => IntegerRange {
            minimum: 0,
            maximum: 2,
            step: 1,
            scale: 1,
        },
        PreferenceId::TemporalDecoderPoolSize => IntegerRange {
            minimum: i64::from(MIN_TEMPORAL_DECODER_POOL_SIZE),
            maximum: i64::from(MAX_TEMPORAL_DECODER_POOL_SIZE),
            step: 1,
            scale: 1,
        },
        PreferenceId::GpuHostMemory => IntegerRange {
            minimum: 0,
            maximum: shrimply_math_core::fraction_scaled_integer(
                physical_system_memory_gib(),
                GPU_MEMORY_UNITS_PER_GIB,
            ),
            step: 1,
            scale: GPU_MEMORY_UNITS_PER_GIB,
        },
        PreferenceId::PreviewZoomPan
        | PreferenceId::CaptionBackgroundColor
        | PreferenceId::DefaultTextFontFamily
        | PreferenceId::BlenderBinary => return None,
    };
    Some(range)
}

pub fn value(store: &SharedPreferences, id: PreferenceId) -> PreferenceValue {
    let snapshot = snapshot(store);
    match id {
        PreferenceId::CaptionFontSize => {
            PreferenceValue::Integer(snapshot.caption_font_size.round() as i64)
        }
        PreferenceId::CaptionBackgroundColor => {
            PreferenceValue::Color(snapshot.caption_background_color)
        }
        PreferenceId::DefaultTextFontFamily => {
            PreferenceValue::FontFamily(snapshot.default_text_font_family)
        }
        PreferenceId::DefaultVisualDuration => {
            PreferenceValue::Integer(shrimply_math_core::fraction_scaled_integer(
                snapshot.default_visual_duration.seconds,
                10,
            ))
        }
        PreferenceId::TimelineSnapRadius => {
            PreferenceValue::Integer(i64::from(snapshot.timeline_snap_radius_px))
        }
        PreferenceId::PreviewZoomPan => PreferenceValue::Boolean(snapshot.preview_zoom_pan_enabled),
        PreferenceId::PreviewPadding => {
            PreferenceValue::Integer(i64::from(snapshot.preview_padding_px))
        }
        PreferenceId::PreviewShadowSize => {
            PreferenceValue::Integer(i64::from(snapshot.preview_shadow_size_px))
        }
        PreferenceId::PreviewUpsampleMethod => {
            PreferenceValue::Integer(match snapshot.preview_upsample_method {
                PreviewUpsampleMethod::Nearest => 0,
                PreviewUpsampleMethod::Bilinear => 1,
            })
        }
        PreferenceId::PreviewDownsampleMethod => {
            PreferenceValue::Integer(match snapshot.preview_downsample_method {
                PreviewDownsampleMethod::Nearest => 0,
                PreviewDownsampleMethod::Bilinear => 1,
                PreviewDownsampleMethod::Trilinear => 2,
            })
        }
        PreferenceId::TemporalDecoderPoolSize => {
            PreferenceValue::Integer(i64::from(snapshot.temporal_decoder_pool_size))
        }
        PreferenceId::GpuHostMemory => PreferenceValue::Integer(
            shrimply_math_core::fraction_scaled_integer(snapshot.gpu_host_memory_gib, 4),
        ),
        PreferenceId::BlenderBinary => PreferenceValue::Text(
            snapshot
                .blender_binary
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
        ),
    }
}

pub fn set_value(
    store: &SharedPreferences,
    id: PreferenceId,
    value: PreferenceValue,
) -> Result<(), &'static str> {
    match (id, value) {
        (PreferenceId::CaptionFontSize, PreferenceValue::Integer(value)) => {
            set_caption_font_size(store, value as f32)
        }
        (PreferenceId::CaptionBackgroundColor, PreferenceValue::Color(value)) => {
            set_caption_background_color(store, value)
        }
        (PreferenceId::DefaultTextFontFamily, PreferenceValue::FontFamily(family)) => {
            set_default_text_font_family(store, family)
        }
        (PreferenceId::DefaultVisualDuration, PreferenceValue::Integer(tenths)) => {
            set_default_visual_duration(
                store,
                Time {
                    seconds: Fraction::new_raw(
                        tenths.max(1) as u64,
                        VISUAL_DURATION_UNITS_PER_SECOND as u64,
                    ),
                },
            )
        }
        (PreferenceId::TimelineSnapRadius, PreferenceValue::Integer(value)) => {
            set_timeline_snap_radius_px(store, value.max(0) as u32)
        }
        (PreferenceId::PreviewZoomPan, PreferenceValue::Boolean(value)) => {
            set_preview_zoom_pan_enabled(store, value)
        }
        (PreferenceId::PreviewPadding, PreferenceValue::Integer(value)) => {
            set_preview_padding_px(store, value.max(0) as u32)
        }
        (PreferenceId::PreviewShadowSize, PreferenceValue::Integer(value)) => {
            set_preview_shadow_size_px(store, value.max(0) as u32)
        }
        (PreferenceId::PreviewUpsampleMethod, PreferenceValue::Integer(value)) => {
            let method = match value {
                0 => PreviewUpsampleMethod::Nearest,
                1 => PreviewUpsampleMethod::Bilinear,
                _ => return Err("Unknown preview upsample method"),
            };
            set_preview_upsample_method(store, method);
        }
        (PreferenceId::PreviewDownsampleMethod, PreferenceValue::Integer(value)) => {
            let method = match value {
                0 => PreviewDownsampleMethod::Nearest,
                1 => PreviewDownsampleMethod::Bilinear,
                2 => PreviewDownsampleMethod::Trilinear,
                _ => return Err("Unknown preview downsample method"),
            };
            set_preview_downsample_method(store, method);
        }
        (PreferenceId::TemporalDecoderPoolSize, PreferenceValue::Integer(value)) => {
            set_temporal_decoder_pool_size(store, value.max(0) as u32)
        }
        (PreferenceId::GpuHostMemory, PreferenceValue::Integer(quarters)) => {
            set_gpu_host_memory_gib(
                store,
                Fraction::new_raw(quarters.max(0) as u64, GPU_MEMORY_UNITS_PER_GIB as u64),
            )
        }
        (PreferenceId::BlenderBinary, PreferenceValue::Text(path)) => {
            set_blender_binary(store, (!path.is_empty()).then(|| Path::new(&path)))
        }
        _ => return Err("Preference value has the wrong type"),
    }
    Ok(())
}

pub fn compute_servers(store: &SharedPreferences) -> (Vec<String>, usize) {
    let snapshot = snapshot(store);
    let selected = snapshot
        .compute_server_urls
        .iter()
        .position(|url| url == &snapshot.compute_server_url)
        .unwrap_or_default();
    (snapshot.compute_server_urls, selected)
}

pub fn add_compute_server(store: &SharedPreferences, value: &str) -> Result<(), &'static str> {
    let url = normalize_server_url(value)?;
    let mut urls = snapshot(store).compute_server_urls;
    if !urls.contains(&url) {
        urls.push(url.clone());
        set_compute_server_urls(store, &urls);
    }
    set_compute_server_url(store, &url);
    Ok(())
}

pub fn edit_compute_server(
    store: &SharedPreferences,
    previous: &str,
    value: &str,
) -> Result<(), &'static str> {
    let url = normalize_server_url(value)?;
    let snapshot = snapshot(store);
    if !snapshot
        .compute_server_urls
        .iter()
        .any(|item| item == previous)
    {
        return Err("Server selection is no longer available");
    }
    let selected = snapshot.compute_server_url == previous;
    let mut urls = Vec::with_capacity(snapshot.compute_server_urls.len());
    for item in snapshot.compute_server_urls {
        let item = if item == previous { &url } else { &item };
        if !urls.contains(item) {
            urls.push(item.clone());
        }
    }
    set_compute_server_urls(store, &urls);
    if selected {
        set_compute_server_url(store, &url);
    }
    Ok(())
}

pub fn remove_compute_server(store: &SharedPreferences, url: &str) {
    let snapshot = snapshot(store);
    if snapshot.compute_server_urls.len() <= 1
        || !snapshot.compute_server_urls.iter().any(|item| item == url)
    {
        return;
    }
    let mut urls = snapshot.compute_server_urls;
    urls.retain(|item| item != url);
    set_compute_server_urls(store, &urls);
    if snapshot.compute_server_url == url {
        set_compute_server_url(store, &urls[0]);
    }
}

pub fn select_compute_server(store: &SharedPreferences, url: &str) -> bool {
    if !snapshot(store)
        .compute_server_urls
        .iter()
        .any(|item| item == url)
    {
        return false;
    }
    set_compute_server_url(store, url);
    true
}
