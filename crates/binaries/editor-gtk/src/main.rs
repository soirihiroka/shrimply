use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nAlertDialogExt;
use shrimply_components_gtk::ui::I18nFileFilterExt;
use shrimply_export_gtk as export;
use shrimply_process_reporting::crash;
mod header_menu;
mod inspector_layout;
mod view_preferences;
use shrimply_components_gtk::project_settings::ProjectSettingsSelector;
use shrimply_editor_state::player_state;
use shrimply_inspector_gtk as inspector;
use shrimply_preferences_core as preferences;
use shrimply_preview_gtk as video_player;
use shrimply_timeline_gtk as timeline;

pub use shrimply_audio_engine as audio;
pub use shrimply_project_document::project;

use adw::prelude::*;
use ffmpeg_next as ffmpeg;
use gdk_pixbuf::prelude::PixbufAnimationExtManual;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::{gio, glib};
use shrimply_cross_ui_core::editor::{EditorSession, LoadEvent, ProjectLoader};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

const DEFAULT_WINDOW_WIDTH: i32 = 1800;
const DEFAULT_WINDOW_HEIGHT: i32 = 1100;
use shrimply_components_core::layout::PANEL_ANIMATION_DURATION_MS;
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(16);
const LOADING_WINDOW_WIDTH: i32 = 800;
const LOADING_WINDOW_HEIGHT: i32 = 600;
const LOADING_SHRIMP_WIDTH: i32 = 160;
const LOADING_SHRIMP_HEIGHT: i32 = 180;
const KDENLIVE_IMPORT_DOCS: &str = "https://shrimply.pages.dev/guides/kdenlive-import";

#[derive(Clone, Copy)]
enum PanelSide {
    Start,
    End,
}

impl PanelSide {
    fn orientation(self) -> gtk::Orientation {
        match self {
            Self::Start => gtk::Orientation::Horizontal,
            Self::End => gtk::Orientation::Vertical,
        }
    }

    fn set_size_request(self, widget: &impl IsA<gtk::Widget>, size: i32) {
        match self {
            Self::Start => widget.set_width_request(size),
            Self::End => widget.set_height_request(size),
        }
    }

    fn set_animating_policy(self, pane: &gtk::ScrolledWindow, animating: bool) {
        let policy = if animating {
            gtk::PolicyType::External
        } else {
            gtk::PolicyType::Never
        };
        match self {
            Self::Start => pane.set_policy(policy, gtk::PolicyType::Never),
            Self::End => pane.set_policy(gtk::PolicyType::Never, policy),
        }
    }

    fn rest_size_request(self) -> i32 {
        match self {
            Self::Start => inspector::INSPECTOR_MIN_WIDTH,
            Self::End => -1,
        }
    }

    fn set_shrink(self, paned: &gtk::Paned, shrink: bool) {
        match self {
            Self::Start => paned.set_shrink_start_child(shrink),
            Self::End => paned.set_shrink_end_child(shrink),
        }
    }
}

fn main() -> glib::ExitCode {
    crash::install();
    shrimply_process_reporting::diagnostics::init();
    let mut args = std::env::args_os().skip(1);
    let project_path = match (args.next(), args.next()) {
        (Some(path), None) => PathBuf::from(path),
        _ => {
            eprintln!(
                "usage: shrimply-editor PROJECT.shrimp|PROJECT.json|TIMELINE.otio|PROJECT.kdenlive\nRun shrimply without arguments to open the start window."
            );
            return glib::ExitCode::FAILURE;
        }
    };
    #[cfg(target_os = "linux")]
    if let Err(error) = shrimply_visual_cuda::gpu::configure_gtk_backend()
        .and_then(|()| shrimply_visual_cuda::gpu::configure_gtk_presentation())
    {
        tracing::error!(%error, "Could not select GTK GPU backend");
        return glib::ExitCode::FAILURE;
    }
    shrimply_components_gtk::i18n::init_system_locale();
    glib::log_set_writer_func(glib::rust_log_writer);
    glib::set_print_handler(|message| {
        tracing::info!("glib print: {}", message.trim_end());
    });
    glib::set_printerr_handler(|message| {
        tracing::error!("glib printerr: {}", message.trim_end());
    });
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Shrimply started");

    let app = adw::Application::new(
        Some("dev.shrimply.Shrimply.Editor"),
        gio::ApplicationFlags::NON_UNIQUE,
    );

    app.connect_activate(move |app| {
        let style = adw::StyleManager::default();
        shrimply_cross_ui_theme::set_dark(style.is_dark());
        shrimply_cross_ui_theme::set_accent(Some((
            style.accent_color_rgba().into(),
            shrimply_preview_gtk::Color::ACCENT_FG,
        )));
        style.connect_dark_notify(|style| {
            shrimply_cross_ui_theme::set_dark(style.is_dark());
        });
        style.connect_accent_color_notify(|style| {
            shrimply_cross_ui_theme::set_accent(Some((
                style.accent_color_rgba().into(),
                shrimply_preview_gtk::Color::ACCENT_FG,
            )));
        });
        begin_project_load(app, project_path.clone());
    });
    let status = app.run_with_args(&[env!("CARGO_BIN_NAME")]);
    let save_result = project::shutdown_history();
    project::clear_project_file_locks();
    if let Err(error) = save_result {
        tracing::error!("Could not save project during shutdown: {error}");
        return glib::ExitCode::FAILURE;
    }
    status
}

fn build_ui(window: &adw::ApplicationWindow, project: project::Project) {
    // Finish GDK's lazy Vulkan initialization before the video worker can initialize Vulkan too.
    drop(gtk::prelude::WidgetExt::display(window).dmabuf_formats());
    let session = Rc::new(
        EditorSession::new(project)
            .unwrap_or_else(|error| panic!("could not initialize editor session: {error}")),
    );
    let project = session.project.clone();
    let player_state = session.player_state.clone();
    let playback_performance = session.playback_performance.clone();
    let selection_state = session.selection_state.clone();
    let preview_focus = session.preview_focus.clone();
    let property_clipboard = session.property_clipboard.clone();
    let preferences = session.preferences.clone();
    let editor_layout = preferences::snapshot(&preferences).editor_layout;
    let audio_levels = session.audio_levels.clone();
    let audio_player = session.audio_player.clone();

    let timeline = timeline::new(
        project.clone(),
        player_state.clone(),
        playback_performance.clone(),
        selection_state.clone(),
        preferences.clone(),
        audio_levels,
        property_clipboard.clone(),
    );
    let video_player = video_player::new(
        project.clone(),
        player_state.clone(),
        playback_performance,
        selection_state.clone(),
        preview_focus.clone(),
        preferences.clone(),
        audio_player,
    );
    let inspector = inspector::new(
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        preview_focus,
        preferences.clone(),
        video_player.clone(),
        property_clipboard.clone(),
    );
    let inspector_width = editor_layout.inspector_width.round() as i32;
    let inspector_pane = panel_pane(&inspector, PanelSide::Start);
    let overlay_mode = Rc::new(Cell::new(false));
    let top = gtk::Paned::new(gtk::Orientation::Horizontal);
    top.set_wide_handle(false);
    top.set_start_child(Some(&inspector_pane.container));
    top.set_end_child(Some(&video_player));
    top.set_position(if editor_layout.inspector_visible {
        inspector_width
    } else {
        0
    });
    inspector_pane
        .container
        .set_visible(editor_layout.inspector_visible);
    top.set_resize_start_child(true);
    top.set_resize_end_child(true);
    top.set_shrink_start_child(false);
    top.set_shrink_end_child(false);

    let timeline_height =
        (editor_layout.timeline_fraction * DEFAULT_WINDOW_HEIGHT as f32).round() as i32;
    let timeline_pane = panel_pane(&timeline, PanelSide::End);
    let layout = gtk::Paned::new(gtk::Orientation::Vertical);
    layout.set_wide_handle(false);
    layout.set_end_child(Some(&timeline_pane.container));
    layout.set_position(
        ((1.0 - editor_layout.timeline_fraction) * DEFAULT_WINDOW_HEIGHT as f32).round() as i32,
    );
    timeline_pane
        .container
        .set_visible(editor_layout.timeline_visible);
    layout.set_resize_start_child(true);
    layout.set_resize_end_child(true);
    layout.set_shrink_start_child(false);
    layout.set_shrink_end_child(false);

    let (header_bar, title, connection_indicator) = header_bar();
    let inspector_toggle = gtk::ToggleButton::builder()
        .active(editor_layout.inspector_visible)
        .icon_name("dock-left-symbolic")
        .tooltip_text(tr!("Toggle Inspector").as_ref())
        .build();
    let stop_inspector = connect_panel_toggle(
        &inspector_toggle,
        &top,
        &inspector_pane,
        PanelSide::Start,
        inspector_width,
        Some(overlay_mode.clone()),
        &preferences,
    );
    let inspector_host = inspector_layout::connect(
        &top,
        inspector_pane.container.upcast_ref(),
        &inspector_toggle,
        overlay_mode,
        stop_inspector,
        &preferences,
    );
    layout.set_start_child(Some(&inspector_host));
    let timeline_toggle = gtk::ToggleButton::builder()
        .active(editor_layout.timeline_visible)
        .icon_name("dock-bottom-symbolic")
        .tooltip_text(tr!("Toggle Timeline").as_ref())
        .build();
    connect_panel_toggle(
        &timeline_toggle,
        &layout,
        &timeline_pane,
        PanelSide::End,
        timeline_height,
        None,
        &preferences,
    );
    view_preferences::connect(&preferences, &inspector_toggle, &top, PanelSide::Start);
    view_preferences::connect(&preferences, &timeline_toggle, &layout, PanelSide::End);
    let panel_toggles = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    panel_toggles.append(&inspector_toggle);
    panel_toggles.append(&timeline_toggle);
    header_bar.pack_start(&panel_toggles);
    shrimply_components_gtk::trust::handle_imports(window);
    let polling_session = session.clone();
    let polling_window = window.downgrade();
    // Frame callbacks stop when minimized; the editing API must keep processing requests.
    glib::timeout_add_local(SESSION_POLL_INTERVAL, move || {
        let Some(polling_window) = polling_window.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let update = polling_session.poll();
        connection_indicator.set_visible(update.has_collaborators);
        if let Some(error) = update.audio_playback_stopped {
            let dialog = adw::AlertDialog::new(Some("Audio playback stopped"), Some(&error));
            dialog.present(Some(&polling_window));
        }
        if let Some(next) = update.title {
            title.set_label(&next.text);
            title.set_tooltip_text(next.save_error.as_deref());
            polling_window.set_title(Some(&next.text));
        }
        glib::ControlFlow::Continue
    });

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header_bar);
    toolbar.set_content(Some(&layout));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    window.set_visible(false);
    window.set_default_size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT);
    window.set_content(Some(&toasts));

    header_menu::add(&header_bar, window, &toasts, session);
    window.present();
    video_player.grab_focus();
}

struct PanelPane {
    container: gtk::ScrolledWindow,
    child: gtk::Widget,
}

fn panel_pane(child: &gtk::Widget, side: PanelSide) -> PanelPane {
    let container = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .propagate_natural_height(false)
        .kinetic_scrolling(false)
        .child(child)
        .build();
    side.set_size_request(&container, side.rest_size_request());
    if matches!(side, PanelSide::Start) {
        container
            .hadjustment()
            .connect_changed(|adj| adj.set_value((adj.upper() - adj.page_size()).max(0.0)));
    }
    PanelPane {
        container,
        child: child.clone(),
    }
}

fn connect_panel_toggle(
    button: &gtk::ToggleButton,
    paned: &gtk::Paned,
    pane: &PanelPane,
    side: PanelSide,
    default_size: i32,
    overlay_mode: Option<Rc<Cell<bool>>>,
    preferences: &preferences::SharedPreferences,
) -> Rc<dyn Fn()> {
    let divider = shrimply_components_core::layout::PANEL_DIVIDER_WIDTH.round() as i32;
    let is_overlay = {
        let overlay_mode = overlay_mode.clone();
        move || overlay_mode.as_deref().is_some_and(Cell::get)
    };
    let finish: Rc<dyn Fn()> = {
        let weak_paned = paned.downgrade();
        let weak_pane = pane.container.downgrade();
        let weak_child = pane.child.downgrade();
        Rc::new(move || {
            let (Some(paned), Some(pane), Some(child)) = (
                weak_paned.upgrade(),
                weak_pane.upgrade(),
                weak_child.upgrade(),
            ) else {
                return;
            };
            side.set_size_request(&child, -1);
            side.set_animating_policy(&pane, false);
            side.set_size_request(&pane, side.rest_size_request());
            side.set_shrink(&paned, false);
        })
    };

    let target = adw::CallbackAnimationTarget::new({
        let weak_paned = paned.downgrade();
        let is_overlay = is_overlay.clone();
        move |size| {
            if is_overlay() {
                return;
            }
            let Some(paned) = weak_paned.upgrade() else {
                return;
            };
            let size = (size.round() as i32).max(0);
            paned.set_position(match side {
                PanelSide::Start => size,
                PanelSide::End if size == 0 => paned.height(),
                PanelSide::End => paned.height().saturating_sub(size + divider),
            });
        }
    });
    let animation = adw::TimedAnimation::new(
        paned,
        f64::from(default_size),
        f64::from(default_size),
        PANEL_ANIMATION_DURATION_MS,
        target,
    );
    animation.connect_done({
        let weak_button = button.downgrade();
        let weak_pane = pane.container.downgrade();
        let is_overlay = is_overlay.clone();
        let finish = finish.clone();
        move |_| {
            if is_overlay() {
                return;
            }
            if let (Some(button), Some(pane)) = (weak_button.upgrade(), weak_pane.upgrade())
                && !button.is_active()
            {
                pane.set_visible(false);
            }
            finish();
        }
    });

    button.connect_toggled({
        let paned = paned.clone();
        let container = pane.container.clone();
        let child = pane.child.clone();
        let animation = animation.clone();
        let preferences = preferences.clone();
        move |button| {
            if is_overlay() {
                return;
            }
            let playing = animation.state() == adw::AnimationState::Playing;
            let current = if playing {
                animation.value()
            } else if !container.is_visible() {
                0.0
            } else {
                f64::from(match side {
                    PanelSide::Start => paned.position().max(0),
                    PanelSide::End => paned.height().saturating_sub(paned.position() + divider),
                })
            };
            let target_size = if button.is_active() {
                let saved = preferences::snapshot(&preferences).editor_layout;
                let total = match side {
                    PanelSide::Start => paned.width(),
                    PanelSide::End => paned.height(),
                };
                side.set_size_request(&child, -1);
                let min_size = child
                    .measure(side.orientation(), -1)
                    .0
                    .max(side.rest_size_request());
                let sibling_min = match side {
                    PanelSide::Start => paned.end_child(),
                    PanelSide::End => paned.start_child(),
                }
                .map_or(0, |sibling| sibling.measure(side.orientation(), -1).0);
                let desired = match side {
                    PanelSide::Start => saved.inspector_width,
                    PanelSide::End => shrimply_components_core::math::split_pane_extent(
                        total as f32,
                        divider as f32,
                        saved.timeline_fraction,
                    ),
                }
                .round() as i32;
                let expanded = desired
                    .clamp(min_size, (total - sibling_min - divider).max(min_size))
                    .max(1);
                side.set_size_request(&child, expanded);
                f64::from(expanded)
            } else {
                if !playing && current > 0.0 {
                    side.set_size_request(&child, current.round() as i32);
                }
                0.0
            };
            side.set_size_request(&container, -1);
            side.set_animating_policy(&container, true);
            side.set_shrink(&paned, true);
            animation.set_value_from(current);
            animation.reset();
            if button.is_active() {
                container.set_visible(true);
            }
            animation.set_value_to(target_size);
            animation.play();
        }
    });
    Rc::new(move || {
        animation.reset();
        finish();
    })
}

fn begin_project_load(app: &adw::Application, path: PathBuf) {
    shrimply_components_gtk::icons::register_bundled();
    let flatpak = std::env::var_os("FLATPAK_ID").is_some();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr!("Shrimply").as_ref())
        .default_width(LOADING_WINDOW_WIDTH)
        .default_height(LOADING_WINDOW_HEIGHT)
        .build();
    let loading_view = if flatpak {
        project_loading_view_with_subtitle(tr!("Preparing GPU renderer…").as_ref())
    } else {
        project_loading_view(&path)
    };
    window.set_content(Some(&loading_view));
    window.present();

    if !flatpak {
        let loader = Rc::new(RefCell::new(ProjectLoader::new(path)));
        let event = loader.borrow_mut().begin();
        handle_load_event(app, &window, loader, event);
        return;
    }

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(shrimply_visual_cuda::gpu::preflight());
    });
    let app = app.clone();
    let poll_window = window.clone();
    window.add_tick_callback(move |_, _| match receiver.try_recv() {
        Ok(Ok(())) => {
            poll_window.set_content(Some(&project_loading_view(&path)));
            let loader = Rc::new(RefCell::new(ProjectLoader::new(path.clone())));
            let event = loader.borrow_mut().begin();
            handle_load_event(&app, &poll_window, loader, event);
            glib::ControlFlow::Break
        }
        Ok(Err(error)) => {
            show_project_load_error(&app, &poll_window, "Could not initialize GPU", &error);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            show_project_load_error(
                &app,
                &poll_window,
                "Could not initialize GPU",
                "GPU startup worker stopped unexpectedly",
            );
            glib::ControlFlow::Break
        }
    });
}

fn handle_load_event(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
    event: LoadEvent,
) {
    match event {
        LoadEvent::ConfirmTrust(review) => {
            let app = app.clone();
            let window = window.clone();
            shrimply_components_gtk::trust::confirm(&window.clone(), &review, move |kind| {
                let event = loader.borrow_mut().confirm_trust(kind);
                handle_load_event(&app, &window, loader, event);
            });
        }
        LoadEvent::ConfirmKdenlive => confirm_kdenlive_conversion(app, window, loader),
        LoadEvent::ChooseOtioSettings => choose_otio_settings(app, window, loader),
        LoadEvent::Progress(subtitle) => {
            window.set_content(Some(&project_loading_view_with_subtitle(subtitle)));
            let poll_app = app.clone();
            let poll_window = window.clone();
            window.add_tick_callback(move |_, _| {
                let Some(event) = loader.borrow_mut().poll() else {
                    return glib::ControlFlow::Continue;
                };
                handle_load_event(&poll_app, &poll_window, loader.clone(), event);
                glib::ControlFlow::Break
            });
        }
        LoadEvent::ConfirmFrameGridRepair => confirm_frame_grid_repair(app, window, loader),
        LoadEvent::ChooseDestination {
            title,
            suggested_name,
        } => choose_project_destination(app, window, loader, title, suggested_name),
        LoadEvent::ImportWarnings(warnings) => show_import_warnings(app, window, loader, warnings),
        LoadEvent::Ready { path, project } => {
            if let Err(error) = shrimply_recent_projects::touch(&path, &project.name, &project.tags)
            {
                tracing::warn!(%error, "could not update recent projects");
            }
            ffmpeg::init().expect("FFmpeg should initialize");
            ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
            build_ui(window, *project);
        }
        LoadEvent::Error { heading, body } => show_project_load_error(app, window, heading, &body),
        LoadEvent::Canceled => app.quit(),
    }
}

fn confirm_kdenlive_conversion(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
) {
    let message = gtk::Label::new(Some(
        tr!("Shrimply supports only some Kdenlive features. Unsupported content may be changed or omitted.")
            .as_ref(),
    ));
    message.set_justify(gtk::Justification::Center);
    message.set_wrap(true);
    let details = gtk::LinkButton::builder()
        .label(tr!("Learn more").as_ref())
        .uri(KDENLIVE_IMPORT_DOCS)
        .halign(gtk::Align::Center)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&message);
    content.append(&details);
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Convert Kdenlive Project?").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("abort", "Abort"), ("convert", "Convert")]);
    dialog.set_default_response(Some("convert"));
    dialog.set_close_response("abort");
    let callback_app = app.clone();
    let callback_window = window.clone();
    dialog.choose(
        Some(window.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |answer| {
            let event = loader.borrow_mut().confirm_kdenlive(answer == "convert");
            handle_load_event(&callback_app, &callback_window, loader, event);
        },
    );
}

fn choose_otio_settings(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
) {
    let selector = ProjectSettingsSelector::new();
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("OTIO Project Settings").as_ref())
        .body(tr!("OTIO does not include the Kdenlive project profile.").as_ref())
        .extra_child(&selector.group)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("import", "Import")]);
    dialog.set_default_response(Some("import"));
    dialog.set_close_response("cancel");
    let callback_app = app.clone();
    let callback_window = window.clone();
    dialog.choose(
        Some(window.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |answer| {
            if answer != "import" {
                let event = loader.borrow_mut().choose_otio_settings(None);
                handle_load_event(&callback_app, &callback_window, loader, event);
                return;
            }
            let Some(settings) = selector.settings() else {
                show_project_load_error(
                    &callback_app,
                    &callback_window,
                    "Could not import OTIO",
                    "The selected project settings are invalid.",
                );
                return;
            };
            let event = loader.borrow_mut().choose_otio_settings(Some(settings));
            handle_load_event(&callback_app, &callback_window, loader, event);
        },
    );
}

fn confirm_frame_grid_repair(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Project Timing Needs Repair").as_ref())
        .body(
            tr!("Some clips are not aligned to the project frame grid. Fixing them will snap and minimally shift clip boundaries, then save a new project without changing the original.")
                .as_ref(),
        )
        .build();
    dialog.add_responses_i18n(&[("abort", "Abort"), ("fix", "Fix")]);
    dialog.set_default_response(Some("fix"));
    dialog.set_close_response("abort");
    let callback_app = app.clone();
    let callback_window = window.clone();
    dialog.choose(
        Some(window.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |answer| {
            let event = loader
                .borrow_mut()
                .confirm_frame_grid_repair(answer == "fix");
            handle_load_event(&callback_app, &callback_window, loader, event);
        },
    );
}

fn choose_project_destination(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
    title: &'static str,
    suggested_name: String,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name_i18n("Shrimply projects");
    filter.add_pattern("*.shrimp");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(tr!(title).as_ref())
        .initial_name(suggested_name)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let callback_app = app.clone();
    let callback_window = window.clone();
    shrimply_components_gtk::file_picker::save(
        title,
        &dialog,
        Some(window.upcast_ref::<gtk::Window>()),
        move |result| {
            let destination = result.ok().and_then(|file| file.path());
            let event = loader.borrow_mut().choose_destination(destination);
            handle_load_event(&callback_app, &callback_window, loader, event);
        },
    );
}

fn show_import_warnings(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    loader: Rc<RefCell<ProjectLoader>>,
    warnings: Vec<String>,
) {
    let dialog = adw::AlertDialog::new(
        Some("OTIO imported with limitations"),
        Some(&warnings.join("\n")),
    );
    dialog.add_response("open", "Open Project");
    dialog.set_default_response(Some("open"));
    dialog.set_close_response("open");
    let callback_app = app.clone();
    let callback_window = window.clone();
    dialog.choose(
        Some(window.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |_| {
            let event = loader.borrow_mut().acknowledge_warnings();
            handle_load_event(&callback_app, &callback_window, loader, event);
        },
    );
}

fn show_project_load_error(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    heading: &str,
    message: &str,
) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    let app = app.clone();
    dialog.choose(
        Some(window.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |_| app.quit(),
    );
}
fn project_loading_view(path: &Path) -> adw::ToolbarView {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| path.to_string_lossy());
    project_loading_view_with_subtitle(filename.as_ref())
}

fn project_loading_view_with_subtitle(subtitle: &str) -> adw::ToolbarView {
    let bytes = glib::Bytes::from_static(include_bytes!("../../../../assets/loading-shrimp.gif"));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let animation = gdk_pixbuf::PixbufAnimation::from_stream(&stream, None::<&gio::Cancellable>)
        .expect("bundled loading animation should decode")
        .iter(Some(SystemTime::now()));
    let shrimp = gtk::DrawingArea::new();
    shrimp.set_content_width(LOADING_SHRIMP_WIDTH);
    shrimp.set_content_height(LOADING_SHRIMP_HEIGHT);
    shrimp.set_halign(gtk::Align::Center);
    shrimp.set_valign(gtk::Align::Center);
    let drawing_animation = animation.clone();
    shrimp.set_draw_func(move |_, context, width, height| {
        let frame = drawing_animation.pixbuf();
        context.scale(
            f64::from(width) / f64::from(frame.width()),
            f64::from(height) / f64::from(frame.height()),
        );
        context.set_source_pixbuf(&frame, 0.0, 0.0);
        context.source().set_filter(gtk::cairo::Filter::Nearest);
        context.paint().expect("loading animation should draw");
    });
    shrimp.add_tick_callback(move |area, _| {
        if animation.advance(SystemTime::now()) {
            area.queue_draw();
        }
        glib::ControlFlow::Continue
    });
    let status = gtk::Label::builder()
        .label(tr!("Loading project…").as_ref())
        .css_classes(["title-3"])
        .build();
    let subtitle = gtk::Label::builder()
        .label(subtitle)
        .css_classes(["dim-label"])
        .ellipsize(pango::EllipsizeMode::Middle)
        .max_width_chars(48)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_halign(gtk::Align::Center);
    body.set_valign(gtk::Align::Center);
    body.set_hexpand(true);
    body.set_vexpand(true);
    body.append(&shrimp);
    body.append(&status);
    body.append(&subtitle);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_css_class("background");
    toolbar.add_top_bar(
        &adw::HeaderBar::builder()
            .title_widget(&gtk::Box::new(gtk::Orientation::Horizontal, 0))
            .build(),
    );
    toolbar.set_content(Some(&body));
    toolbar
}

fn header_bar() -> (adw::HeaderBar, gtk::Label, gtk::Image) {
    let title = gtk::Label::builder()
        .label(tr!("Shrimply").as_ref())
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["title"])
        .build();
    let connection_indicator = gtk::Image::builder()
        .icon_name("system-users-symbolic")
        .pixel_size(16)
        .tooltip_text(tr!("Collaborators are editing this project").as_ref())
        .visible(false)
        .build();
    let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    title_box.append(&connection_indicator);
    title_box.append(&title);

    (
        adw::HeaderBar::builder().title_widget(&title_box).build(),
        title,
        connection_indicator,
    )
}
