#![recursion_limit = "256"]

use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nFileFilterExt;
use shrimply_components_gtk::ui::I18nMenuExt;
use shrimply_components_gtk::ui::I18nWidgetExt;
use std::cell::{Cell, RefCell};

mod fullscreen;
mod paint_toolbar;
mod preview_surface;
mod transport;
use shrimply_preview_render_core::playback::toolbar::{self as preview_toolbar, ContextAction};
use shrimply_preview_runtime_cuda::{PreviewMedia, StepDirection};
pub use shrimply_preview_runtime_cuda::{
    playback_speed_label, playback_time_label, rendered_frame_rate_label,
};

pub use shrimply_audio_engine as audio;
pub use shrimply_components_gtk::{gl_loader, playback_shortcuts};
pub use shrimply_editor_state::player_state;
pub use shrimply_editor_state::preview_focus;
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project_document::{caption, project, time_format};
pub use shrimply_project_evaluation as transform_eval;
pub use shrimply_property_model::timeline_value;
pub use shrimply_render_core::math;
pub use shrimply_timeline_edit::selection_state;
pub use shrimply_visual_cuda as video;
pub use shrimply_visual_modifiers as modifiers;

pub mod preferences {
    pub use shrimply_editor_state::preferences as store;
}

pub mod timeline {
    pub use shrimply_components_gtk::canvas as renderer;
}
use std::rc::Rc;

use crate::audio::AudioPlayer;
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::preview_focus::SharedPreviewFocus;
use crate::preview_surface::PreviewController;
use crate::project::{Project, Time, scaled_time_delta};
use crate::selection_state::SharedSelectionState;
use crate::timeline::renderer::vec2;
use crate::video::compositor::VideoEvent;
use adw::prelude::AdwDialogExt;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use shrimply_paint_edit_skia::{PAINT_PREVIEW_STATE, PaintPreviewState};
use shrimply_playback_performance as playback_performance;

use shrimply_preview_provider_skia::playback::{
    LoadingIndicator, STEP_REPEAT_TICK, playback_lagging,
};
const LOADING_SPINNER_SIZE: i32 = 16;
const PREVIEW_FULLSCREEN_ICON: &str = "arrows-pointing-outward-symbolic";
const PREVIEW_TOOLBAR_ICON_SIZE: i32 = 20;
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
        shrimply_visual_cuda::compositor::spawn_worker_with_resources_and_observer,
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

    let playbar_controls = transport::controls(
        [&step_back_button, &play_button, &step_forward_button],
        &progress_scale,
        &time_label,
        &fullscreen_button,
    );
    playbar_controls.set_valign(gtk::Align::Start);
    playbar_controls.set_vexpand(false);
    let left_bar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    left_bar.set_margin_top(0);
    left_bar.set_margin_bottom(0);
    left_bar.set_margin_start(0);
    left_bar.set_margin_end(0);
    left_bar.set_halign(gtk::Align::Start);
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
    let paint_tools = paint_toolbar::new(
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        video_surface.clone(),
    );
    left_bar.append(&paint_tools);
    let side_controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_controls.set_hexpand(false);
    side_controls.set_valign(gtk::Align::Fill);
    side_controls.set_vexpand(true);
    side_controls.set_halign(gtk::Align::Start);
    let rail_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_width(true)
        .propagate_natural_height(false)
        .vexpand(true)
        .child(&left_bar)
        .build();
    side_controls.append(&rail_scroll);
    side_controls.set_visible(preferences_store::snapshot(&preferences).preview_controls_visible);

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
    preview_area.set_width_request(shrimply_components_gtk::layout::MIN_PREVIEW_WIDTH as i32);
    preview_area.set_hexpand(true);
    preview_area.set_vexpand(true);
    preview_area.set_child(Some(&layout));

    player_state::set_duration(&player_state, duration);

    let playbar_seek = transport::PlaybarSeek::attach(&progress_scale, &player_state);
    update_controls(
        &play_button,
        &time_label,
        &playback_speed_label,
        &progress_scale,
        &playbar_seek,
        &player_state,
    );

    attach_frame_pump(
        PreviewFramePumpWidgets {
            preview_area: preview_area.clone(),
            video_surface: video_surface.clone(),
            loading_spinner: loading_spinner.clone(),
            loading_indicator: loading_indicator.clone(),
            frame_rate_label: frame_rate_label.clone(),
            playbar_seek: playbar_seek.clone(),
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
    player_state::connect_named(&player_state, "video player controls update", move |_| {
        update_controls(
            &controls_play_button,
            &controls_time_label,
            &controls_playback_speed_label,
            &controls_progress_scale,
            &playbar_seek,
            &controls_state,
        );
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
            preferences: preferences.clone(),
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

struct PreviewFramePumpWidgets {
    preview_area: gtk::Overlay,
    video_surface: PreviewController,
    loading_spinner: adw::Spinner,
    loading_indicator: gtk::Stack,
    frame_rate_label: gtk::Label,
    playbar_seek: Rc<transport::PlaybarSeek>,
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
        playbar_seek,
    } = widgets;
    let displayed_position = Cell::new(None);
    let render_loading = Cell::new(false);
    let loading = LoadingIndicator::default();
    preview_area.add_tick_callback(move |_, _| {
        playbar_seek.flush(&player_state);
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
                    settled,
                    render_elapsed: _,
                    render_generation: _,
                } => {
                    displayed_position.set(Some(position));
                    video_surface.set_frame(
                        frame,
                        audio_analysis,
                        position,
                        revision,
                        excluded_item_id,
                        settled,
                    );
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
                VideoEvent::Loading { .. } | VideoEvent::Manim(_) | VideoEvent::Error(_) => {
                    unreachable!()
                }
            }
        }

        let frame_step = project.borrow().frame_step();
        let playback_frame_step = scaled_time_delta(frame_step, snapshot.playback_speed);
        let show_loading = loading.update(
            render_loading.get()
                || playback_lagging(
                    snapshot.playing,
                    snapshot.position,
                    displayed_position.get(),
                    playback_frame_step,
                ),
        );
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
    view.set_width_request(shrimply_components_gtk::layout::MIN_PREVIEW_WIDTH as i32);
    view
}

fn attach_preview_context_menu(
    video_surface: &PreviewController,
    preferences: preferences_store::SharedPreferences,
) {
    let surface = video_surface.weak();
    let open = gio::SimpleAction::new(
        "open",
        Some(glib::VariantTy::new("(ddb)").expect("preview context parameters")),
    );
    open.connect_activate(move |_, parameter| {
        let Some(surface) = surface() else {
            return;
        };
        let area = surface.widget().clone();
        let (x, y, touch) = parameter
            .and_then(|value| value.get::<(f64, f64, bool)>())
            .expect("preview context source and position");
        let menu = gio::Menu::new();
        let actions = gio::SimpleActionGroup::new();
        for item in preview_toolbar::context_menu(&preferences, true) {
            menu.append(
                Some(&item.label),
                Some(&format!("preview.{}", item.action.id())),
            );
            if item.checked.is_some() {
                let preferences = preferences.clone();
                let action = shrimply_components_gtk::ui::menu_action(
                    item.action.id(),
                    item.checked,
                    move || {
                        assert!(
                            item.action.activate(&preferences).is_none(),
                            "checked preview action completes in shared core"
                        );
                        preview_toolbar::context_menu(&preferences, true)
                            .into_iter()
                            .find(|entry| entry.action == item.action)
                            .and_then(|entry| entry.checked)
                    },
                );
                actions.add_action(&action);
            }
        }
        if surface.can_inspect_sanim() {
            menu.append_i18n("Inspect Sanim Element", "preview.inspect-sanim");
            let inspect = gio::SimpleAction::new("inspect-sanim", None);
            let inspect_surface = surface.clone();
            inspect.connect_activate(move |_, _| {
                let surface = inspect_surface.clone();
                glib::idle_add_local_once(move || {
                    if surface.widget().is_mapped() {
                        surface.inspect_sanim(vec2(x as f32, y as f32));
                    }
                });
            });
            actions.add_action(&inspect);
        }
        let copy = gio::SimpleAction::new(ContextAction::CopyImage.id(), None);
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

        let save = gio::SimpleAction::new(ContextAction::SaveImage.id(), None);
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
            shrimply_components_gtk::file_picker::save(
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

        if shrimply_components_gtk::layout::menu_uses_bottom_sheet(if touch {
            "touch"
        } else {
            "pointer"
        }) {
            let dialog =
                shrimply_components_gtk::ui::action_menu_sheet(&menu, "preview", &actions, &[]);
            dialog.present(Some(&area));
            return;
        }
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
        shrimply_components_gtk::ui::unparent_on_close(&popover);
        popover.popup();
    });
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&open);
    video_surface
        .widget()
        .insert_action_group("preview-context", Some(&actions));
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let area = video_surface.widget().downgrade();
    click.connect_pressed(move |_, _, x, y| {
        let Some(area) = area.upgrade() else {
            return;
        };
        area.activate_action("preview-context.open", Some(&(x, y, false).to_variant()))
            .expect("preview context menu is installed");
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
    playbar_seek: &transport::PlaybarSeek,
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
    playback_speed_label.set_tooltip_text(Some(&shrimply_components_gtk::i18n::text_args(
        "Playback speed %{speed}",
        &[("speed", playback_speed_text)],
    )));

    playbar_seek.sync(progress_scale, snapshot);
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
