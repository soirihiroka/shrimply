use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nFileFilterExt;
use shrimply_gtk_components::ui::I18nMenuExt;
use shrimply_gtk_components::ui::I18nWidgetExt;
use std::cell::{Cell, RefCell};

mod fullscreen;
mod preview_surface;
use shrimply_preview_runtime::{PreviewMedia, StepDirection};
pub use shrimply_preview_runtime::{
    playback_speed_label, playback_time_label, rendered_frame_rate_label,
};

pub use shrimply_audio as audio;
pub use shrimply_core::timeline_value;
pub use shrimply_evaluation as transform_eval;
pub use shrimply_gtk_components::{gl_loader, playback_shortcuts};
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project::{caption, project, time_format};
pub use shrimply_render_core::math;
pub use shrimply_state::player_state;
pub use shrimply_state::preview_focus;
pub use shrimply_timeline::selection_state;
pub use shrimply_video_cuda as video;
pub use shrimply_video_modifiers as modifiers;

pub mod preferences {
    pub use shrimply_state::preferences as store;
}

pub mod timeline {
    pub use shrimply_gtk_components::canvas as renderer;
}
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::audio::AudioPlayer;
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::preview_focus::SharedPreviewFocus;
use crate::preview_surface::PreviewController;
use crate::project::{Project, Time, scaled_time_delta};
use crate::selection_state::SharedSelectionState;
use crate::timeline::renderer::{Vec2, vec2};
use crate::video::compositor::VideoEvent;
use adw::prelude::AdwDialogExt;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use shrimply_paint_edit::{PAINT_PREVIEW_STATE, PaintPreviewMode as PaintMode, PaintPreviewState};
use shrimply_playback_performance as playback_performance;

use shrimply_preview_core::playback::STEP_REPEAT_TICK;
const LOADING_SPINNER_SIZE: i32 = 16;
const LOADING_SPINNER_DELAY: Duration = Duration::from_nanos(1_000_000_000 / 24);
const PREVIEW_FULLSCREEN_ICON: &str = "arrows-pointing-outward-symbolic";
const PREVIEW_TOOLBAR_ICON_SIZE: i32 = 20;
use shrimply_paint_edit::DEFAULT_PAINT_ERASER_SCALE;
type PaintPaletteStructure = Rc<RefCell<Option<(uuid::Uuid, Vec<uuid::Uuid>)>>>;

trait PaintSurfaceState {
    fn set_paint_mode(&self, mode: PaintMode);
    fn set_paint_eraser(&self, enabled: bool);
    fn paint_brush_scale(&self, eraser: bool) -> f32;
    fn set_paint_brush_scale(&self, eraser: bool, scale: f32);
    fn paint_fill_tolerance(&self) -> f32;
    fn set_paint_fill_tolerance(&self, tolerance: f32);
    fn select_paint_color(&self, index: usize, palette_len: usize);
    fn set_paint_onion_skin(&self, previous: bool, enabled: bool);
    fn set_paint_adjusting(&self, enabled: bool);
}

impl PaintSurfaceState for PreviewController {
    fn set_paint_mode(&self, mode: PaintMode) {
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            state.mode = mode;
            if mode == PaintMode::StrokeTransform {
                state.eraser = false;
            }
            if mode != PaintMode::Pen {
                state.focused = None;
            }
        });
    }

    fn set_paint_eraser(&self, enabled: bool) {
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            state.eraser = enabled;
            if enabled {
                state.mode = PaintMode::Pen;
                state.focused = None;
            }
        });
    }

    fn paint_brush_scale(&self, eraser: bool) -> f32 {
        self.preview_state(PAINT_PREVIEW_STATE, |state: &PaintPreviewState| {
            if eraser {
                state.eraser_scale
            } else {
                state.pen_scale
            }
        })
    }

    fn set_paint_brush_scale(&self, eraser: bool, scale: f32) {
        if !scale.is_finite() {
            return;
        }
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            let value = scale.clamp(0.1, 99.0);
            if eraser {
                state.eraser_scale = value;
            } else {
                state.pen_scale = value;
            }
        });
    }

    fn paint_fill_tolerance(&self) -> f32 {
        self.preview_state(PAINT_PREVIEW_STATE, |state: &PaintPreviewState| {
            state.fill_tolerance
        })
    }

    fn set_paint_fill_tolerance(&self, tolerance: f32) {
        if tolerance.is_finite() {
            self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
                state.fill_tolerance = tolerance.clamp(1.0, 99.0);
            });
        }
    }

    fn select_paint_color(&self, index: usize, palette_len: usize) {
        assert!(palette_len > 0, "paint palette is empty");
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            state.palette_index = index.min(palette_len - 1);
        });
    }

    fn set_paint_onion_skin(&self, previous: bool, enabled: bool) {
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            if previous {
                state.onion_previous = enabled;
            } else {
                state.onion_next = enabled;
            }
        });
    }

    fn set_paint_adjusting(&self, enabled: bool) {
        self.update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            state.adjusting = enabled;
        });
    }
}
pub fn new(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    playback_performance: playback_performance::SharedCollector,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    preferences: preferences_store::SharedPreferences,
    audio_player: Rc<AudioPlayer>,
) -> gtk::Widget {
    match player(
        project,
        player_state,
        playback_performance,
        selection_state.clone(),
        preview_focus,
        preferences,
        audio_player,
    ) {
        Ok(widget) => widget.upcast(),
        Err(error) => error_page(&error).upcast(),
    }
}

fn player(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    playback_performance: playback_performance::SharedCollector,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    preferences: preferences_store::SharedPreferences,
    audio_player: Rc<AudioPlayer>,
) -> Result<gtk::Overlay, String> {
    fullscreen::install_css();

    let initial_project = project.borrow().clone();
    let media = PreviewMedia::new(
        project.clone(),
        player_state.clone(),
        playback_performance.clone(),
        preferences.clone(),
    );
    let video_tx = media.sender();
    let video_surface = PreviewController::new(
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        preview_focus,
        preferences.clone(),
        video_tx.clone(),
    );
    video_surface.install_preview_state(
        PAINT_PREVIEW_STATE,
        PaintPreviewState {
            eraser_scale: DEFAULT_PAINT_ERASER_SCALE,
            ..PaintPreviewState::default()
        },
    );

    let duration = initial_project.duration();
    let progress_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        0.0,
        duration.as_secs_f64().max(0.01),
        0.01,
    );
    progress_scale.set_draw_value(false);
    progress_scale.set_hexpand(true);

    let step_back_button = gtk::Button::from_icon_name("media-seek-backward-symbolic");
    step_back_button.set_tooltip_i18n("Step back one frame");
    let play_button = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play_button.set_tooltip_i18n("Play");
    let step_forward_button = gtk::Button::from_icon_name("media-seek-forward-symbolic");
    step_forward_button.set_tooltip_i18n("Step forward one frame");
    let fullscreen_button = gtk::Button::from_icon_name(PREVIEW_FULLSCREEN_ICON);
    fullscreen_button.set_tooltip_i18n("Fullscreen preview");
    let guide_button = gtk::ToggleButton::new();
    guide_button.set_child(Some(&gtk::Image::from_icon_name("ruler-angled-symbolic")));
    guide_button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    guide_button.set_tooltip_i18n("Guide");
    guide_button.set_valign(gtk::Align::Start);
    guide_button.set_vexpand(false);
    guide_button.set_margin_start(4);
    guide_button.set_margin_end(4);
    guide_button.set_margin_top(2);
    guide_button.set_margin_bottom(2);
    guide_button.add_css_class("preview-control-chip");
    guide_button.add_css_class("flat");
    guide_button.set_active(preferences_store::snapshot(&preferences).preview_guides_visible);
    let guide_preferences = preferences.clone();
    guide_button.connect_toggled(move |button| {
        preferences_store::set_preview_guides_visible(&guide_preferences, button.is_active());
    });
    let synced_guide_button = guide_button.clone();
    preferences_store::connect(&preferences, move |snapshot| {
        if synced_guide_button.is_active() != snapshot.preview_guides_visible {
            synced_guide_button.set_active(snapshot.preview_guides_visible);
        }
    });
    step_back_button.add_css_class("flat");
    play_button.add_css_class("flat");
    step_forward_button.add_css_class("flat");
    fullscreen_button.add_css_class("flat");
    let time_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .width_chars(34)
        .build();
    time_label.add_css_class("monospace");
    let playback_speed_label = gtk::Button::with_label(tr!("x1").as_ref());
    playback_speed_label.set_can_focus(false);
    playback_speed_label.set_sensitive(false);
    playback_speed_label.set_margin_start(4);
    playback_speed_label.set_margin_end(4);
    playback_speed_label.set_margin_top(2);
    playback_speed_label.set_margin_bottom(2);
    playback_speed_label.add_css_class("preview-control-chip");
    playback_speed_label.add_css_class("flat");
    playback_speed_label.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    playback_speed_label.set_tooltip_i18n("Playback speed");
    let frame_rate_label = gtk::Label::builder()
        .label(tr!("--").as_ref())
        .width_chars(3)
        .xalign(0.5)
        .build();
    frame_rate_label.set_halign(gtk::Align::Center);
    frame_rate_label.add_css_class("monospace");
    frame_rate_label.add_css_class("preview-frame-rate-label");
    let frame_rate_button = gtk::Button::new();
    frame_rate_button.set_can_focus(false);
    frame_rate_button.set_sensitive(false);
    frame_rate_button.add_css_class("preview-control-chip");
    frame_rate_button.add_css_class("flat");
    frame_rate_button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    frame_rate_button.set_halign(gtk::Align::Center);
    frame_rate_button.set_valign(gtk::Align::Start);
    frame_rate_button.set_hexpand(false);
    frame_rate_button.set_vexpand(false);
    frame_rate_button.set_margin_start(4);
    frame_rate_button.set_margin_end(4);
    frame_rate_button.set_margin_top(2);
    frame_rate_button.set_margin_bottom(2);
    frame_rate_button.set_tooltip_i18n("Frame rate");
    frame_rate_button.set_child(Some(&frame_rate_label));

    let playbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    playbar.set_margin_top(8);
    playbar.set_margin_bottom(8);
    playbar.set_margin_start(8);
    playbar.set_margin_end(8);
    playbar.append(&step_back_button);
    playbar.append(&play_button);
    playbar.append(&step_forward_button);
    playbar.append(&progress_scale);
    playbar.append(&time_label);
    playbar.append(&fullscreen_button);

    let playbar_controls = playbar;
    playbar_controls.set_valign(gtk::Align::Start);
    playbar_controls.set_vexpand(false);
    let left_bar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    left_bar.set_margin_top(0);
    left_bar.set_margin_bottom(0);
    left_bar.set_margin_start(0);
    left_bar.set_margin_end(0);
    left_bar.set_halign(gtk::Align::Start);
    left_bar.set_valign(gtk::Align::Start);
    left_bar.set_valign(gtk::Align::Fill);
    left_bar.set_vexpand(true);
    let left_bar_divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    left_bar_divider.set_margin_top(0);
    left_bar_divider.set_margin_bottom(0);
    left_bar_divider.set_margin_start(0);
    left_bar_divider.set_margin_end(0);

    let loading_spinner = adw::Spinner::new();
    loading_spinner.set_size_request(LOADING_SPINNER_SIZE, LOADING_SPINNER_SIZE);
    loading_spinner.set_visible(false);
    loading_spinner.set_can_target(false);
    loading_spinner.set_halign(gtk::Align::Center);
    loading_spinner.set_valign(gtk::Align::Center);
    let loading_done_icon = gtk::Image::from_icon_name("check-plain-symbolic");
    loading_done_icon.set_size_request(LOADING_SPINNER_SIZE, LOADING_SPINNER_SIZE);
    loading_done_icon.set_pixel_size(LOADING_SPINNER_SIZE);
    loading_done_icon.set_halign(gtk::Align::Center);
    loading_done_icon.set_valign(gtk::Align::Center);
    let loading_done_button = gtk::Button::new();
    loading_done_button.set_child(Some(&loading_done_icon));
    loading_done_button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    loading_done_button.set_sensitive(false);
    loading_done_button.set_can_focus(false);
    loading_done_button.add_css_class("flat");
    loading_done_button.set_valign(gtk::Align::Center);
    loading_done_button.set_halign(gtk::Align::Center);
    let loading_indicator = gtk::Stack::new();
    loading_indicator.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    loading_indicator.set_halign(gtk::Align::Center);
    loading_indicator.set_valign(gtk::Align::Start);
    loading_indicator.set_margin_top(0);
    loading_indicator.set_vexpand(false);
    loading_indicator.add_named(&loading_spinner, Some("loading"));
    loading_indicator.add_named(&loading_done_button, Some("done"));
    loading_indicator.set_visible_child_name("done");
    left_bar.append(&loading_indicator);
    left_bar.append(&frame_rate_button);
    left_bar.append(&playback_speed_label);
    left_bar.append(&left_bar_divider);
    left_bar.append(&guide_button);
    let paint_tools = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let paint_divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    paint_divider.set_margin_top(2);
    paint_divider.set_margin_bottom(2);
    let last_paint_mode = Rc::new(Cell::new(PaintMode::Pen));
    let make_paint_button = |icon: &str, tooltip: &str, mode: PaintMode| {
        let button = gtk::ToggleButton::new();
        button.set_child(Some(&gtk::Image::from_icon_name(icon)));
        button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
        button.set_tooltip_i18n(tooltip);
        button.set_valign(gtk::Align::Start);
        button.set_vexpand(false);
        button.set_margin_start(4);
        button.set_margin_end(4);
        button.set_margin_top(2);
        button.set_margin_bottom(2);
        button.add_css_class("preview-control-chip");
        button.add_css_class("flat");
        let surface = video_surface.clone();
        let last_mode = last_paint_mode.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                if matches!(mode, PaintMode::Pen | PaintMode::Fill) {
                    last_mode.set(mode);
                }
                surface.set_paint_mode(mode);
            }
        });
        button
    };
    let pen_button = make_paint_button("document-edit-symbolic", "Pen (B)", PaintMode::Pen);
    let fill_button = make_paint_button("fill-tool-symbolic", "Fill (F)", PaintMode::Fill);
    let stroke_transform_button = make_paint_button(
        "move-tool-symbolic",
        "Stroke Transform (T)",
        PaintMode::StrokeTransform,
    );
    fill_button.set_group(Some(&pen_button));
    stroke_transform_button.set_group(Some(&pen_button));
    pen_button.set_active(true);
    let eraser_button = gtk::ToggleButton::new();
    eraser_button.set_child(Some(&gtk::Image::from_icon_name("eraser-symbolic")));
    eraser_button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    eraser_button.set_tooltip_i18n("Eraser (E)");
    eraser_button.set_valign(gtk::Align::Start);
    eraser_button.set_vexpand(false);
    eraser_button.set_margin_start(4);
    eraser_button.set_margin_end(4);
    eraser_button.set_margin_top(2);
    eraser_button.set_margin_bottom(2);
    eraser_button.add_css_class("preview-control-chip");
    eraser_button.add_css_class("flat");
    let adjust_button = gtk::ToggleButton::new();
    adjust_button.set_child(Some(&gtk::Image::from_icon_name(
        "function-exponential-symbolic",
    )));
    adjust_button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
    adjust_button.set_tooltip_i18n("Adjust points (hold Ctrl for temporary adjustment)");
    adjust_button.set_valign(gtk::Align::Start);
    adjust_button.set_vexpand(false);
    adjust_button.set_margin_start(4);
    adjust_button.set_margin_end(4);
    adjust_button.set_margin_top(2);
    adjust_button.set_margin_bottom(2);
    adjust_button.add_css_class("preview-control-chip");
    adjust_button.add_css_class("flat");
    let brush_scale =
        gtk::Button::with_label(&paint_scale_label(video_surface.paint_brush_scale(false)));
    brush_scale.set_size_request(
        PREVIEW_TOOLBAR_ICON_SIZE + 22,
        PREVIEW_TOOLBAR_ICON_SIZE + 8,
    );
    brush_scale.set_tooltip_i18n("Pen width scale");
    brush_scale.set_margin_start(2);
    brush_scale.set_margin_end(2);
    brush_scale.add_css_class("preview-control-chip");
    brush_scale.add_css_class("flat");
    let eraser_surface = video_surface.clone();
    let eraser_adjust = adjust_button.clone();
    let eraser_size = brush_scale.clone();
    let eraser_fill = fill_button.clone();
    let eraser_transform = stroke_transform_button.clone();
    let eraser_pen = pen_button.clone();
    eraser_button.connect_toggled(move |button| {
        if button.is_active() {
            eraser_adjust.set_active(false);
            if eraser_transform.is_active() {
                eraser_pen.set_active(true);
            }
        }
        eraser_surface.set_paint_eraser(button.is_active());
        if eraser_fill.is_active() {
            eraser_size.set_label(&paint_fill_tolerance_label(
                eraser_surface.paint_fill_tolerance(),
            ));
            eraser_size.set_tooltip_i18n("Fill gap tolerance");
        } else {
            eraser_size.set_label(&paint_scale_label(
                eraser_surface.paint_brush_scale(button.is_active()),
            ));
            eraser_size.set_tooltip_i18n(if button.is_active() {
                "Eraser width scale"
            } else {
                "Pen width scale"
            });
        }
    });
    let scale_popover = gtk::Popover::new();
    scale_popover.set_autohide(true);
    scale_popover.set_has_arrow(true);
    scale_popover.set_parent(&brush_scale);
    let scale_slider = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.1, 99.0, 0.1);
    scale_slider.set_size_request(56, 280);
    scale_slider.set_vexpand(true);
    scale_slider.set_draw_value(false);
    scale_popover.set_child(Some(&scale_slider));
    let slider_updating = Rc::new(Cell::new(false));
    let slider_surface = video_surface.clone();
    let slider_eraser = eraser_button.clone();
    let slider_fill = fill_button.clone();
    let slider_button = brush_scale.clone();
    let slider_updating_value = slider_updating.clone();
    scale_slider.connect_value_changed(move |slider| {
        if slider_updating_value.get() {
            return;
        }
        if slider_fill.is_active() {
            let tolerance = slider.value().round() as f32;
            if slider.value() != f64::from(tolerance) {
                slider_updating_value.set(true);
                slider.set_value(f64::from(tolerance));
                slider_updating_value.set(false);
            }
            slider_surface.set_paint_fill_tolerance(tolerance);
            slider_button.set_label(&paint_fill_tolerance_label(tolerance));
        } else {
            let scale = if slider.value() < 1.0 {
                (slider.value() * 10.0).round() as f32 / 10.0
            } else {
                slider.value().round() as f32
            };
            if slider.value() != f64::from(scale) {
                slider_updating_value.set(true);
                slider.set_value(f64::from(scale));
                slider_updating_value.set(false);
            }
            let eraser = slider_eraser.is_active();
            slider_surface.set_paint_brush_scale(eraser, scale);
            slider_button.set_label(&paint_scale_label(scale));
        }
    });
    let size_surface = video_surface.clone();
    let size_eraser = eraser_button.clone();
    let size_fill = fill_button.clone();
    let size_popover = scale_popover.clone();
    let size_slider = scale_slider.clone();
    brush_scale.connect_clicked(move |_| {
        slider_updating.set(true);
        if size_fill.is_active() {
            size_slider.set_digits(0);
            size_slider.adjustment().configure(
                f64::from(size_surface.paint_fill_tolerance()),
                1.0,
                99.0,
                1.0,
                10.0,
                0.0,
            );
        } else {
            let eraser = size_eraser.is_active();
            size_slider.set_digits(1);
            size_slider.adjustment().configure(
                f64::from(size_surface.paint_brush_scale(eraser)),
                0.1,
                99.0,
                0.1,
                1.0,
                0.0,
            );
        }
        slider_updating.set(false);
        size_popover.popup();
    });
    let smaller_scale = gtk::GestureClick::new();
    smaller_scale.set_button(gdk::BUTTON_SECONDARY);
    let smaller_surface = video_surface.clone();
    let smaller_eraser = eraser_button.clone();
    let smaller_fill = fill_button.clone();
    let smaller_button = brush_scale.clone();
    smaller_scale.connect_pressed(move |_, _, _, _| {
        if smaller_fill.is_active() {
            let tolerance = stepped_fill_tolerance(smaller_surface.paint_fill_tolerance(), false);
            smaller_surface.set_paint_fill_tolerance(tolerance);
            smaller_button.set_label(&paint_fill_tolerance_label(tolerance));
        } else {
            let eraser = smaller_eraser.is_active();
            let scale = stepped_paint_scale(smaller_surface.paint_brush_scale(eraser), false);
            smaller_surface.set_paint_brush_scale(eraser, scale);
            smaller_button.set_label(&paint_scale_label(scale));
        }
    });
    brush_scale.add_controller(smaller_scale);
    let adjust_surface = video_surface.clone();
    let adjust_pen = pen_button.clone();
    let adjust_eraser = eraser_button.clone();
    adjust_button.connect_toggled(move |button| {
        if button.is_active() {
            adjust_pen.set_active(true);
            adjust_eraser.set_active(false);
        }
        adjust_surface.set_paint_adjusting(button.is_active());
    });
    let fill_adjust = adjust_button.clone();
    let fill_size = brush_scale.clone();
    let fill_surface = video_surface.clone();
    let fill_eraser = eraser_button.clone();
    fill_button.connect_toggled(move |button| {
        if button.is_active() {
            fill_adjust.set_active(false);
            fill_size.set_label(&paint_fill_tolerance_label(
                fill_surface.paint_fill_tolerance(),
            ));
            fill_size.set_tooltip_i18n("Fill gap tolerance");
        } else {
            let eraser = fill_eraser.is_active();
            fill_size.set_label(&paint_scale_label(fill_surface.paint_brush_scale(eraser)));
            fill_size.set_tooltip_i18n(if eraser {
                "Eraser width scale"
            } else {
                "Pen width scale"
            });
        }
    });
    let transform_adjust = adjust_button.clone();
    let transform_eraser = eraser_button.clone();
    stroke_transform_button.connect_toggled(move |button| {
        if button.is_active() {
            transform_adjust.set_active(false);
            transform_eraser.set_active(false);
        }
    });
    paint_tools.append(&paint_divider);
    paint_tools.append(&pen_button);
    paint_tools.append(&fill_button);
    paint_tools.append(&eraser_button);
    paint_tools.append(&adjust_button);
    paint_tools.append(&stroke_transform_button);
    paint_tools.append(&brush_scale);
    let palette_divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    palette_divider.set_margin_top(2);
    palette_divider.set_margin_bottom(2);
    paint_tools.append(&palette_divider);
    let palette_tools = gtk::Box::new(gtk::Orientation::Vertical, 2);
    paint_tools.append(&palette_tools);
    let onion_divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    onion_divider.set_margin_top(2);
    onion_divider.set_margin_bottom(2);
    paint_tools.append(&onion_divider);
    let make_onion_button = |label: &str, tooltip: &str, previous: bool| {
        let button = gtk::ToggleButton::with_label(label);
        button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
        button.set_tooltip_i18n(tooltip);
        button.set_margin_start(4);
        button.set_margin_end(4);
        button.add_css_class("preview-control-chip");
        button.add_css_class("flat");
        let surface = video_surface.clone();
        button.connect_toggled(move |button| {
            surface.set_paint_onion_skin(previous, button.is_active());
        });
        button
    };
    paint_tools.append(&make_onion_button(
        "−1",
        "Previous drawing onion skin",
        true,
    ));
    paint_tools.append(&make_onion_button("+1", "Next drawing onion skin", false));

    let selected_palette_index = Rc::new(Cell::new(0));
    let palette_structure = Rc::new(RefCell::new(None));
    rebuild_paint_palette_buttons(
        &palette_tools,
        &project,
        &player_state,
        &selection_state,
        &video_surface,
        &selected_palette_index,
        &palette_structure,
    );
    let palette_selection_tools = palette_tools.clone();
    let palette_selection_project = project.clone();
    let palette_selection_player = player_state.clone();
    let palette_selection_state = selection_state.clone();
    let palette_selection_surface = video_surface.clone();
    let palette_selection_index = selected_palette_index.clone();
    let palette_selection_structure = palette_structure.clone();
    selection_state::connect_named(
        &selection_state,
        "preview paint palette selection",
        move || {
            rebuild_paint_palette_buttons(
                &palette_selection_tools,
                &palette_selection_project,
                &palette_selection_player,
                &palette_selection_state,
                &palette_selection_surface,
                &palette_selection_index,
                &palette_selection_structure,
            );
        },
    );
    let palette_player_tools = palette_tools.clone();
    let palette_player_project = project.clone();
    let palette_player_state = player_state.clone();
    let palette_player_selection = selection_state.clone();
    let palette_player_surface = video_surface.clone();
    let palette_player_index = selected_palette_index.clone();
    let palette_player_structure = palette_structure.clone();
    player_state::connect_named(
        &player_state,
        "preview paint palette colors",
        move |event| match event {
            player_state::PlayerEvent::State(_) => palette_player_tools.queue_draw(),
            player_state::PlayerEvent::Project(_) => rebuild_paint_palette_buttons(
                &palette_player_tools,
                &palette_player_project,
                &palette_player_state,
                &palette_player_selection,
                &palette_player_surface,
                &palette_player_index,
                &palette_player_structure,
            ),
        },
    );

    let paint_visible = {
        let project = project.borrow();
        selection_state::focused_video_address(&selection_state, &project)
            .and_then(|key| project.video_item(&key))
            .is_some_and(|item| matches!(item.content, crate::project::VideoItemContent::Paint(_)))
    };
    paint_tools.set_visible(paint_visible);
    let visible_tools = paint_tools.clone();
    let visible_project = project.clone();
    let visible_selection = selection_state.clone();
    selection_state::connect_named(
        &selection_state,
        "preview paint tools visibility",
        move || {
            let project = visible_project.borrow();
            let visible = selection_state::focused_video_address(&visible_selection, &project)
                .and_then(|key| project.video_item(&key))
                .is_some_and(|item| {
                    matches!(item.content, crate::project::VideoItemContent::Paint(_))
                });
            visible_tools.set_visible(visible);
        },
    );
    left_bar.append(&paint_tools);
    let side_controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_controls.set_hexpand(false);
    side_controls.set_valign(gtk::Align::Fill);
    side_controls.set_vexpand(true);
    side_controls.set_halign(gtk::Align::Start);
    side_controls.set_valign(gtk::Align::Start);
    side_controls.append(&left_bar);

    let video_overlay = gtk::Overlay::new();
    video_overlay.set_hexpand(true);
    video_overlay.set_vexpand(true);
    video_overlay.set_child(Some(video_surface.widget()));
    attach_preview_context_menu(&video_surface, preferences.clone());

    let controls_host = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    controls_host.set_hexpand(true);
    controls_host.set_vexpand(true);
    controls_host.append(&side_controls);
    controls_host.append(&video_overlay);
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layout.set_hexpand(true);
    layout.set_vexpand(true);
    layout.append(&controls_host);
    layout.append(&playbar_controls);
    let toggle_state = player_state.clone();
    let speed_state = player_state.clone();
    playback_shortcuts::attach_space_play_toggle(
        &layout,
        move || player_state::toggle_playing(&toggle_state),
        move || player_state::step_playback_speed_forward(&speed_state),
    );

    let preview_area = gtk::Overlay::new();
    preview_area.set_hexpand(true);
    preview_area.set_vexpand(true);
    preview_area.set_child(Some(&layout));

    player_state::set_duration(&player_state, duration);

    let updating_progress = Rc::new(Cell::new(false));
    update_controls(
        &play_button,
        &time_label,
        &playback_speed_label,
        &progress_scale,
        &updating_progress,
        &player_state,
    );

    attach_frame_pump(
        PreviewFramePumpWidgets {
            preview_area: preview_area.clone(),
            video_surface: video_surface.clone(),
            loading_spinner: loading_spinner.clone(),
            loading_indicator: loading_indicator.clone(),
            frame_rate_label: frame_rate_label.clone(),
        },
        project.clone(),
        player_state.clone(),
        media.clone(),
    );

    let step_direction = media.step_direction();

    let caption_surface = video_surface.clone();
    player_state::connect_named(&player_state, "video player caption render", move |event| {
        match event {
            player_state::PlayerEvent::State(_) => caption_surface.queue_render(),
            player_state::PlayerEvent::Project(change) if change.captions => {
                caption_surface.queue_render();
            }
            _ => {}
        }
    });

    let controls_state = player_state.clone();
    let controls_play_button = play_button.clone();
    let controls_time_label = time_label.clone();
    let controls_playback_speed_label = playback_speed_label.clone();
    let controls_progress_scale = progress_scale.clone();
    let controls_updating_progress = updating_progress.clone();
    player_state::connect_named(&player_state, "video player controls update", move |_| {
        update_controls(
            &controls_play_button,
            &controls_time_label,
            &controls_playback_speed_label,
            &controls_progress_scale,
            &controls_updating_progress,
            &controls_state,
        );
    });

    let progress_state = player_state.clone();
    let progress_updating = updating_progress.clone();
    progress_scale.connect_value_changed(move |scale| {
        if progress_updating.get() {
            return;
        }

        player_state::seek_time(&progress_state, Time::from_seconds_f64(scale.value()));
    });

    let play_state = player_state.clone();
    play_button.connect_clicked(move |_| {
        let snapshot = player_state::snapshot(&play_state);
        tracing::info!(
            "Play button clicked: playing {} -> {}, position {} / {}",
            snapshot.playing,
            !snapshot.playing,
            snapshot.position.as_label(),
            snapshot.duration.as_label()
        );
        player_state::toggle_playing(&play_state);
    });

    attach_step_button(
        &step_back_button,
        player_state.clone(),
        step_direction.clone(),
        StepDirection::Backward,
    );
    attach_step_button(
        &step_forward_button,
        player_state.clone(),
        step_direction.clone(),
        StepDirection::Forward,
    );
    fullscreen::attach(
        fullscreen::Widgets {
            layout: layout.clone(),
            video_overlay: video_overlay.clone(),
            controls: playbar_controls.clone(),
            side_controls: side_controls.clone(),
            left_bar: left_bar.clone(),
            button: fullscreen_button.clone(),
            video_surface: video_surface.clone(),
        },
        player_state.clone(),
    );

    let stop_media = media;
    let stop_audio_player = audio_player.clone();
    layout.connect_unrealize(move |_| {
        stop_media.stop();
        stop_audio_player.stop();
    });

    Ok(preview_area)
}

fn paint_scale_label(scale: f32) -> String {
    if scale < 1.0 {
        format!(".{}x", (scale * 10.0).round() as u8)
    } else {
        format!("{:.0}x", scale)
    }
}

fn stepped_paint_scale(scale: f32, larger: bool) -> f32 {
    if larger {
        if scale < 1.0 {
            (scale * 10.0 + 1.0).round().min(10.0) / 10.0
        } else {
            (scale.round() + 1.0).min(99.0)
        }
    } else if scale > 1.0 {
        (scale.round() - 1.0).max(1.0)
    } else {
        (scale * 10.0 - 1.0).round().max(1.0) / 10.0
    }
}

fn stepped_fill_tolerance(tolerance: f32, larger: bool) -> f32 {
    if larger {
        (tolerance.round() + 1.0).min(99.0)
    } else {
        (tolerance.round() - 1.0).max(1.0)
    }
}

fn paint_fill_tolerance_label(tolerance: f32) -> String {
    format!("{:.0}", tolerance.clamp(1.0, 99.0))
}

fn rebuild_paint_palette_buttons(
    tools: &gtk::Box,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    video_surface: &PreviewController,
    selected_index: &Rc<Cell<usize>>,
    structure: &PaintPaletteStructure,
) {
    let palette = {
        let project = project.borrow();
        let Some(key) = selection_state::focused_video_address(selection_state, &project) else {
            structure.borrow_mut().take();
            while let Some(child) = tools.first_child() {
                tools.remove(&child);
            }
            return;
        };
        let Some(item) = project.video_item(&key) else {
            return;
        };
        let crate::project::VideoItemContent::Paint(paint) = &item.content else {
            structure.borrow_mut().take();
            while let Some(child) = tools.first_child() {
                tools.remove(&child);
            }
            return;
        };
        (
            key,
            item.id,
            paint
                .palette
                .iter()
                .map(|entry| entry.color.id)
                .collect::<Vec<_>>(),
        )
    };
    if palette.2.is_empty() {
        return;
    }
    if structure.borrow().as_ref() == Some(&(palette.1, palette.2.clone())) {
        tools.queue_draw();
        return;
    }
    *structure.borrow_mut() = Some((palette.1, palette.2.clone()));
    while let Some(child) = tools.first_child() {
        tools.remove(&child);
    }

    let active_index = selected_index.get().min(palette.2.len() - 1);
    if active_index != selected_index.get() {
        selected_index.set(active_index);
        video_surface.select_paint_color(active_index, palette.2.len());
    }
    let palette_len = palette.2.len();
    let mut group = None::<gtk::ToggleButton>;
    for (index, color_id) in palette.2.into_iter().enumerate() {
        let swatch = gtk::DrawingArea::new();
        swatch.set_content_width(PREVIEW_TOOLBAR_ICON_SIZE - 6);
        swatch.set_content_height(PREVIEW_TOOLBAR_ICON_SIZE - 6);
        let swatch_project = project.clone();
        let swatch_player = player_state.clone();
        let swatch_key = palette.0.clone();
        swatch.set_draw_func(move |_, context, width, height| {
            let project = swatch_project.borrow();
            let Some(sequence_time) = project.timeline_time_to_sequence(
                &swatch_key.track(),
                player_state::snapshot(&swatch_player).position,
            ) else {
                return;
            };
            let Some(item) = project.video_item(&swatch_key) else {
                return;
            };
            let crate::project::VideoItemContent::Paint(paint) = &item.content else {
                return;
            };
            let local_time = crate::project::generated_item_time(item, sequence_time)
                .unwrap_or(crate::project::Time::ZERO);
            let Some(color) = paint
                .palette
                .iter()
                .map(|entry| &entry.color)
                .find(|color| color.id == color_id)
                .map(|color| color.value_at(local_time))
            else {
                return;
            };
            let [red, green, blue, alpha] = color.to_srgba();
            context.set_source_rgba(
                f64::from(red),
                f64::from(green),
                f64::from(blue),
                f64::from(alpha),
            );
            let radius = f64::from(width.min(height)) * 0.5;
            context.arc(
                f64::from(width) * 0.5,
                f64::from(height) * 0.5,
                radius,
                0.0,
                std::f64::consts::TAU,
            );
            context.fill().expect("could not draw paint palette swatch");
        });
        let button = gtk::ToggleButton::new();
        button.set_child(Some(&swatch));
        button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
        button.set_tooltip_text(Some(&shrimply_gtk_components::i18n::text_args(
            "Paint texture %{number}",
            &[("number", (index + 1).to_string())],
        )));
        button.set_margin_start(4);
        button.set_margin_end(4);
        button.set_margin_top(2);
        button.set_margin_bottom(2);
        button.add_css_class("preview-control-chip");
        button.add_css_class("flat");
        if let Some(group) = &group {
            button.set_group(Some(group));
        } else {
            group = Some(button.clone());
        }
        let surface = video_surface.clone();
        let selected = selected_index.clone();
        button.connect_toggled(move |button| {
            if button.is_active() && selected.get() != index {
                selected.set(index);
                surface.select_paint_color(index, palette_len);
            }
        });
        button.set_active(index == active_index);
        tools.append(&button);
    }
}

struct PreviewFramePumpWidgets {
    preview_area: gtk::Overlay,
    video_surface: PreviewController,
    loading_spinner: adw::Spinner,
    loading_indicator: gtk::Stack,
    frame_rate_label: gtk::Label,
}

fn attach_frame_pump(
    widgets: PreviewFramePumpWidgets,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    media: PreviewMedia,
) {
    let PreviewFramePumpWidgets {
        preview_area,
        video_surface,
        loading_spinner,
        loading_indicator,
        frame_rate_label,
    } = widgets;
    let displayed_position = Cell::new(None);
    let render_loading = Cell::new(false);
    let loading_since = Cell::new(None::<Instant>);
    preview_area.add_tick_callback(move |_, _| {
        let update = media.poll();
        if !update.running {
            return glib::ControlFlow::Break;
        }
        if let Some(loading) = update.render_loading {
            render_loading.set(loading);
        }

        let snapshot = player_state::snapshot(&player_state);
        if let Some(label) = update.render_elapsed.and_then(rendered_frame_rate_label) {
            frame_rate_label.set_label(&label);
        }

        if let Some(event) = update.visual {
            match event {
                VideoEvent::Frame {
                    frame,
                    audio_analysis,
                    position,
                    revision,
                    excluded_item_id,
                    settled: _,
                    render_elapsed: _,
                    render_generation: _,
                } => {
                    displayed_position.set(Some(position));
                    video_surface.set_frame(frame, audio_analysis, revision, excluded_item_id);
                }
                VideoEvent::Clear {
                    audio_analysis,
                    position,
                    revision,
                    excluded_item_id,
                    render_elapsed: _,
                    render_generation: _,
                } => {
                    displayed_position.set(Some(position));
                    video_surface.clear_frame(audio_analysis, revision, excluded_item_id);
                }
                VideoEvent::Loading { .. }
                | VideoEvent::ManimDuration { .. }
                | VideoEvent::ManimParameters { .. }
                | VideoEvent::ManimStatus { .. }
                | VideoEvent::Error(_) => unreachable!(),
            }
        }

        let frame_step = project.borrow().frame_step();
        let playback_frame_step = scaled_time_delta(frame_step, snapshot.playback_speed);
        let playback_lagging = snapshot.playing
            && displayed_position.get().is_none_or(|displayed| {
                displayed
                    .max(snapshot.position)
                    .saturating_sub(displayed.min(snapshot.position))
                    > playback_frame_step
            });
        let is_loading = render_loading.get() || playback_lagging;
        if !is_loading {
            loading_since.set(None);
        } else if loading_since.get().is_none() {
            loading_since.set(Some(Instant::now()));
        }
        let show_loading = loading_since
            .get()
            .is_some_and(|loading_since| loading_since.elapsed() >= LOADING_SPINNER_DELAY);
        loading_spinner.set_visible(show_loading);
        loading_indicator.set_visible_child_name(if show_loading { "loading" } else { "done" });
        glib::ControlFlow::Continue
    });
}

fn error_page(error: &str) -> adw::StatusPage {
    let view = adw::StatusPage::builder()
        .title(tr!("Video Player").as_ref())
        .description(error)
        .icon_name("video-display-symbolic")
        .hexpand(true)
        .vexpand(true)
        .build();
    view.set_width_request(640);
    view
}

fn attach_preview_context_menu(
    video_surface: &PreviewController,
    preferences: preferences_store::SharedPreferences,
) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let area = video_surface.widget().clone();
    let surface = video_surface.clone();
    click.connect_pressed(move |_, _, x, y| {
        let menu = gio::Menu::new();
        menu.append_i18n("Copy Preview Image", "preview.copy-image");
        menu.append_i18n("Save Preview Image…", "preview.save-image");

        let actions = gio::SimpleActionGroup::new();
        let copy = gio::SimpleAction::new("copy-image", None);
        copy.set_enabled(surface.has_frame());
        let copy_area = area.clone();
        let copy_surface = surface.clone();
        copy.connect_activate(move |_, _| match copy_surface.current_frame_texture() {
            Ok(Some(texture)) => copy_area.display().clipboard().set_texture(&texture),
            Ok(None) => {}
            Err(error) => {
                show_preview_image_error(&copy_area, "Could not copy preview image", &error)
            }
        });
        actions.add_action(&copy);

        let save = gio::SimpleAction::new("save-image", None);
        save.set_enabled(surface.has_frame());
        let save_area = area.clone();
        let save_surface = surface.clone();
        let save_preferences = preferences.clone();
        save.connect_activate(move |_, _| {
            let texture = match save_surface.current_frame_texture() {
                Ok(Some(texture)) => texture,
                Ok(None) => return,
                Err(error) => {
                    show_preview_image_error(&save_area, "Could not save preview image", &error);
                    return;
                }
            };
            let filter = gtk::FileFilter::new();
            filter.set_name_i18n("PNG image");
            filter.add_mime_type("image/png");
            filter.add_pattern("*.png");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let label = "Save Preview Image";
            let dialog = gtk::FileDialog::builder()
                .title(tr!(label).as_ref())
                .initial_name("preview.png")
                .filters(&filters)
                .default_filter(&filter)
                .build();
            let initial_folder = preferences_store::preview_image_folder(&save_preferences)
                .or_else(|| glib::user_special_dir(glib::UserDirectory::Pictures));
            if let Some(folder) = initial_folder {
                dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
            let parent = save_area.root().and_downcast::<gtk::Window>();
            let result_area = save_area.clone();
            let result_preferences = save_preferences.clone();
            shrimply_gtk_components::file_picker::save(
                label,
                &dialog,
                parent.as_ref(),
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(mut path) = file.path() else {
                        show_preview_image_error(
                            &result_area,
                            "Could not save preview image",
                            "The selected location does not have a local path.",
                        );
                        return;
                    };
                    if !path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                    {
                        path.set_extension("png");
                    }
                    if let Some(folder) = path.parent() {
                        preferences_store::set_preview_image_folder(&result_preferences, folder);
                    }
                    if let Err(error) = texture.save_to_png(&path) {
                        show_preview_image_error(
                            &result_area,
                            "Could not save preview image",
                            &error.to_string(),
                        );
                    }
                },
            );
        });
        actions.add_action(&save);

        let parent = area.parent().expect("preview GLArea must have a parent");
        let point = area
            .compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
            .expect("preview coordinates must translate to its parent");
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_halign(gtk::Align::Start);
        popover.insert_action_group("preview", Some(&actions));
        popover.set_parent(&parent);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            point.x() as i32,
            point.y() as i32,
            1,
            1,
        )));
        popover.popup();
    });
    video_surface.widget().add_controller(click);
}

fn show_preview_image_error(area: &gtk::GLArea, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.present(area.root().and_downcast::<gtk::Window>().as_ref());
}

fn update_controls(
    play_button: &gtk::Button,
    time_label: &gtk::Label,
    playback_speed_label: &gtk::Button,
    progress_scale: &gtk::Scale,
    updating_progress: &Rc<Cell<bool>>,
    player_state: &SharedPlayerState,
) {
    let snapshot = player_state::snapshot(player_state);
    if snapshot.playing {
        play_button.set_icon_name("media-playback-pause-symbolic");
        play_button.set_tooltip_i18n("Pause");
    } else {
        play_button.set_icon_name("media-playback-start-symbolic");
        play_button.set_tooltip_i18n("Play");
    }

    time_label.set_label(&playback_time_label(snapshot.position, snapshot.duration));
    let playback_speed_text = crate::playback_speed_label(snapshot.playback_speed);
    playback_speed_label.set_label(&playback_speed_text);
    playback_speed_label.set_tooltip_text(Some(&shrimply_gtk_components::i18n::text_args(
        "Playback speed %{speed}",
        &[("speed", playback_speed_text)],
    )));

    updating_progress.set(true);
    progress_scale.set_range(
        0.0,
        (snapshot
            .duration
            .as_secs_f64()
            .max(snapshot.position.as_secs_f64()))
        .max(0.01),
    );
    progress_scale.set_value(snapshot.position.as_secs_f64());
    updating_progress.set(false);
}

fn attach_step_button(
    button: &gtk::Button,
    player_state: SharedPlayerState,
    step_direction: Rc<Cell<Option<StepDirection>>>,
    direction: StepDirection,
) {
    let press_generation = Rc::new(Cell::new(0u64));
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);

    let pressed_state = player_state.clone();
    let pressed_step_direction = step_direction.clone();
    let pressed_generation = press_generation.clone();
    click.connect_pressed(move |_, _, _, _| {
        let generation = pressed_generation.get().wrapping_add(1);
        pressed_generation.set(generation);
        step_by_frame(&pressed_state, &pressed_step_direction, direction);

        let repeat_state = pressed_state.clone();
        let repeat_step_direction = pressed_step_direction.clone();
        let repeat_generation = pressed_generation.clone();
        glib::timeout_add_local(STEP_REPEAT_TICK, move || {
            if repeat_generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            step_by_frame(&repeat_state, &repeat_step_direction, direction);
            glib::ControlFlow::Continue
        });
    });

    let released_generation = press_generation.clone();
    click.connect_released(move |_, _, _, _| {
        released_generation.set(released_generation.get().wrapping_add(1));
    });

    let cancelled_generation = press_generation.clone();
    click.connect_cancel(move |_, _| {
        cancelled_generation.set(cancelled_generation.get().wrapping_add(1));
    });

    button.add_controller(click);
}

fn step_by_frame(
    player_state: &SharedPlayerState,
    step_direction: &Rc<Cell<Option<StepDirection>>>,
    direction: StepDirection,
) {
    player_state::set_playing(player_state, false);
    let snapshot = player_state::snapshot(player_state);
    let step = shrimply_math_core::time_from_frame(1, snapshot.frame_rate)
        .expect("validated project FPS must produce an exact frame step");
    let position = match direction {
        StepDirection::Backward => snapshot.position.saturating_sub(step),
        StepDirection::Forward => snapshot.position.saturating_add(step),
    };
    if position != snapshot.position {
        step_direction.set(Some(direction));
    }
    player_state::seek_time(player_state, position);
    tracing::debug!(
        "Exact frame step to {} from {}",
        position.as_label(),
        snapshot.position.as_label()
    );
}
