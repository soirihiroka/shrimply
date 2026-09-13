use rusqlite::{Connection, OptionalExtension, params};
use shrimply_math_color::Color;
use shrimply_math_core::{Fraction, Time};
use shrimply_project_document::project::DEFAULT_TEXT_FONT_FAMILY;
pub use shrimply_project_document::project::FontFamily;
use shrimply_recent_projects::settings_db_path;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;

mod ui;
pub use ui::*;

const KEY_CAPTION_FONT_SIZE: &str = "caption_font_size";
const KEY_CAPTION_BACKGROUND_COLOR: &str = "caption_background_color";
const KEY_TIMELINE_MAGNET: &str = "timeline_magnet";
const KEY_TIMELINE_BEAT_GRID: &str = "timeline_beat_grid";
const KEY_TIMELINE_SNAP_RADIUS_PX: &str = "timeline_snap_radius_px";
const KEY_TIMELINE_CURSOR: &str = "timeline_cursor";
const KEY_TIMELINE_DRAG_COLLISION_MODE: &str = "timeline_drag_collision_mode";
const KEY_DEFAULT_VISUAL_DURATION_SECONDS: &str = "default_visual_duration_seconds";
const KEY_DEFAULT_TEXT_FONT_FAMILY: &str = "default_text_font_family";
const KEY_COMPUTE_SERVER_URL: &str = "compute_server_url";
const KEY_COMPUTE_SERVER_URLS: &str = "compute_server_urls";
const KEY_LAST_STT_MODEL: &str = "last_stt_model";
const KEY_LAST_TTS_MODEL: &str = "last_tts_model";
const KEY_PREVIEW_IMAGE_FOLDER: &str = "preview_image_folder";
const KEY_PREVIEW_PADDING_PX: &str = "preview_padding_px";
const KEY_PREVIEW_SHADOW_SIZE_PX: &str = "preview_shadow_size_px";
const KEY_PREVIEW_UPSAMPLE_METHOD: &str = "preview_upsample_method";
const KEY_PREVIEW_DOWNSAMPLE_METHOD: &str = "preview_downsample_method";
const KEY_PREVIEW_ZOOM_PAN_ENABLED: &str = "preview_zoom_pan_enabled";
const KEY_PREVIEW_GUIDES_VISIBLE: &str = "preview_guides_visible";
const KEY_TEMPORAL_DECODER_POOL_SIZE: &str = "temporal_decoder_pool_size";
const KEY_GPU_HOST_MEMORY_GIB: &str = "gpu_host_memory_gib";
const KEY_BLENDER_BINARY: &str = "blender_binary";
const DEFAULT_CAPTION_FONT_SIZE: f32 = 32.0;
const DEFAULT_CAPTION_BACKGROUND_COLOR: Color<u8> = Color::new(0, 0, 0, 204);
const DEFAULT_TIMELINE_MAGNET: &str = "true";
const DEFAULT_TIMELINE_BEAT_GRID: &str = "false";
const DEFAULT_TIMELINE_SNAP_RADIUS_PX: u32 = 10;
const MIN_TIMELINE_SNAP_RADIUS_PX: u32 = 1;
const MAX_TIMELINE_SNAP_RADIUS_PX: u32 = 50;
const DEFAULT_PREVIEW_PADDING_PX: u32 = 20;
const DEFAULT_PREVIEW_SHADOW_SIZE_PX: u32 = 20;
const DEFAULT_PREVIEW_UPSAMPLE_METHOD: PreviewUpsampleMethod = PreviewUpsampleMethod::Bilinear;
const DEFAULT_PREVIEW_DOWNSAMPLE_METHOD: PreviewDownsampleMethod =
    PreviewDownsampleMethod::Trilinear;
const DEFAULT_PREVIEW_GUIDES_VISIBLE: bool = false;
const DEFAULT_TEMPORAL_DECODER_POOL_SIZE: u32 = 16;
const MAX_PREVIEW_PADDING_PX: u32 = 200;
const MAX_PREVIEW_SHADOW_SIZE_PX: u32 = 200;
const MIN_TEMPORAL_DECODER_POOL_SIZE: u32 = 1;
const MAX_TEMPORAL_DECODER_POOL_SIZE: u32 = 256;
const GIB_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_TIMELINE_CURSOR: &str = "pointer";
const DEFAULT_TIMELINE_DRAG_COLLISION_MODE: &str = "overwrite";
const DEFAULT_VISUAL_DURATION: Time = Time {
    seconds: Fraction::new_raw(3, 1),
};
const DEFAULT_COMPUTE_SERVER_URL: &str = "http://127.0.0.1:8787";
const MIN_CAPTION_FONT_SIZE: f32 = 8.0;
const MAX_CAPTION_FONT_SIZE: f32 = 240.0;
const MIN_VISUAL_DURATION: Time = Time {
    seconds: Fraction::new_raw(1, 10),
};
const MAX_VISUAL_DURATION: Time = Time {
    seconds: Fraction::new_raw(3600, 1),
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewUpsampleMethod {
    Nearest,
    Bilinear,
}

impl PreviewUpsampleMethod {
    const fn key(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Bilinear => "bilinear",
        }
    }

    fn from_key(value: &str) -> Option<Self> {
        match value {
            "nearest" => Some(Self::Nearest),
            "bilinear" => Some(Self::Bilinear),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewDownsampleMethod {
    Nearest,
    Bilinear,
    Trilinear,
}

impl PreviewDownsampleMethod {
    const fn key(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Bilinear => "bilinear",
            Self::Trilinear => "trilinear",
        }
    }

    fn from_key(value: &str) -> Option<Self> {
        match value {
            "nearest" => Some(Self::Nearest),
            "bilinear" => Some(Self::Bilinear),
            "trilinear" => Some(Self::Trilinear),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct PreferencesSnapshot {
    pub caption_font_size: f32,
    pub caption_background_color: Color<u8>,
    pub timeline_magnet: String,
    pub timeline_beat_grid: String,
    pub timeline_snap_radius_px: u32,
    pub timeline_cursor: String,
    pub timeline_drag_collision_mode: String,
    pub default_visual_duration: Time,
    pub default_text_font_family: FontFamily,
    pub compute_server_url: String,
    pub compute_server_urls: Vec<String>,
    pub last_stt_model: String,
    pub last_tts_model: String,
    pub preview_padding_px: u32,
    pub preview_shadow_size_px: u32,
    pub preview_upsample_method: PreviewUpsampleMethod,
    pub preview_downsample_method: PreviewDownsampleMethod,
    pub preview_zoom_pan_enabled: bool,
    pub preview_guides_visible: bool,
    pub temporal_decoder_pool_size: u32,
    pub gpu_host_memory_gib: Fraction,
    pub blender_binary: Option<PathBuf>,
}

pub type SharedPreferences = Rc<RefCell<PreferencesStore>>;
type PreferenceListener = Rc<dyn Fn(PreferencesSnapshot)>;

#[derive(Clone)]
struct PreferenceListenerEntry {
    listener: PreferenceListener,
}

pub struct PreferencesStore {
    caption_font_size: f32,
    caption_background_color: Color<u8>,
    timeline_magnet: String,
    timeline_beat_grid: String,
    timeline_snap_radius_px: u32,
    timeline_cursor: String,
    timeline_drag_collision_mode: String,
    default_visual_duration: Time,
    default_text_font_family: FontFamily,
    compute_server_url: String,
    compute_server_urls: Vec<String>,
    last_stt_model: String,
    last_tts_model: String,
    preview_padding_px: u32,
    preview_shadow_size_px: u32,
    preview_upsample_method: PreviewUpsampleMethod,
    preview_downsample_method: PreviewDownsampleMethod,
    preview_zoom_pan_enabled: bool,
    preview_guides_visible: bool,
    temporal_decoder_pool_size: u32,
    gpu_host_memory_gib: Fraction,
    blender_binary: Option<PathBuf>,
    conn: Option<Connection>,
    listeners: Vec<PreferenceListenerEntry>,
}

impl PreferencesStore {
    fn new_with_conn(conn: Option<Connection>) -> Self {
        let caption_font_size = conn.as_ref().map_or(DEFAULT_CAPTION_FONT_SIZE, |conn| {
            read_f32_or_default(
                conn,
                KEY_CAPTION_FONT_SIZE,
                DEFAULT_CAPTION_FONT_SIZE,
                MIN_CAPTION_FONT_SIZE,
                MAX_CAPTION_FONT_SIZE,
            )
        });
        let caption_background_color =
            conn.as_ref()
                .map_or(DEFAULT_CAPTION_BACKGROUND_COLOR, |conn| {
                    Color::<u8>::from(read_u32_or_default(
                        conn,
                        KEY_CAPTION_BACKGROUND_COLOR,
                        DEFAULT_CAPTION_BACKGROUND_COLOR.to_rgba_u32(),
                        u32::MAX,
                    ))
                });
        let timeline_magnet = conn.as_ref().map_or_else(
            || DEFAULT_TIMELINE_MAGNET.to_string(),
            |conn| {
                read_string_or_default(
                    conn,
                    KEY_TIMELINE_MAGNET,
                    DEFAULT_TIMELINE_MAGNET,
                    |value| matches!(value, "true" | "false"),
                )
            },
        );
        let timeline_beat_grid = conn.as_ref().map_or_else(
            || DEFAULT_TIMELINE_BEAT_GRID.to_string(),
            |conn| {
                read_string_or_default(
                    conn,
                    KEY_TIMELINE_BEAT_GRID,
                    DEFAULT_TIMELINE_BEAT_GRID,
                    |value| matches!(value, "true" | "false"),
                )
            },
        );
        let timeline_snap_radius_px =
            conn.as_ref()
                .map_or(DEFAULT_TIMELINE_SNAP_RADIUS_PX, |conn| {
                    read_u32_in_range_or_default(
                        conn,
                        KEY_TIMELINE_SNAP_RADIUS_PX,
                        DEFAULT_TIMELINE_SNAP_RADIUS_PX,
                        MIN_TIMELINE_SNAP_RADIUS_PX,
                        MAX_TIMELINE_SNAP_RADIUS_PX,
                    )
                });
        let timeline_cursor = conn.as_ref().map_or_else(
            || DEFAULT_TIMELINE_CURSOR.to_string(),
            |conn| {
                read_string_or_default(
                    conn,
                    KEY_TIMELINE_CURSOR,
                    DEFAULT_TIMELINE_CURSOR,
                    |value| matches!(value, "pointer" | "cut"),
                )
            },
        );
        let timeline_drag_collision_mode = conn.as_ref().map_or_else(
            || DEFAULT_TIMELINE_DRAG_COLLISION_MODE.to_string(),
            |conn| {
                read_string_or_default(
                    conn,
                    KEY_TIMELINE_DRAG_COLLISION_MODE,
                    DEFAULT_TIMELINE_DRAG_COLLISION_MODE,
                    |value| matches!(value, "overwrite" | "block" | "new_track"),
                )
            },
        );
        let default_visual_duration = conn.as_ref().map_or(DEFAULT_VISUAL_DURATION, |conn| {
            Time::from_seconds_f64(f64::from(read_f32_or_default(
                conn,
                KEY_DEFAULT_VISUAL_DURATION_SECONDS,
                DEFAULT_VISUAL_DURATION.as_secs_f64() as f32,
                MIN_VISUAL_DURATION.as_secs_f64() as f32,
                MAX_VISUAL_DURATION.as_secs_f64() as f32,
            )))
        });
        let default_text_font_family =
            conn.as_ref().map_or_else(default_text_font_family, |conn| {
                decode_font_family(&read_string_or_default(
                    conn,
                    KEY_DEFAULT_TEXT_FONT_FAMILY,
                    &encode_font_family(&default_text_font_family()),
                    |value| decode_font_family(value).is_some(),
                ))
                .expect("validated default text font preference")
            });
        let mut compute_server_url = conn.as_ref().map_or_else(
            || DEFAULT_COMPUTE_SERVER_URL.to_string(),
            |conn| {
                read_string_or_default(
                    conn,
                    KEY_COMPUTE_SERVER_URL,
                    DEFAULT_COMPUTE_SERVER_URL,
                    |_| true,
                )
            },
        );
        let mut compute_server_urls = conn.as_ref().map_or_else(
            || vec![compute_server_url.clone()],
            |conn| {
                read_string_or_default(conn, KEY_COMPUTE_SERVER_URLS, &compute_server_url, |_| true)
                    .lines()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            },
        );
        if compute_server_urls.is_empty() {
            compute_server_urls.push(if compute_server_url.is_empty() {
                DEFAULT_COMPUTE_SERVER_URL.to_string()
            } else {
                compute_server_url.clone()
            });
        }
        if !compute_server_urls.contains(&compute_server_url) {
            compute_server_url.clone_from(&compute_server_urls[0]);
        }
        let last_stt_model = conn.as_ref().map_or_else(String::new, |conn| {
            read_string_or_default(conn, KEY_LAST_STT_MODEL, "", |_| true)
        });
        let last_tts_model = conn.as_ref().map_or_else(String::new, |conn| {
            read_string_or_default(conn, KEY_LAST_TTS_MODEL, "", |_| true)
        });
        let preview_padding_px = conn.as_ref().map_or(DEFAULT_PREVIEW_PADDING_PX, |conn| {
            read_u32_or_default(
                conn,
                KEY_PREVIEW_PADDING_PX,
                DEFAULT_PREVIEW_PADDING_PX,
                MAX_PREVIEW_PADDING_PX,
            )
        });
        let preview_shadow_size_px = conn
            .as_ref()
            .map_or(DEFAULT_PREVIEW_SHADOW_SIZE_PX, |conn| {
                read_u32_or_default(
                    conn,
                    KEY_PREVIEW_SHADOW_SIZE_PX,
                    DEFAULT_PREVIEW_SHADOW_SIZE_PX,
                    MAX_PREVIEW_SHADOW_SIZE_PX,
                )
            });
        let preview_upsample_method =
            conn.as_ref()
                .map_or(DEFAULT_PREVIEW_UPSAMPLE_METHOD, |conn| {
                    PreviewUpsampleMethod::from_key(&read_string_or_default(
                        conn,
                        KEY_PREVIEW_UPSAMPLE_METHOD,
                        DEFAULT_PREVIEW_UPSAMPLE_METHOD.key(),
                        |value| PreviewUpsampleMethod::from_key(value).is_some(),
                    ))
                    .expect("validated preview upsample method preference")
                });
        let preview_downsample_method =
            conn.as_ref()
                .map_or(DEFAULT_PREVIEW_DOWNSAMPLE_METHOD, |conn| {
                    PreviewDownsampleMethod::from_key(&read_string_or_default(
                        conn,
                        KEY_PREVIEW_DOWNSAMPLE_METHOD,
                        DEFAULT_PREVIEW_DOWNSAMPLE_METHOD.key(),
                        |value| PreviewDownsampleMethod::from_key(value).is_some(),
                    ))
                    .expect("validated preview downsample method preference")
                });
        let preview_zoom_pan_enabled = conn.as_ref().is_none_or(|conn| {
            read_string_or_default(conn, KEY_PREVIEW_ZOOM_PAN_ENABLED, "true", |value| {
                matches!(value, "true" | "false")
            }) == "true"
        });
        let preview_guides_visible = conn.as_ref().is_some_and(|conn| {
            read_string_or_default(
                conn,
                KEY_PREVIEW_GUIDES_VISIBLE,
                if DEFAULT_PREVIEW_GUIDES_VISIBLE {
                    "true"
                } else {
                    "false"
                },
                |value| matches!(value, "true" | "false"),
            ) == "true"
        });
        let temporal_decoder_pool_size =
            conn.as_ref()
                .map_or(DEFAULT_TEMPORAL_DECODER_POOL_SIZE, |conn| {
                    read_u32_in_range_or_default(
                        conn,
                        KEY_TEMPORAL_DECODER_POOL_SIZE,
                        DEFAULT_TEMPORAL_DECODER_POOL_SIZE,
                        MIN_TEMPORAL_DECODER_POOL_SIZE,
                        MAX_TEMPORAL_DECODER_POOL_SIZE,
                    )
                });
        let maximum_gpu_host_memory_gib = physical_system_memory_gib();
        let default_gpu_host_memory_gib = maximum_gpu_host_memory_gib / Fraction::from(2_u8);
        let gpu_host_memory_gib = conn.as_ref().map_or(default_gpu_host_memory_gib, |conn| {
            read_fraction_or_default(
                conn,
                KEY_GPU_HOST_MEMORY_GIB,
                default_gpu_host_memory_gib,
                maximum_gpu_host_memory_gib,
            )
        });
        let default_blender_binary = if cfg!(target_os = "macos") {
            "/Applications/Blender.app/Contents/MacOS/Blender"
        } else {
            ""
        };
        let blender_binary = conn.as_ref().map_or_else(
            || default_blender_binary.to_string(),
            |conn| {
                read_string_or_default(conn, KEY_BLENDER_BINARY, default_blender_binary, |_| true)
            },
        );
        let blender_binary = (!blender_binary.is_empty()).then(|| PathBuf::from(blender_binary));
        Self {
            caption_font_size,
            caption_background_color,
            timeline_magnet,
            timeline_beat_grid,
            timeline_snap_radius_px,
            timeline_cursor,
            timeline_drag_collision_mode,
            default_visual_duration,
            default_text_font_family,
            compute_server_url,
            compute_server_urls,
            last_stt_model,
            last_tts_model,
            preview_padding_px,
            preview_shadow_size_px,
            preview_upsample_method,
            preview_downsample_method,
            preview_zoom_pan_enabled,
            preview_guides_visible,
            temporal_decoder_pool_size,
            gpu_host_memory_gib,
            blender_binary,
            conn,
            listeners: Vec::new(),
        }
    }
}

pub fn open() -> SharedPreferences {
    let path = settings_db_path();
    let conn = match Connection::open(&path) {
        Ok(conn) => conn,
        Err(error) => {
            tracing::warn!(
                "Could not open preferences DB at {}: {error}",
                path.display()
            );
            return Rc::new(RefCell::new(PreferencesStore::new_with_conn(None)));
        }
    };
    if let Err(error) = conn.execute(
        "CREATE TABLE IF NOT EXISTS key_values (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    ) {
        tracing::warn!("Could not initialize preferences DB: {error}");
        return Rc::new(RefCell::new(PreferencesStore::new_with_conn(None)));
    }
    Rc::new(RefCell::new(PreferencesStore::new_with_conn(Some(conn))))
}

pub fn open_with_defaults() -> SharedPreferences {
    let path = settings_db_path();
    let directory = path
        .parent()
        .expect("settings database should have a parent directory");
    if let Err(error) = fs::create_dir_all(directory) {
        tracing::warn!(
            "Could not create preferences dir {}: {error}",
            directory.display()
        );
    }
    open()
}

pub fn snapshot(store: &SharedPreferences) -> PreferencesSnapshot {
    let state = store.borrow();
    snapshot_from_state(&state)
}

pub fn connect(store: &SharedPreferences, listener: impl Fn(PreferencesSnapshot) + 'static) {
    let mut store = store.borrow_mut();
    store.listeners.push(PreferenceListenerEntry {
        listener: Rc::new(listener),
    });
}

pub fn set_caption_font_size(store: &SharedPreferences, caption_font_size: f32) {
    let normalized = caption_font_size.clamp(MIN_CAPTION_FONT_SIZE, MAX_CAPTION_FONT_SIZE);
    let mut state = store.borrow_mut();
    if state.caption_font_size == normalized {
        return;
    }

    let previous = state.caption_font_size;
    state.caption_font_size = normalized;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_CAPTION_FONT_SIZE, &normalized.to_string())
    {
        tracing::warn!("Could not write caption font size preference: {error}");
        state.caption_font_size = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_caption_background_color(
    store: &SharedPreferences,
    caption_background_color: Color<u8>,
) {
    let mut state = store.borrow_mut();
    if state.caption_background_color == caption_background_color {
        return;
    }

    let previous = state.caption_background_color;
    state.caption_background_color = caption_background_color;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_CAPTION_BACKGROUND_COLOR,
            &caption_background_color.to_rgba_u32().to_string(),
        )
    {
        tracing::warn!("Could not write caption background color preference: {error}");
        state.caption_background_color = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_preview_padding_px(store: &SharedPreferences, padding_px: u32) {
    let padding_px = padding_px.min(MAX_PREVIEW_PADDING_PX);
    let mut state = store.borrow_mut();
    if state.preview_padding_px == padding_px {
        return;
    }

    let previous = state.preview_padding_px;
    state.preview_padding_px = padding_px;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_PREVIEW_PADDING_PX, &padding_px.to_string())
    {
        tracing::warn!("Could not write preview padding preference: {error}");
        state.preview_padding_px = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_preview_shadow_size_px(store: &SharedPreferences, shadow_size_px: u32) {
    let shadow_size_px = shadow_size_px.min(MAX_PREVIEW_SHADOW_SIZE_PX);
    let mut state = store.borrow_mut();
    if state.preview_shadow_size_px == shadow_size_px {
        return;
    }

    let previous = state.preview_shadow_size_px;
    state.preview_shadow_size_px = shadow_size_px;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_PREVIEW_SHADOW_SIZE_PX,
            &shadow_size_px.to_string(),
        )
    {
        tracing::warn!("Could not write preview shadow size preference: {error}");
        state.preview_shadow_size_px = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_preview_upsample_method(store: &SharedPreferences, method: PreviewUpsampleMethod) {
    let mut state = store.borrow_mut();
    if state.preview_upsample_method == method {
        return;
    }

    let previous = state.preview_upsample_method;
    state.preview_upsample_method = method;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_PREVIEW_UPSAMPLE_METHOD, method.key())
    {
        tracing::warn!("Could not write preview upsample method preference: {error}");
        state.preview_upsample_method = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_preview_downsample_method(store: &SharedPreferences, method: PreviewDownsampleMethod) {
    let mut state = store.borrow_mut();
    if state.preview_downsample_method == method {
        return;
    }

    let previous = state.preview_downsample_method;
    state.preview_downsample_method = method;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_PREVIEW_DOWNSAMPLE_METHOD, method.key())
    {
        tracing::warn!("Could not write preview downsample method preference: {error}");
        state.preview_downsample_method = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_preview_guides_visible(store: &SharedPreferences, visible: bool) {
    let mut state = store.borrow_mut();
    if state.preview_guides_visible == visible {
        return;
    }

    let previous = state.preview_guides_visible;
    state.preview_guides_visible = visible;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_PREVIEW_GUIDES_VISIBLE,
            if visible { "true" } else { "false" },
        )
    {
        tracing::warn!("Could not write preview guide visibility preference: {error}");
        state.preview_guides_visible = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_temporal_decoder_pool_size(store: &SharedPreferences, size: u32) {
    let size = size.clamp(
        MIN_TEMPORAL_DECODER_POOL_SIZE,
        MAX_TEMPORAL_DECODER_POOL_SIZE,
    );
    let mut state = store.borrow_mut();
    if state.temporal_decoder_pool_size == size {
        return;
    }

    let previous = state.temporal_decoder_pool_size;
    state.temporal_decoder_pool_size = size;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TEMPORAL_DECODER_POOL_SIZE, &size.to_string())
    {
        tracing::warn!("Could not write temporal decoder pool size preference: {error}");
        state.temporal_decoder_pool_size = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_gpu_host_memory_gib(store: &SharedPreferences, size_gib: Fraction) {
    let size_gib = clamp_gpu_host_memory_gib(size_gib);
    let mut state = store.borrow_mut();
    if state.gpu_host_memory_gib == size_gib {
        return;
    }

    let previous = state.gpu_host_memory_gib;
    state.gpu_host_memory_gib = size_gib;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_GPU_HOST_MEMORY_GIB, &size_gib.to_string())
    {
        tracing::warn!("Could not write GPU host memory budget preference: {error}");
        state.gpu_host_memory_gib = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_blender_binary(store: &SharedPreferences, binary: Option<&Path>) {
    let binary = binary.map(Path::to_path_buf);
    let mut state = store.borrow_mut();
    if state.blender_binary == binary {
        return;
    }
    let previous = std::mem::replace(&mut state.blender_binary, binary);
    let value = state
        .blender_binary
        .as_deref()
        .map_or_else(String::new, |path| path.to_string_lossy().into_owned());
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_BLENDER_BINARY, &value)
    {
        tracing::warn!("Could not write Blender binary preference: {error}");
        state.blender_binary = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_timeline_magnet(store: &SharedPreferences, enabled: bool) {
    let value = if enabled { "true" } else { "false" };
    let mut state = store.borrow_mut();
    if state.timeline_magnet == value {
        return;
    }

    let previous = state.timeline_magnet.clone();
    state.timeline_magnet = value.to_string();
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TIMELINE_MAGNET, value)
    {
        tracing::warn!("Could not write timeline magnet preference: {error}");
        state.timeline_magnet = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_timeline_beat_grid(store: &SharedPreferences, enabled: bool) {
    let value = if enabled { "true" } else { "false" };
    let mut state = store.borrow_mut();
    if state.timeline_beat_grid == value {
        return;
    }

    let previous = state.timeline_beat_grid.clone();
    state.timeline_beat_grid = value.to_string();
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TIMELINE_BEAT_GRID, value)
    {
        tracing::warn!("Could not write timeline beat grid preference: {error}");
        state.timeline_beat_grid = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_timeline_snap_radius_px(store: &SharedPreferences, radius_px: u32) {
    let radius_px = radius_px.clamp(MIN_TIMELINE_SNAP_RADIUS_PX, MAX_TIMELINE_SNAP_RADIUS_PX);
    let mut state = store.borrow_mut();
    if state.timeline_snap_radius_px == radius_px {
        return;
    }

    let previous = state.timeline_snap_radius_px;
    state.timeline_snap_radius_px = radius_px;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TIMELINE_SNAP_RADIUS_PX, &radius_px.to_string())
    {
        tracing::warn!("Could not write timeline snap radius preference: {error}");
        state.timeline_snap_radius_px = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_timeline_cursor(store: &SharedPreferences, cursor: &str) {
    if !matches!(cursor, "pointer" | "cut") {
        return;
    }
    let mut state = store.borrow_mut();
    if state.timeline_cursor == cursor {
        return;
    }

    let previous = state.timeline_cursor.clone();
    state.timeline_cursor = cursor.to_string();
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TIMELINE_CURSOR, cursor)
    {
        tracing::warn!("Could not write timeline cursor preference: {error}");
        state.timeline_cursor = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_timeline_drag_collision_mode(store: &SharedPreferences, mode: &str) {
    if !matches!(mode, "overwrite" | "block" | "new_track") {
        return;
    }
    let mut state = store.borrow_mut();
    if state.timeline_drag_collision_mode == mode {
        return;
    }

    let previous = state.timeline_drag_collision_mode.clone();
    state.timeline_drag_collision_mode = mode.to_string();
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_TIMELINE_DRAG_COLLISION_MODE, mode)
    {
        tracing::warn!("Could not write timeline drag collision preference: {error}");
        state.timeline_drag_collision_mode = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_default_visual_duration(store: &SharedPreferences, duration: Time) {
    let normalized = duration.clamp(MIN_VISUAL_DURATION, MAX_VISUAL_DURATION);
    let mut state = store.borrow_mut();
    if state.default_visual_duration == normalized {
        return;
    }

    let previous = state.default_visual_duration;
    state.default_visual_duration = normalized;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_DEFAULT_VISUAL_DURATION_SECONDS,
            &normalized.as_secs_f64().to_string(),
        )
    {
        tracing::warn!("Could not write default visual duration preference: {error}");
        state.default_visual_duration = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_default_text_font_family(store: &SharedPreferences, family: FontFamily) {
    if family.name().trim().is_empty() {
        return;
    }
    let mut state = store.borrow_mut();
    if state.default_text_font_family == family {
        return;
    }

    let previous = std::mem::replace(&mut state.default_text_font_family, family);
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_DEFAULT_TEXT_FONT_FAMILY,
            &encode_font_family(&state.default_text_font_family),
        )
    {
        tracing::warn!("Could not write default text font preference: {error}");
        state.default_text_font_family = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_compute_server_url(store: &SharedPreferences, url: &str) {
    let normalized = url.trim();
    let mut state = store.borrow_mut();
    if state.compute_server_url == normalized {
        return;
    }

    let previous = std::mem::replace(&mut state.compute_server_url, normalized.to_string());
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_COMPUTE_SERVER_URL, normalized)
    {
        tracing::warn!("Could not write compute server URL preference: {error}");
        state.compute_server_url = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_compute_server_urls(store: &SharedPreferences, urls: &[String]) {
    let mut normalized = Vec::new();
    for url in urls
        .iter()
        .map(|url| url.trim())
        .filter(|url| !url.is_empty())
    {
        if !normalized.iter().any(|saved| saved == url) {
            normalized.push(url.to_string());
        }
    }
    let mut state = store.borrow_mut();
    if normalized.is_empty() {
        normalized.push(if state.compute_server_url.is_empty() {
            DEFAULT_COMPUTE_SERVER_URL.to_string()
        } else {
            state.compute_server_url.clone()
        });
    }
    if state.compute_server_urls == normalized {
        return;
    }

    let previous = std::mem::replace(&mut state.compute_server_urls, normalized);
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_COMPUTE_SERVER_URLS,
            &state.compute_server_urls.join("\n"),
        )
    {
        tracing::warn!("Could not write compute server list preference: {error}");
        state.compute_server_urls = previous;
        return;
    }
    notify_listeners(state);
}

pub fn normalize_server_url(value: &str) -> Result<String, &'static str> {
    let value = value.trim().trim_end_matches('/');
    let Some(rest) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
    else {
        return Err("Use an http:// or https:// server URL");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() {
        return Err("Server URL must include a host");
    }
    if host.starts_with(':') || value.chars().any(char::is_whitespace) {
        return Err("Enter a valid server URL");
    }
    Ok(value.to_string())
}

pub fn validate_blender_binary(path: &Path) -> Result<PathBuf, String> {
    let path = shrimply_blender_core::canonical_binary(path)?;
    shrimply_blender_core::probe(&path)?;
    Ok(path)
}

pub fn apply_blender_binary(store: &SharedPreferences, path: Option<PathBuf>) {
    set_blender_binary(store, path.as_deref());
    shrimply_blender_core::set_binary(snapshot(store).blender_binary);
}

pub use shrimply_compute_client::ServerStatus;

pub fn compute_server_status(url: &str) -> Result<ServerStatus, String> {
    shrimply_compute_client::server_status(url)
}

pub fn select_compute_device(url: &str, device: &str) -> Result<ServerStatus, String> {
    shrimply_compute_client::set_compute_device(url, device)
}

pub fn set_last_stt_model(store: &SharedPreferences, model: &str) {
    let mut state = store.borrow_mut();
    if state.last_stt_model == model {
        return;
    }

    let previous = std::mem::replace(&mut state.last_stt_model, model.to_string());
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_LAST_STT_MODEL, model)
    {
        tracing::warn!("Could not write speech-to-text model preference: {error}");
        state.last_stt_model = previous;
        return;
    }
    notify_listeners(state);
}

pub fn set_last_tts_model(store: &SharedPreferences, model: &str) {
    let mut state = store.borrow_mut();
    if state.last_tts_model == model {
        return;
    }

    let previous = std::mem::replace(&mut state.last_tts_model, model.to_string());
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(conn, KEY_LAST_TTS_MODEL, model)
    {
        tracing::warn!("Could not write text-to-speech model preference: {error}");
        state.last_tts_model = previous;
        return;
    }
    notify_listeners(state);
}

pub fn preview_image_folder(store: &SharedPreferences) -> Option<std::path::PathBuf> {
    let state = store.borrow();
    let conn = state.conn.as_ref()?;
    conn.query_row(
        "SELECT CAST(value AS TEXT) FROM key_values WHERE key = ?1",
        [KEY_PREVIEW_IMAGE_FOLDER],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(std::path::PathBuf::from)
    .filter(|path| path.is_dir())
}

pub fn set_preview_image_folder(store: &SharedPreferences, path: &std::path::Path) {
    let state = store.borrow();
    let Some(conn) = &state.conn else {
        return;
    };
    if let Err(error) = write_string(conn, KEY_PREVIEW_IMAGE_FOLDER, &path.to_string_lossy()) {
        tracing::warn!("Could not write preview image folder preference: {error}");
    }
}

fn notify_listeners(state: std::cell::RefMut<'_, PreferencesStore>) {
    let snapshot = snapshot_from_state(&state);
    let listeners = state.listeners.clone();
    drop(state);
    for listener in listeners {
        (listener.listener)(snapshot.clone());
    }
}

fn snapshot_from_state(state: &PreferencesStore) -> PreferencesSnapshot {
    PreferencesSnapshot {
        caption_font_size: state.caption_font_size,
        caption_background_color: state.caption_background_color,
        timeline_magnet: state.timeline_magnet.clone(),
        timeline_beat_grid: state.timeline_beat_grid.clone(),
        timeline_snap_radius_px: state.timeline_snap_radius_px,
        timeline_cursor: state.timeline_cursor.clone(),
        timeline_drag_collision_mode: state.timeline_drag_collision_mode.clone(),
        default_visual_duration: state.default_visual_duration,
        default_text_font_family: state.default_text_font_family.clone(),
        compute_server_url: state.compute_server_url.clone(),
        compute_server_urls: state.compute_server_urls.clone(),
        last_stt_model: state.last_stt_model.clone(),
        last_tts_model: state.last_tts_model.clone(),
        preview_padding_px: state.preview_padding_px,
        preview_shadow_size_px: state.preview_shadow_size_px,
        preview_upsample_method: state.preview_upsample_method,
        preview_downsample_method: state.preview_downsample_method,
        preview_zoom_pan_enabled: state.preview_zoom_pan_enabled,
        preview_guides_visible: state.preview_guides_visible,
        temporal_decoder_pool_size: state.temporal_decoder_pool_size,
        gpu_host_memory_gib: state.gpu_host_memory_gib,
        blender_binary: state.blender_binary.clone(),
    }
}

fn default_text_font_family() -> FontFamily {
    FontFamily::GoogleFonts {
        name: DEFAULT_TEXT_FONT_FAMILY.to_string(),
    }
}

fn encode_font_family(family: &FontFamily) -> String {
    match family {
        FontFamily::Local { name } => format!("local\t{name}"),
        FontFamily::GoogleFonts { name } => format!("google_fonts\t{name}"),
    }
}

fn decode_font_family(value: &str) -> Option<FontFamily> {
    let (source, name) = value.split_once('\t')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    match source {
        "local" => Some(FontFamily::Local {
            name: name.to_string(),
        }),
        "google_fonts" => Some(FontFamily::GoogleFonts {
            name: name.to_string(),
        }),
        _ => None,
    }
}

fn read_f32_or_default(conn: &Connection, key: &str, default: f32, min: f32, max: f32) -> f32 {
    read_string_or_default(conn, key, &default.to_string(), |value| {
        value
            .parse::<f32>()
            .is_ok_and(|value| (min..=max).contains(&value))
    })
    .parse()
    .unwrap_or(default)
}

fn read_u32_or_default(conn: &Connection, key: &str, default: u32, max: u32) -> u32 {
    parse_u32(&read_string_or_default(
        conn,
        key,
        &default.to_string(),
        |value| parse_u32(value).is_some_and(|value| value <= max),
    ))
    .unwrap_or(default)
}

fn read_u32_in_range_or_default(
    conn: &Connection,
    key: &str,
    default: u32,
    min: u32,
    max: u32,
) -> u32 {
    parse_u32(&read_string_or_default(
        conn,
        key,
        &default.to_string(),
        |value| parse_u32(value).is_some_and(|value| (min..=max).contains(&value)),
    ))
    .unwrap_or(default)
}

fn read_fraction_or_default(
    conn: &Connection,
    key: &str,
    default: Fraction,
    maximum: Fraction,
) -> Fraction {
    Fraction::from_str(&read_string_or_default(
        conn,
        key,
        &default.to_string(),
        |value| {
            Fraction::from_str(value).is_ok_and(|value| clamp_fraction(value, maximum) == value)
        },
    ))
    .unwrap_or(default)
}

fn clamp_gpu_host_memory_gib(value: Fraction) -> Fraction {
    clamp_fraction(value, physical_system_memory_gib())
}

fn clamp_fraction(value: Fraction, maximum: Fraction) -> Fraction {
    let zero = Fraction::from(0_u8);
    if value < zero {
        zero
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

pub fn physical_system_memory_gib() -> Fraction {
    #[cfg(unix)]
    let bytes = {
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let page_bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let pages = u64::try_from(pages)
            .expect("detect physical system RAM: sysconf(_SC_PHYS_PAGES) failed");
        let page_bytes = u64::try_from(page_bytes)
            .expect("detect physical system RAM: sysconf(_SC_PAGESIZE) failed");
        pages
            .checked_mul(page_bytes)
            .expect("detect physical system RAM: byte count overflowed")
    };
    #[cfg(windows)]
    let bytes = {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>())
                .expect("memory status structure size fits u32"),
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut memory) }.expect("detect physical system RAM");
        memory.ullTotalPhys
    };
    Fraction::new_raw(bytes, GIB_BYTES)
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse().ok().or_else(|| {
        let value = value.parse::<f64>().ok()?;
        (value.is_finite() && value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value))
            .then_some(value as u32)
    })
}

fn read_string_or_default(
    conn: &Connection,
    key: &str,
    default: &str,
    valid: impl Fn(&str) -> bool,
) -> String {
    match conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM key_values WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
    {
        Ok(Some(value)) if valid(&value) => value,
        Ok(_) => {
            if let Err(error) = write_string(conn, key, default) {
                tracing::warn!("Could not write default preference {key}: {error}");
            }
            default.to_string()
        }
        Err(error) => {
            tracing::warn!("Could not read preference {key}: {error}");
            if let Err(error) = write_string(conn, key, default) {
                tracing::warn!("Could not write default preference {key}: {error}");
            }
            default.to_string()
        }
    }
}

fn write_string(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO key_values (key, value) VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![key, value],
    )
}
pub fn set_preview_zoom_pan_enabled(store: &SharedPreferences, visible: bool) {
    let mut state = store.borrow_mut();
    if state.preview_zoom_pan_enabled == visible {
        return;
    }

    let previous = state.preview_zoom_pan_enabled;
    state.preview_zoom_pan_enabled = visible;
    if let Some(conn) = &state.conn
        && let Err(error) = write_string(
            conn,
            KEY_PREVIEW_ZOOM_PAN_ENABLED,
            if visible { "true" } else { "false" },
        )
    {
        tracing::warn!("Could not write preview zoom and pan preference: {error}");
        state.preview_zoom_pan_enabled = previous;
        return;
    }
    notify_listeners(state);
}
