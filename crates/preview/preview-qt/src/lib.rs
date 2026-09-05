mod preview;
mod renderer;
mod system_cursor;

pub use preview::ToolkitPreview;

use shrimply_audio::{AudioPlayer, SharedAudioLevels};
use shrimply_playback_performance as playback_performance;
use shrimply_preview_core::PreviewViewport;
use shrimply_preview_runtime::captions::{CaptionAppearance, draw_captions};
use shrimply_preview_runtime::guides;
use shrimply_preview_runtime::preferences::store as preferences_store;
use shrimply_preview_runtime::renderer::{Appearance, VideoRenderer};
use shrimply_preview_runtime::{PreviewMedia, StepDirection, rendered_frame_rate_label};
use shrimply_project::project::{Project, Time};
use shrimply_skia_adw_core::canvas::UVec2;
use shrimply_skia_adw_core::canvas::{Rect, vec2};
use shrimply_skia_gl::GlAudioMeter;
use shrimply_state::player_state::{self, SharedPlayerState};
use shrimply_video_cuda::compositor::{
    CompositeAccuracy, VideoCommand, VideoCommandSender, VideoEvent,
};
use shrimply_video_cuda::gpu::CompositedVideoFrame;
use std::cell::RefCell;
use std::rc::Rc;

use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_math_color::Color;
use shrimply_timeline_gtk::{RenderedVideoFrame, ToolkitPointerButton, ToolkitTimeline};
use shrimply_timeline_qt::{
    ContextMenuControl, ContextMenuRequest, CursorTool, DragCollisionMode,
    TIMELINE_CLIPBOARD_MARKER,
};
use std::ffi::c_void;
use std::path::PathBuf;

struct Surfaces {
    timeline: ToolkitTimeline,
    timeline_menu: shrimply_timeline_qt::MenuModel,
    track_add_menu: shrimply_timeline_qt::TrackAddMenuModel,
    track_add_kind: shrimply_timeline::TrackKind,
    track_add_x: f32,
    track_add_y: f32,
    track_add_pending: bool,
    timeline_error_pending: bool,
    context_frame: Option<RenderedVideoFrame>,
    context_open_path: String,
    context_delete_clip_count: usize,
    context_action_error: String,
    preview: ToolkitPreview,
    audio_meter: GlAudioMeter,
    audio_levels: SharedAudioLevels,
}

thread_local! {
    static SURFACES: RefCell<Option<Surfaces>> = const { RefCell::new(None) };
}

pub fn install(session: &EditorSession) -> Result<(), String> {
    let timeline = ToolkitTimeline::new(
        session.project.clone(),
        session.player_state.clone(),
        session.playback_performance.clone(),
        session.selection_state.clone(),
        session.preferences.clone(),
        session.property_clipboard.clone(),
    );
    let preview = ToolkitPreview::new(
        session.project.clone(),
        session.player_state.clone(),
        session.selection_state.clone(),
        session.preview_focus.clone(),
        session.playback_performance.clone(),
        session.preferences.clone(),
        session.audio_player.clone(),
    )?;
    let audio_meter = GlAudioMeter::default();
    SURFACES.with_borrow_mut(|surfaces| {
        assert!(
            surfaces.is_none(),
            "Qt editor surfaces are already installed"
        );
        *surfaces = Some(Surfaces {
            timeline,
            timeline_menu: shrimply_timeline_qt::MenuModel::default(),
            track_add_menu: shrimply_timeline_qt::TrackAddMenuModel::default(),
            track_add_kind: shrimply_timeline::TrackKind::Video,
            track_add_x: 0.0,
            track_add_y: 0.0,
            track_add_pending: false,
            timeline_error_pending: false,
            context_frame: None,
            context_open_path: String::new(),
            context_delete_clip_count: 0,
            context_action_error: String::new(),
            preview,
            audio_meter,
            audio_levels: session.audio_levels.clone(),
        });
    });
    tracing::info!(thread = ?std::thread::current().id(), "installed Qt GPU surfaces");
    Ok(())
}

fn missing(surface: &str) -> bool {
    tracing::error!(
        surface,
        thread = ?std::thread::current().id(),
        "Qt GPU surface render requested before the shared editor lifecycle installed it"
    );
    false
}

fn render(result: Result<(), String>, surface: &str) -> bool {
    if let Err(error) = result {
        tracing::error!(%error, surface, "Qt GPU surface render failed");
        false
    } else {
        true
    }
}

#[repr(C)]
struct PlatformColor {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

#[repr(C)]
struct PlatformPalette {
    window_bg: PlatformColor,
    window_fg: PlatformColor,
    view_bg: PlatformColor,
    view_fg: PlatformColor,
    alternate_bg: PlatformColor,
    button_bg: PlatformColor,
    button_fg: PlatformColor,
    border: PlatformColor,
    accent_bg: PlatformColor,
    accent_fg: PlatformColor,
}

const PLATFORM_PALETTE_COLOR_COUNT: usize = 10;
const _: () = assert!(
    std::mem::size_of::<PlatformPalette>()
        == std::mem::size_of::<PlatformColor>() * PLATFORM_PALETTE_COLOR_COUNT
);

impl PlatformColor {
    fn color(&self) -> Color {
        Color::new(self.red, self.green, self.blue, self.alpha)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn shrimply_qt_set_platform_palette(palette: *const PlatformPalette) {
    let palette = unsafe {
        palette
            .as_ref()
            .expect("Qt must provide its platform palette")
    };
    shrimply_cross_ui_theme::set_platform_palette(Some(shrimply_cross_ui_theme::PlatformPalette {
        window_bg: palette.window_bg.color(),
        window_fg: palette.window_fg.color(),
        view_bg: palette.view_bg.color(),
        view_fg: palette.view_fg.color(),
        alternate_bg: palette.alternate_bg.color(),
        button_bg: palette.button_bg.color(),
        button_fg: palette.button_fg.color(),
        border: palette.border.color(),
        accent_bg: palette.accent_bg.color(),
        accent_fg: palette.accent_fg.color(),
    }));
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_render_timeline(
    width: u32,
    height: u32,
    scale: f32,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    dark: bool,
) -> bool {
    shrimply_cross_ui_theme::set_dark(dark);
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return missing("timeline");
        };
        let result =
            surfaces
                .timeline
                .render(width, height, scale, Color::new(red, green, blue, alpha));
        if result.is_ok()
            && let Some(presentation) = surfaces.timeline.take_track_add_menu()
        {
            surfaces.track_add_menu =
                shrimply_timeline_qt::TrackAddMenuModel::new(presentation.kind);
            surfaces.track_add_kind = presentation.kind;
            surfaces.track_add_x = presentation.x;
            surfaces.track_add_y = presentation.y;
            surfaces.track_add_pending = true;
        }
        if let Some(error) = surfaces.timeline.take_track_import_error() {
            surfaces.context_action_error = error;
            surfaces.timeline_error_pending = true;
        }
        render(result, "timeline")
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_take_error() -> bool {
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return false;
        };
        std::mem::take(&mut surfaces.timeline_error_pending)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_take_track_add_menu() -> bool {
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return false;
        };
        std::mem::take(&mut surfaces.track_add_pending)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_track_add_menu_x() -> f32 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0.0, |surfaces| surfaces.track_add_x)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_track_add_menu_y() -> f32 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0.0, |surfaces| surfaces.track_add_y)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_track_add_menu_count() -> usize {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0, |surfaces| surfaces.track_add_menu.entries().len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_track_add_menu_kind(index: usize) -> u8 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.track_add_menu.entries().get(index))
            .map_or(0, |entry| match entry {
                shrimply_timeline_qt::TrackAddMenuEntry::Action(_) => 1,
                shrimply_timeline_qt::TrackAddMenuEntry::Separator => 2,
            })
    })
}

fn write_track_add_text(index: usize, output: *mut u8, capacity: usize, icon: bool) -> usize {
    let text = SURFACES.with_borrow(|surfaces| {
        let Some(surfaces) = surfaces.as_ref() else {
            return "";
        };
        match surfaces.track_add_menu.entries().get(index) {
            Some(shrimply_timeline_qt::TrackAddMenuEntry::Action(action)) if icon => action.icon(),
            Some(shrimply_timeline_qt::TrackAddMenuEntry::Action(action)) => {
                action.label(surfaces.track_add_kind)
            }
            Some(shrimply_timeline_qt::TrackAddMenuEntry::Separator) | None => "",
        }
    });
    let bytes = text.as_bytes();
    if capacity > 0 {
        let length = bytes.len().min(capacity - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length);
            output.add(length).write(0);
        }
    }
    bytes.len()
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `capacity` is nonzero, `output` must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_track_add_menu_label(
    index: usize,
    output: *mut u8,
    capacity: usize,
) -> usize {
    write_track_add_text(index, output, capacity, false)
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `capacity` is nonzero, `output` must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_track_add_menu_icon(
    index: usize,
    output: *mut u8,
    capacity: usize,
) -> usize {
    write_track_add_text(index, output, capacity, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_activate_track_add_menu_item(index: usize) -> bool {
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return false;
        };
        let Some(action) = surfaces.track_add_menu.action(index) else {
            return false;
        };
        surfaces.timeline.activate_track_add_action(action)
            && action == shrimply_timeline_qt::TrackAddAction::Import
    })
}

#[unsafe(no_mangle)]
/// # Safety
///
/// `path` must point to `length` readable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_import_track_file(
    path: *const u8,
    length: usize,
) -> bool {
    let path = unsafe { std::slice::from_raw_parts(path, length) };
    let path = String::from_utf8_lossy(path);
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return false;
        };
        match surfaces
            .timeline
            .import_track_file(PathBuf::from(path.as_ref()))
        {
            Ok(()) => true,
            Err(error) => {
                surfaces.context_action_error = error;
                false
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_render_preview(
    width: u32,
    height: u32,
    scale: f32,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    dark: bool,
    fullscreen: bool,
) -> bool {
    shrimply_cross_ui_theme::set_dark(dark);
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return missing("preview");
        };
        render(
            surfaces.preview.render(
                width,
                height,
                scale,
                Color::new(red, green, blue, alpha),
                fullscreen,
            ),
            "preview",
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_render_audio_meter(
    width: u32,
    height: u32,
    scale: f32,
    dark: bool,
) -> bool {
    shrimply_cross_ui_theme::set_dark(dark);
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return missing("audio meter");
        };
        render(
            surfaces.audio_meter.render(
                surfaces.audio_levels.take_peaks(),
                UVec2::new(width, height),
                scale,
            ),
            "audio meter",
        )
    })
}

pub fn mark_preview_step(delta: i32) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.preview.mark_step(delta);
        }
    });
}

pub fn preview_frame_rate_label() -> String {
    SURFACES.with_borrow(|surfaces| {
        surfaces.as_ref().map_or_else(
            || String::from("--"),
            |surfaces| surfaces.preview.frame_rate_label().into(),
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_pointer_move(x: f32, y: f32, control: bool, shift: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.pointer_move(x, y, control, shift);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_pointer_cursor() -> u8 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0, |surfaces| surfaces.timeline.pointer_cursor() as u8)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_pointer_leave() {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.pointer_leave();
        }
    });
}

fn pointer_button(button: u8) -> ToolkitPointerButton {
    match button {
        0 => ToolkitPointerButton::Primary,
        1 => ToolkitPointerButton::Middle,
        _ => panic!("unsupported Qt timeline pointer button {button}"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_pointer_press(
    button: u8,
    x: f32,
    y: f32,
    control: bool,
    shift: bool,
) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces
                .timeline
                .pointer_press(pointer_button(button), x, y, control, shift);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_pointer_release(
    button: u8,
    x: f32,
    y: f32,
    control: bool,
    shift: bool,
) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces
                .timeline
                .pointer_release(pointer_button(button), x, y, control, shift);
        }
    });
}

#[unsafe(no_mangle)]
/// # Safety
///
/// The pointers must be valid Wayland display, surface, and seat handles for the duration of the
/// call.
pub unsafe extern "C" fn shrimply_qt_timeline_begin_pointer_lock(
    display: *mut c_void,
    surface: *mut c_void,
    seat: *mut c_void,
) -> bool {
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return false;
        };
        unsafe {
            surfaces
                .timeline
                .begin_pointer_lock(display, surface, seat, system_cursor::grabbing())
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_end_pointer_lock(control: bool, shift: bool) {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.timeline.end_pointer_lock(control, shift);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_scroll(dx: f32, dy: f32, control: bool, shift: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.scroll(dx, dy, control, shift);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_prepare_context_menu(x: f32, y: f32) -> usize {
    let entries = SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return Vec::new();
        };
        surfaces.timeline.prepare_context_menu(x, y);
        surfaces.timeline_menu =
            shrimply_timeline_qt::MenuModel::new(surfaces.timeline.context_menu());
        surfaces.timeline_menu.entries().to_vec()
    });
    tracing::debug!(
        x,
        y,
        count = entries.len(),
        ?entries,
        "prepared Qt timeline context menu"
    );
    entries.len()
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `capacity` is nonzero, `output` must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_context_menu_label(
    index: usize,
    output: *mut u8,
    capacity: usize,
) -> usize {
    let label = SURFACES.with_borrow(|surfaces| {
        surfaces.as_ref().map_or("", |surfaces| {
            match surfaces.timeline_menu.entries().get(index) {
                Some(shrimply_timeline_qt::MenuEntry::Action(item)) => item.label(),
                Some(shrimply_timeline_qt::MenuEntry::Control(control)) => control.label(),
                Some(shrimply_timeline_qt::MenuEntry::Separator) | None => "",
            }
        })
    });
    let bytes = label.as_bytes();
    if capacity > 0 {
        let length = bytes.len().min(capacity - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length);
            output.add(length).write(0);
        }
    }
    bytes.len()
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_count() -> usize {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0, |surfaces| surfaces.timeline_menu.entries().len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_kind(index: usize) -> u8 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.entries().get(index))
            .map_or(0, |entry| match entry {
                shrimply_timeline_qt::MenuEntry::Action(_) => 1,
                shrimply_timeline_qt::MenuEntry::Separator => 2,
                shrimply_timeline_qt::MenuEntry::Control(ContextMenuControl::PlaybackSpeed {
                    ..
                }) => 3,
                shrimply_timeline_qt::MenuEntry::Control(ContextMenuControl::AudioTrackGain {
                    ..
                }) => 4,
            })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_enabled(index: usize) -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.entries().get(index))
            .is_some_and(|entry| match entry {
                shrimply_timeline_qt::MenuEntry::Action(item) => item.enabled,
                _ => true,
            })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_value(index: usize) -> f64 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.control(index))
            .map_or(0.0, ContextMenuControl::value)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_minimum(index: usize) -> f64 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.control(index))
            .map_or(0.0, ContextMenuControl::minimum)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_maximum(index: usize) -> f64 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.control(index))
            .map_or(0.0, ContextMenuControl::maximum)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_step(index: usize) -> f64 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.control(index))
            .map_or(0.0, ContextMenuControl::step)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_menu_mixed(index: usize) -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.timeline_menu.control(index))
            .is_some_and(ContextMenuControl::mixed)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_set_context_menu_control(index: usize, value: f64) {
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return;
        };
        if let Some(control) = surfaces.timeline_menu.control(index) {
            surfaces.timeline.set_context_menu_control(control, value);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_activate_context_menu_item(index: usize) -> u8 {
    tracing::debug!(index, "activating Qt timeline context menu item");
    SURFACES.with_borrow_mut(|surfaces| {
        let Some(surfaces) = surfaces.as_mut() else {
            return 0;
        };
        surfaces.context_frame = None;
        surfaces.context_open_path.clear();
        surfaces.context_delete_clip_count = 0;
        surfaces.context_action_error.clear();
        let request = surfaces
            .timeline_menu
            .action(index)
            .and_then(|action| surfaces.timeline.activate_context_menu_action(action));
        surfaces.timeline_menu = shrimply_timeline_qt::MenuModel::default();
        let (selection, result_code) = match request {
            None => return 0,
            Some(ContextMenuRequest::CopyFrame(selection)) => (selection, 1),
            Some(ContextMenuRequest::SaveFrame(selection)) => (selection, 2),
            Some(ContextMenuRequest::ShowInFolder) => {
                let Some(path) = surfaces.timeline.context_file_path() else {
                    surfaces.context_action_error = "The selected item has no file.".to_string();
                    return 3;
                };
                match shrimply_cross_ui_core::desktop_open::prepare(path, None) {
                    Ok(
                        shrimply_cross_ui_core::desktop_open::Action::Open(path)
                        | shrimply_cross_ui_core::desktop_open::Action::FocusRevealed(path),
                    ) => {
                        surfaces.context_open_path = path.to_string_lossy().into_owned();
                        return 4;
                    }
                    Err(error) => {
                        surfaces.context_action_error = error;
                        return 3;
                    }
                }
            }
            Some(ContextMenuRequest::DeleteFoldedTrack { clip_count }) => {
                surfaces.context_delete_clip_count = clip_count;
                return 5;
            }
            Some(ContextMenuRequest::SetTimelineClipboardMarker) => return 6,
            Some(ContextMenuRequest::PasteFromClipboard) => return 7,
            Some(request) => {
                surfaces.context_action_error =
                    format!("{request:?} is not implemented by the Qt timeline adapter");
                return 3;
            }
        };
        match surfaces.timeline.render_context_video_frame(selection) {
            Ok(frame) => {
                surfaces.context_frame = Some(frame);
                result_code
            }
            Err(error) => {
                surfaces.context_action_error = error;
                3
            }
        }
    })
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `output` is non-null and `capacity` is nonzero, it must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_clipboard_marker(
    output: *mut u8,
    capacity: usize,
) -> usize {
    let bytes = TIMELINE_CLIPBOARD_MARKER.as_bytes();
    if !output.is_null() && capacity > 0 {
        let length = bytes.len().min(capacity - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length);
            output.add(length).write(0);
        }
    }
    bytes.len()
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `text` is non-null, it must point to `length` readable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_paste_clipboard_text(text: *const u8, length: usize) {
    if text.is_null() {
        return;
    }
    let text = unsafe { std::slice::from_raw_parts(text, length) };
    if let Ok(text) = std::str::from_utf8(text) {
        SURFACES.with_borrow(|surfaces| {
            if let Some(surfaces) = surfaces.as_ref() {
                surfaces
                    .timeline
                    .paste_context_clipboard_text(text.to_owned());
            }
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_delete_clip_count() -> usize {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0, |surfaces| surfaces.context_delete_clip_count)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_delete_context_folded_track() {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.timeline.delete_context_folded_track();
            surfaces.context_delete_clip_count = 0;
        }
    });
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `output` is non-null, it must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_context_open_path(
    output: *mut u8,
    capacity: usize,
) -> usize {
    SURFACES.with_borrow(|surfaces| {
        let value = surfaces
            .as_ref()
            .map_or(&[][..], |surfaces| surfaces.context_open_path.as_bytes());
        if !output.is_null() && capacity > value.len() {
            unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), output, value.len()) };
            unsafe { *output.add(value.len()) = 0 };
        }
        value.len()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_frame_width() -> i32 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.context_frame.as_ref())
            .map_or(0, |frame| frame.width)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_context_frame_height() -> i32 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.context_frame.as_ref())
            .map_or(0, |frame| frame.height)
    })
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `output` is non-null, it must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_copy_context_frame(
    output: *mut u8,
    capacity: usize,
) -> usize {
    SURFACES.with_borrow(|surfaces| {
        let Some(pixels) = surfaces
            .as_ref()
            .and_then(|surfaces| surfaces.context_frame.as_ref())
            .map(|frame| frame.pixels.as_slice())
        else {
            return 0;
        };
        if !output.is_null() && capacity >= pixels.len() {
            unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), output, pixels.len()) };
        }
        pixels.len()
    })
}

#[unsafe(no_mangle)]
/// # Safety
///
/// When `output` is non-null and `capacity` is nonzero, it must point to `capacity` writable bytes.
pub unsafe extern "C" fn shrimply_qt_timeline_context_action_error(
    output: *mut u8,
    capacity: usize,
) -> usize {
    SURFACES.with_borrow(|surfaces| {
        let error = surfaces
            .as_ref()
            .map_or("", |surfaces| surfaces.context_action_error.as_str());
        let bytes = error.as_bytes();
        if !output.is_null() && capacity > 0 {
            let length = bytes.len().min(capacity - 1);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length);
                output.add(length).write(0);
            }
        }
        bytes.len()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_magnet() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .is_some_and(|surfaces| surfaces.timeline.tool_state().magnet)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_set_magnet(enabled: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.set_magnet(enabled);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_beat_grid() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .is_some_and(|surfaces| surfaces.timeline.tool_state().beat_grid)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_set_beat_grid(enabled: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.set_beat_grid(enabled);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_cut_enabled() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .is_some_and(|surfaces| surfaces.timeline.tool_state().cursor == CursorTool::Cut)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_set_cut_enabled(enabled: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.set_cursor_tool(if enabled {
                CursorTool::Cut
            } else {
                CursorTool::Pointer
            });
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_overwrite_mode() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces.as_ref().is_some_and(|surfaces| {
            surfaces.timeline.tool_state().drag_collision == DragCollisionMode::Overwrite
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_block_mode() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces.as_ref().is_some_and(|surfaces| {
            surfaces.timeline.tool_state().drag_collision == DragCollisionMode::Block
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_new_track_mode() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces.as_ref().is_some_and(|surfaces| {
            surfaces.timeline.tool_state().drag_collision == DragCollisionMode::NewTrack
        })
    })
}

fn select_drag_collision_mode(mode: DragCollisionMode) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.timeline.set_drag_collision_mode(mode);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_select_overwrite_mode() {
    select_drag_collision_mode(DragCollisionMode::Overwrite);
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_select_block_mode() {
    select_drag_collision_mode(DragCollisionMode::Block);
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_timeline_select_new_track_mode() {
    select_drag_collision_mode(DragCollisionMode::NewTrack);
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_move(
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    control: bool,
    shift: bool,
    alt: bool,
) {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.preview.pointer_move(
                width,
                height,
                x,
                y,
                preview::pointer_modifiers(control, shift, alt),
            );
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_cursor() -> u8 {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .map_or(0, |surfaces| surfaces.preview.pointer_cursor())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_leave() {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.preview.pointer_leave();
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_press(
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    control: bool,
    shift: bool,
    alt: bool,
) -> bool {
    SURFACES.with_borrow_mut(|surfaces| {
        surfaces.as_mut().is_some_and(|surfaces| {
            surfaces.preview.pointer_press(
                width,
                height,
                x,
                y,
                preview::pointer_modifiers(control, shift, alt),
            )
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_release(
    width: f32,
    height: f32,
    x: f32,
    y: f32,
    control: bool,
    shift: bool,
    alt: bool,
) {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.preview.pointer_release(
                width,
                height,
                x,
                y,
                preview::pointer_modifiers(control, shift, alt),
            );
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_pointer_cancel() {
    SURFACES.with_borrow_mut(|surfaces| {
        if let Some(surfaces) = surfaces.as_mut() {
            surfaces.preview.pointer_cancel();
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_guides_visible() -> bool {
    SURFACES.with_borrow(|surfaces| {
        surfaces
            .as_ref()
            .is_some_and(|surfaces| surfaces.preview.guides_visible())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn shrimply_qt_preview_set_guides_visible(visible: bool) {
    SURFACES.with_borrow(|surfaces| {
        if let Some(surfaces) = surfaces.as_ref() {
            surfaces.preview.set_guides_visible(visible);
        }
    });
}
