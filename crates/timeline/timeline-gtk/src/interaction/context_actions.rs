use super::*;
use crate::RenderedVideoFrame;
use shrimply_timeline_core::{
    ContextItemKind, ContextMenuControl, ItemMenuContext, TrackMenuContext, VideoFrameSelection,
};
use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nAlertDialogExt;
use shrimply_gtk_components::ui::I18nFileFilterExt;
use shrimply_gtk_components::ui::I18nMenuExt;
use shrimply_gtk_components::ui::I18nWidgetExt;

pub(crate) mod folded_items;
pub(crate) mod folded_tracks;
use folded_items::*;
use folded_tracks::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn show_timeline_item_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    x: f64,
    y: f64,
) {
    let folded_hit = {
        let project = project.borrow();
        let runtime = runtime.borrow();
        crate::folded_sequence::hit_projected_item(&project, runtime.view, x, y)
    };
    if let Some(hit) = folded_hit {
        let context = SequenceTimeline::for_item(&project.borrow(), &hit.key)
            .expect("projected item must have a valid operation scope");
        prepare_item_context_menu(&context, hit.key.clone(), project, selection_state, runtime);
        show_folded_item_context_menu(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            context,
            hit.key,
            x,
            y,
        );
        return;
    }

    let (hit, path, folder) = {
        let project_state = project.borrow();
        let runtime_state = runtime.borrow();
        let Some(hit) = hit_item_at(&project_state, runtime_state.view, x, y) else {
            drop(runtime_state);
            drop(project_state);
            show_track_context_menu(
                area,
                project,
                player_state,
                selection_state,
                runtime,
                preferences,
                x,
                y,
            );
            return;
        };
        let folder = crate::folded_sequence::reference(&project_state, hit)
            .and_then(|_| selection_state::item_address(&project_state, hit));
        (hit, item_file_path(&project_state, hit), folder)
    };

    let preserve_track_selection = {
        let project_state = project.borrow();
        let selected_items = selected_timeline_items(selection_state);
        let selected_tracks = selected_timeline_tracks(selection_state);
        let hit_track = TrackKey {
            kind: hit.kind,
            track_index: hit.track_index,
        };
        matches!(hit.kind, TrackKind::Audio)
            && selected_tracks.contains(&hit_track)
            && silence::can_remove(&project_state, &selected_items, &selected_tracks, hit)
    };
    if preserve_track_selection {
        reset_item_context_menu(runtime);
    } else {
        let project_state = project.borrow();
        let address = selection_state::item_address(&project_state, hit)
            .expect("hit-tested root item must have an address");
        drop(project_state);
        prepare_item_context_menu(
            &SequenceTimeline::root(),
            address,
            project,
            selection_state,
            runtime,
        );
    }

    let selected_items = selected_timeline_items(selection_state);
    if !preserve_track_selection {
        debug_assert!(selected_items.contains(&hit));
    }

    let property_targets = {
        let project = project.borrow();
        selection_state::selected_item_addresses(selection_state, &project)
    };
    let property_clipboard = runtime.borrow().property_clipboard.clone();
    let (can_replace_properties, can_paste_modifiers) = {
        let project = project.borrow();
        let clipboard = property_clipboard.borrow();
        (
            clipboard.can_replace_properties(&project, &property_targets),
            clipboard.can_append_modifiers(&project, &property_targets),
        )
    };
    let foldable = {
        let selected = selected_timeline_items(selection_state);
        selected.len() >= 2
            && selected
                .iter()
                .all(|key| matches!(key.kind, TrackKind::Video | TrackKind::Audio))
    };
    let unlinkable_folder = folder.as_ref().is_some_and(|_| {
        let project = project.borrow();
        item_group_id(&project, hit).is_some()
    });

    let speed_items = selected_timeline_items(selection_state)
        .iter()
        .copied()
        .filter(|key| matches!(key.kind, TrackKind::Audio | TrackKind::Video))
        .collect::<Vec<_>>();
    let speeds = {
        let project = project.borrow();
        speed_items
            .iter()
            .filter_map(|key| match key.kind {
                TrackKind::Audio => project
                    .audio_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .map(|item| playback_speed_or_default(item.playback_speed)),
                TrackKind::Video => project
                    .video_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .map(|item| playback_speed_or_default(item.playback_speed)),
                TrackKind::Caption => None,
            })
            .collect::<Vec<_>>()
    };
    let playback_speed = speeds
        .first()
        .map(|first| ContextMenuControl::PlaybackSpeed {
            position: shrimply_math_media::playback_speed_scale_position(fraction_as_f64(*first)),
            mixed: speeds.iter().any(|speed| speed != first),
        });
    let speed_control = (!speeds.is_empty()).then(|| {
        let first_speed = speeds[0];
        let mixed = speeds.iter().any(|speed| *speed != first_speed);
        let row = adw::ActionRow::builder()
            .title(tr!("Speed").as_ref())
            .build();
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -2.0, 2.0, 0.05);
        scale.set_width_request(260);
        scale.set_draw_value(false);
        for (value, label) in [
            (-2.0, Some("0.25×")),
            (-1.0, Some("0.5×")),
            (0.0, Some("1×")),
            (1.0, Some("2×")),
            (2.0, Some("4×")),
        ] {
            scale.add_mark(value, gtk::PositionType::Bottom, label);
        }
        scale.set_value(if mixed {
            0.0
        } else {
            shrimply_math_media::playback_speed_scale_position(fraction_as_f64(first_speed))
        });
        if mixed {
            scale.add_css_class("dim-label");
        }
        let scale_area = area.clone();
        let scale_project = project.clone();
        let scale_player_state = player_state.clone();
        scale.connect_value_changed(move |scale| {
            scale.remove_css_class("dim-label");
            let speed = Fraction::from(
                (shrimply_math_media::playback_speed_from_scale_position(scale.value()) * 100.0)
                    .round() as i64,
            ) / Fraction::from(100);
            let mut project = scale_project.borrow_mut();
            let mut audio = false;
            let mut video = false;
            for key in &speed_items {
                match key.kind {
                    TrackKind::Audio => {
                        if let Some(item) = project
                            .audio_tracks
                            .get_mut(key.track_index)
                            .and_then(|track| track.items.get_mut(key.item_index))
                            && item.playback_speed != speed
                        {
                            item.playback_speed = speed;
                            audio = true;
                        }
                    }
                    TrackKind::Video => {
                        if let Some(item) = project
                            .video_tracks
                            .get_mut(key.track_index)
                            .and_then(|track| track.items.get_mut(key.item_index))
                            && item.playback_speed != speed
                        {
                            item.playback_speed = speed;
                            video = true;
                        }
                    }
                    TrackKind::Caption => {}
                }
            }
            if !audio && !video {
                return;
            }
            let duration = project.duration();
            crate::project::commit_coalesced_edit(&project, "selected-item-speed");
            drop(project);
            player_state::refresh_project(
                &scale_player_state,
                ProjectChange {
                    duration: Some(duration),
                    audio,
                    audio_waveforms: audio,
                    video,
                    inspector: true,
                    ..ProjectChange::default()
                },
            );
            scale_area.queue_render();
        });
        row.add_suffix(&scale);
        row.upcast::<gtk::Widget>()
    });
    let selected_tracks = selected_timeline_tracks(selection_state);
    let enable_beat_detection = hit.kind == TrackKind::Audio
        && selected_items
            .iter()
            .filter(|key| key.kind == TrackKind::Audio)
            .any(|key| {
                project
                    .borrow()
                    .audio_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .is_some_and(|item| !item.beat_detection)
            });
    let can_remove_silences = hit.kind == TrackKind::Audio
        && silence::can_remove(&project.borrow(), &selected_items, &selected_tracks, hit);
    let contract = shrimply_timeline_core::item_context_menu(ItemMenuContext {
        kind: match hit.kind {
            TrackKind::Caption => ContextItemKind::Caption,
            TrackKind::Video => ContextItemKind::Video,
            TrackKind::Audio => ContextItemKind::Audio,
        },
        can_replace_properties,
        can_paste_modifiers,
        has_file: path.is_some(),
        foldable,
        unlinkable_folder,
        folder: folder.is_some(),
        playback_speed,
        enable_beat_detection,
        can_remove_silences,
    });
    let menu = crate::native_menu::menu_model(&contract).menu;

    let actions = gio::SimpleActionGroup::new();
    if foldable {
        add_menu_action(&actions, "fold-sequence", {
            let area = area.clone();
            let project = project.clone();
            let player_state = player_state.clone();
            let selection_state = selection_state.clone();
            move || fold_selected_timeline_items(&area, &project, &player_state, &selection_state)
        });
    }
    if unlinkable_folder {
        add_menu_action(&actions, "unlink-folder", {
            let area = area.clone();
            let project = project.clone();
            let selection_state = selection_state.clone();
            move || ungroup_selected_timeline_items(&area, &project, &selection_state)
        });
    }
    if let Some(folder) = folder {
        for (name, at_top) in [
            ("add-folder-track-top", true),
            ("add-folder-track-bottom", false),
        ] {
            add_menu_action(&actions, name, {
                let area = area.clone();
                let project = project.clone();
                let player_state = player_state.clone();
                let folder = folder.clone();
                move || create_folded_track(&area, &project, &player_state, &folder, at_top)
            });
        }
    }
    add_menu_action(&actions, "copy", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        move || {
            copy_selected_timeline_items(&area, &project, &player_state, &selection_state, &runtime)
        }
    });
    add_menu_action(&actions, "cut", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        move || {
            cut_selected_timeline_items(&area, &project, &player_state, &selection_state, &runtime);
        }
    });
    add_menu_action(&actions, "paste", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        move || {
            crate::clipboard::paste(&area, &project, &player_state, &selection_state, &runtime);
        }
    });
    add_menu_action_enabled(&actions, "replace-properties", can_replace_properties, {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        move || {
            replace_selected_item_properties(
                &area,
                &project,
                &player_state,
                &selection_state,
                &runtime,
            );
        }
    });
    add_menu_action_enabled(&actions, "paste-modifiers", can_paste_modifiers, {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        move || {
            append_selected_item_modifiers(
                &area,
                &project,
                &player_state,
                &selection_state,
                &runtime,
            );
        }
    });
    match hit.kind {
        TrackKind::Audio => add_audio_item_context_actions(
            &menu,
            &actions,
            area,
            project,
            player_state,
            selection_state,
            runtime,
            preferences,
            hit,
            false,
        ),
        TrackKind::Caption => add_caption_item_context_actions(
            &actions,
            area,
            project,
            player_state,
            selection_state,
            preferences,
        ),
        TrackKind::Video => add_video_frame_context_actions(
            &menu,
            &actions,
            area,
            project,
            player_state,
            selection_state,
            preferences,
            VideoFrameSelection::Items,
            false,
        ),
    }
    if let Some(path) = path {
        add_menu_action(&actions, "show-folder", {
            let area = area.clone();
            move || show_path_in_folder(&area, path.clone())
        });
    }

    popup_timeline_context_menu(area, runtime, &menu, &actions, speed_control.as_ref(), x, y);
    area.queue_render();
}

fn prepare_item_context_menu(
    context: &dyn TimelineOperationContext,
    address: crate::project::ItemAddress,
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let project = project.borrow();
    select_item_in_context(context, &project, selection_state, address, false, false);
    drop(project);
    reset_item_context_menu(runtime);
}

fn reset_item_context_menu(runtime: &Rc<RefCell<TimelineRuntime>>) {
    let mut runtime = runtime.borrow_mut();
    runtime.dragged_group = None;
    runtime.resize_drag = None;
    runtime.transition_drag = None;
    runtime.cut_preview = None;
    runtime.view.selection = None;
    runtime.view.drag_mode = DragMode::None;
    if let Some(existing) = runtime.active_context_menu.take() {
        existing.popdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn add_video_frame_context_actions(
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: &preferences_store::SharedPreferences,
    selection: VideoFrameSelection,
    append_menu: bool,
) {
    if append_menu {
        let section = gio::Menu::new();
        section.append_i18n("Copy Frame", "timeline.copy-frame");
        section.append_i18n("Save Frame…", "timeline.save-frame");
        menu.append_section(None, &section);
    }

    add_menu_action(actions, "copy-frame", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        move || {
            render_selected_video_frame(&project, &player_state, &selection_state, selection, {
                let area = area.clone();
                move |result| match result {
                    Ok(frame) => {
                        area.display().clipboard().set_texture(&frame.texture());
                        shrimply_gtk_components::toast::show_confirmation_for_widget(
                            &area,
                            "Frame copied",
                        );
                    }
                    Err(error) => show_error_dialog(&area, "Could not copy selected frame", &error),
                }
            });
        }
    });
    add_menu_action(actions, "save-frame", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preferences = preferences.clone();
        move || {
            let filter = gtk::FileFilter::new();
            filter.set_name_i18n("PNG image");
            filter.add_mime_type("image/png");
            filter.add_pattern("*.png");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let label = "Save Selected Frame";
            let dialog = gtk::FileDialog::builder()
                .title(tr!(label).as_ref())
                .initial_name("frame.png")
                .filters(&filters)
                .default_filter(&filter)
                .build();
            let initial_folder = preferences_store::preview_image_folder(&preferences)
                .or_else(|| glib::user_special_dir(glib::UserDirectory::Pictures));
            if let Some(folder) = initial_folder {
                dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
            let area = area.clone();
            let project = project.clone();
            let player_state = player_state.clone();
            let selection_state = selection_state.clone();
            let preferences = preferences.clone();
            shrimply_gtk_components::file_picker::save(
                label,
                &dialog,
                area.root().and_downcast::<gtk::Window>().as_ref(),
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(mut path) = file.path() else {
                        show_error_dialog(
                            &area,
                            "Could not save selected frame",
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
                        preferences_store::set_preview_image_folder(&preferences, folder);
                    }
                    render_selected_video_frame(
                        &project,
                        &player_state,
                        &selection_state,
                        selection,
                        {
                            let area = area.clone();
                            move |result| match result {
                                Ok(frame) => {
                                    if let Err(error) = frame.texture().save_to_png(&path) {
                                        show_error_dialog(
                                            &area,
                                            "Could not save selected frame",
                                            &error.to_string(),
                                        );
                                    }
                                }
                                Err(error) => show_error_dialog(
                                    &area,
                                    "Could not save selected frame",
                                    &error,
                                ),
                            }
                        },
                    );
                },
            );
        }
    });
}

impl RenderedVideoFrame {
    fn texture(self) -> gdk::Texture {
        let stride = self.width as usize * std::mem::size_of::<u32>();
        gdk::MemoryTexture::new(
            self.width,
            self.height,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(self.pixels),
            stride,
        )
        .upcast()
    }
}

fn render_selected_video_frame(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    selection: VideoFrameSelection,
    done: impl FnOnce(Result<RenderedVideoFrame, String>) + 'static,
) {
    let (project, position, item_ids) = crate::toolkit_context_menu::prepare_selected_video_frame(
        project,
        player_state,
        selection_state,
        selection,
    );
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = crate::toolkit_context_menu::render_video_frame(project, position, &item_ids);
        let _ = tx.send(result);
    });

    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
        Ok(result) => {
            done.take().expect("frame callback must run once")(result);
            glib::ControlFlow::Break
        }
        Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(TryRecvError::Disconnected) => {
            done.take().expect("frame callback must run once")(Err(
                "The frame renderer stopped unexpectedly.".to_string(),
            ));
            glib::ControlFlow::Break
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn add_audio_item_context_actions(
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    hit: ItemKey,
    append_menu: bool,
) {
    let selected_items = selected_timeline_items(selection_state);
    let selected_tracks = selected_timeline_tracks(selection_state);
    let selected_audio = selected_items
        .iter()
        .copied()
        .filter(|key| key.kind == TrackKind::Audio)
        .collect::<Vec<_>>();
    let enable_beat_detection = selected_audio.iter().any(|key| {
        project
            .borrow()
            .audio_tracks
            .get(key.track_index)
            .and_then(|track| track.items.get(key.item_index))
            .is_some_and(|item| !item.beat_detection)
    });
    let can_remove_silences =
        silence::can_remove(&project.borrow(), &selected_items, &selected_tracks, hit);
    if append_menu {
        let section = gio::Menu::new();
        section.append_i18n(
            if enable_beat_detection {
                "Enable Beat Detection"
            } else {
                "Disable Beat Detection"
            },
            "timeline.beat-detection",
        );
        section.append_i18n("Export Audio…", "timeline.export-audio");
        section.append_i18n("Transcribe", "timeline.transcribe");
        if can_remove_silences {
            section.append_i18n("Remove Silences", "timeline.remove-silences");
        }
        menu.append_section(None, &section);
    }
    add_menu_action(actions, "beat-detection", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        move || {
            crate::toolkit_context_menu::set_selected_audio_beat_detection(
                &project,
                &player_state,
                &selection_state,
                enable_beat_detection,
            );
            area.queue_render();
        }
    });
    add_export_audio_action(actions, area, project, selection_state);
    add_menu_action(actions, "transcribe", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preferences = preferences.clone();
        move || {
            show_transcribe_dialog(
                &area,
                &project,
                &player_state,
                &selection_state,
                &preferences,
            )
        }
    });
    if can_remove_silences {
        add_menu_action(actions, "remove-silences", {
            let area = area.clone();
            let project = project.clone();
            let player_state = player_state.clone();
            let selection_state = selection_state.clone();
            let runtime = runtime.clone();
            move || silence::show_dialog(&area, &project, &player_state, &selection_state, &runtime)
        });
    }
}

fn add_export_audio_action(
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    add_menu_action(actions, "export-audio", {
        let area = area.clone();
        let project = project.clone();
        let selection_state = selection_state.clone();
        move || show_export_audio_dialog(&area, &project, &selection_state)
    });
}

fn show_export_audio_dialog(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    let selection = {
        let project = project.borrow();
        selected_audio_project(
            &project,
            &selected_timeline_items(selection_state),
            &selected_timeline_tracks(selection_state),
        )
    };
    let Some(mut selection) = selection else {
        show_error_dialog(area, "Could not export audio", "No audio is selected.");
        return;
    };
    for track in &mut selection.project.audio_tracks {
        for item in &mut track.items {
            item.start = item.start.saturating_sub(selection.start);
            item.end = item.end.saturating_sub(selection.start);
        }
    }

    let formats = gtk::StringList::new(&["WAV", "FLAC", "MP3", "OGG Vorbis", "Opus"]);
    let format_row = adw::ComboRow::builder()
        .title(tr!("Format").as_ref())
        .model(&formats)
        .selected(0)
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&format_row);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);
    content.append(&group);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Export Selected Audio").as_ref())
        .body(tr!("The selected items will be mixed into one audio file.").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("continue", "Choose File")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("continue"));
    dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    let area = area.clone();
    dialog.clone().choose(
        Some(area.clone().upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() != "continue" {
                return;
            }
            let format = export::audio::Format::from_index(format_row.selected());
            let label = "Export Selected Audio";
            let file_dialog = gtk::FileDialog::builder()
                .title(tr!(label).as_ref())
                .initial_name(format!("selected-audio.{}", format.extension()))
                .build();
            let area = area.clone();
            let export_project = selection.project.clone();
            shrimply_gtk_components::file_picker::save(
                label,
                &file_dialog,
                area.root().and_downcast::<gtk::Window>().as_ref(),
                move |result| {
                    let Some(file) = result.ok() else {
                        return;
                    };
                    let Some(mut path) = file.path() else {
                        show_error_dialog(
                            &area,
                            "Could not export audio",
                            "Could not resolve the selected file path.",
                        );
                        return;
                    };
                    if !path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()))
                    {
                        path.set_extension(format.extension());
                    }
                    enum AudioExportEvent {
                        Progress(export::audio::ExportProgress),
                        Finished(Result<(), String>),
                    }

                    let progress_dialog = adw::Dialog::builder()
                        .title(tr!("Exporting Audio").as_ref())
                        .content_width(460)
                        .build();
                    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
                    content.set_margin_top(24);
                    content.set_margin_bottom(24);
                    content.set_margin_start(24);
                    content.set_margin_end(24);
                    let progress_bar = gtk::ProgressBar::new();
                    progress_bar.set_show_text(true);
                    progress_bar.set_text(Some(tr!("Preparing audio").as_ref()));
                    progress_bar.pulse();
                    let state_label = gtk::Label::new(Some(tr!("Preparing audio").as_ref()));
                    state_label.set_halign(gtk::Align::Center);
                    state_label.set_wrap(true);
                    content.append(&progress_bar);
                    content.append(&state_label);
                    let toolbar = adw::ToolbarView::new();
                    toolbar.add_top_bar(&adw::HeaderBar::new());
                    toolbar.set_content(Some(&content));
                    progress_dialog.set_child(Some(&toolbar));
                    progress_dialog.present(Some(area.upcast_ref::<gtk::Widget>()));

                    let cancelled = Arc::new(AtomicBool::new(false));
                    progress_dialog.connect_closed({
                        let cancelled = cancelled.clone();
                        move |_| cancelled.store(true, Ordering::Relaxed)
                    });
                    let (tx, rx) = mpsc::channel();
                    let exported_path = path.clone();
                    let worker_cancelled = cancelled.clone();
                    thread::spawn(move || {
                        let progress_tx = tx.clone();
                        let result = export::audio::export_with_progress(
                            &export_project,
                            &path,
                            format,
                            move |progress| {
                                let _ = progress_tx.send(AudioExportEvent::Progress(progress));
                                !worker_cancelled.load(Ordering::Relaxed)
                            },
                        );
                        let _ = tx.send(AudioExportEvent::Finished(result));
                    });
                    let area_for_result = area.clone();
                    glib::timeout_add_local(Duration::from_millis(100), move || {
                        let mut finished = None;
                        loop {
                            match rx.try_recv() {
                                Ok(AudioExportEvent::Progress(progress)) => {
                                    let (label, completed_frames, total_frames) = match progress {
                                        export::audio::ExportProgress::Mixing {
                                            completed_frames,
                                            total_frames,
                                        } => ("Preparing audio", completed_frames, total_frames),
                                        export::audio::ExportProgress::Encoding {
                                            completed_frames,
                                            total_frames,
                                        } => ("Encoding audio", completed_frames, total_frames),
                                    };
                                    let fraction = if total_frames == 0 {
                                        1.0
                                    } else {
                                        completed_frames as f64 / total_frames as f64
                                    }
                                    .clamp(0.0, 1.0);
                                    state_label.set_label(tr!(label).as_ref());
                                    progress_bar.set_fraction(fraction);
                                    let progress_text = match progress {
                                        export::audio::ExportProgress::Mixing { .. } => {
                                            "Preparing audio (%{percent}%)"
                                        }
                                        export::audio::ExportProgress::Encoding { .. } => {
                                            "Encoding audio (%{percent}%)"
                                        }
                                    };
                                    progress_bar.set_text(Some(
                                        &shrimply_gtk_components::i18n::text_args(
                                            progress_text,
                                            &[("percent", format!("{:.0}", fraction * 100.0))],
                                        ),
                                    ));
                                }
                                Ok(AudioExportEvent::Finished(result)) => {
                                    finished = Some(result);
                                    break;
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    finished = Some(Err(
                                        "The export worker stopped unexpectedly.".to_string()
                                    ));
                                    break;
                                }
                            }
                        }
                        match finished {
                            Some(Ok(())) => {
                                progress_dialog.close();
                                export::show_export_finished_for_widget(
                                    &area_for_result,
                                    "Audio exported",
                                    &exported_path,
                                );
                                glib::ControlFlow::Break
                            }
                            Some(Err(error)) => {
                                let was_cancelled = cancelled.load(Ordering::Relaxed);
                                progress_dialog.close();
                                if !was_cancelled {
                                    show_error_dialog(
                                        &area_for_result,
                                        "Could not export audio",
                                        &error,
                                    );
                                }
                                glib::ControlFlow::Break
                            }
                            None => glib::ControlFlow::Continue,
                        }
                    });
                },
            );
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn show_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    x: f64,
    y: f64,
) {
    let (track_row, key, empty_space_above, empty_space_below) = {
        let project_state = project.borrow();
        let runtime_state = runtime.borrow();
        let rows = crate::items::track_rows(&project_state);
        let row = (y >= RULER_HEIGHT).then(|| {
            ((y + runtime_state.view.scroll_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize
        });
        let track_row = ((0.0..timeline_x()).contains(&x))
            .then_some(row)
            .flatten()
            .and_then(|row| rows.get(row).cloned());
        let key =
            track_label_action_at(&project_state, runtime_state.view, x, y).map(|(key, _)| key);
        let over_track_handles = (0.0..timeline_x()).contains(&x);
        let empty_space_above = over_track_handles && (0.0..RULER_HEIGHT).contains(&y);
        let empty_space_below = over_track_handles
            && y >= RULER_HEIGHT
            && y + runtime_state.view.scroll_y >= RULER_HEIGHT + rows.len() as f64 * TRACK_HEIGHT;
        (track_row, key, empty_space_above, empty_space_below)
    };
    if let Some(track_row) = track_row.filter(|row| row.root_key.is_none()) {
        show_folded_track_context_menu(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            track_row.address,
            x,
            y,
        );
        return;
    }
    let Some(key) = key else {
        if empty_space_above || empty_space_below {
            show_new_track_context_menu(
                area,
                project,
                player_state,
                selection_state,
                runtime,
                empty_space_above,
                x,
                y,
            );
        }
        return;
    };

    match key.kind {
        TrackKind::Audio => show_audio_track_context_menu(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            preferences,
            key,
            x,
            y,
        ),
        TrackKind::Caption => show_caption_track_context_menu(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            preferences,
            key,
            x,
            y,
        ),
        TrackKind::Video => show_video_track_context_menu(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            preferences,
            key,
            x,
            y,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn show_video_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    key: TrackKey,
    x: f64,
    y: f64,
) {
    prepare_track_context_menu(runtime, selection_state, key);
    let menu = crate::native_menu::menu_model(&shrimply_timeline_core::track_context_menu(
        TrackMenuContext::Video,
    ))
    .menu;
    let actions = gio::SimpleActionGroup::new();
    add_video_frame_context_actions(
        &menu,
        &actions,
        area,
        project,
        player_state,
        selection_state,
        preferences,
        VideoFrameSelection::Tracks,
        false,
    );
    popup_timeline_context_menu(area, runtime, &menu, &actions, None, x, y);
    area.queue_render();
}

#[allow(clippy::too_many_arguments)]
fn show_new_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    at_top: bool,
    x: f64,
    y: f64,
) {
    prepare_empty_track_context_menu(project, runtime, selection_state);
    let menu =
        crate::native_menu::menu_model(&shrimply_timeline_core::empty_track_context_menu()).menu;
    let actions = gio::SimpleActionGroup::new();
    for (name, kind) in [
        ("add-caption-track", TrackKind::Caption),
        ("add-video-track", TrackKind::Video),
        ("add-audio-track", TrackKind::Audio),
    ] {
        let index = match (kind, at_top) {
            (TrackKind::Caption | TrackKind::Video, true) | (TrackKind::Audio, false) => None,
            (TrackKind::Caption | TrackKind::Video, false) | (TrackKind::Audio, true) => Some(0),
        };
        add_create_track_action(
            &actions,
            name,
            area,
            project,
            player_state,
            selection_state,
            kind,
            index,
        );
    }
    popup_timeline_context_menu(area, runtime, &menu, &actions, None, x, y);
    area.queue_render();
}

#[allow(clippy::too_many_arguments)]
fn show_audio_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    key: TrackKey,
    x: f64,
    y: f64,
) {
    let can_remove_silences = silence::can_remove_track(
        &project.borrow(),
        &selected_timeline_tracks(selection_state),
        key,
    );
    prepare_track_context_menu(runtime, selection_state, key);
    let menu = crate::native_menu::menu_model(&shrimply_timeline_core::track_context_menu(
        TrackMenuContext::Audio {
            can_remove_silences,
            gain_db: project.borrow().audio_tracks[key.track_index].gain_db,
        },
    ))
    .menu;
    let actions = gio::SimpleActionGroup::new();
    add_export_audio_action(&actions, area, project, selection_state);
    add_menu_action(&actions, "transcribe", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preferences = preferences.clone();
        move || {
            show_transcribe_dialog(
                &area,
                &project,
                &player_state,
                &selection_state,
                &preferences,
            )
        }
    });
    if can_remove_silences {
        add_menu_action(&actions, "remove-silences", {
            let area = area.clone();
            let project = project.clone();
            let player_state = player_state.clone();
            let selection_state = selection_state.clone();
            let runtime = runtime.clone();
            move || silence::show_dialog(&area, &project, &player_state, &selection_state, &runtime)
        });
    }
    let gain_row = adw::ActionRow::builder()
        .title(tr!("Gain Offset").as_ref())
        .build();
    let gain = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(AUDIO_TRACK_GAIN_MIN_DB),
        f64::from(AUDIO_TRACK_GAIN_MAX_DB),
        0.5,
    );
    gain.set_width_request(260);
    gain.set_draw_value(false);
    gain.add_mark(0.0, gtk::PositionType::Bottom, None);
    gain.set_value(f64::from(
        project.borrow().audio_tracks[key.track_index].gain_db,
    ));
    gain.set_tooltip_i18n("Track gain offset in decibels");
    let gain_area = area.clone();
    let gain_project = project.clone();
    let gain_player_state = player_state.clone();
    gain.connect_value_changed(move |gain| {
        let gain_db = gain.value() as f32;
        let mut project = gain_project.borrow_mut();
        let track = &mut project.audio_tracks[key.track_index];
        if track.gain_db == gain_db {
            return;
        }
        track.gain_db = gain_db;
        crate::project::commit_coalesced_edit(&project, "audio-track-gain");
        drop(project);
        player_state::refresh_project(
            &gain_player_state,
            ProjectChange {
                audio: true,
                ..ProjectChange::default()
            },
        );
        gain_area.queue_render();
    });
    gain_row.add_suffix(&gain);
    popup_timeline_context_menu(
        area,
        runtime,
        &menu,
        &actions,
        Some(&gain_row.upcast()),
        x,
        y,
    );
    area.queue_render();
}

#[allow(clippy::too_many_arguments)]
fn show_caption_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    key: TrackKey,
    x: f64,
    y: f64,
) {
    prepare_track_context_menu(runtime, selection_state, key);
    let menu = crate::native_menu::menu_model(&shrimply_timeline_core::track_context_menu(
        TrackMenuContext::Caption,
    ))
    .menu;
    let actions = gio::SimpleActionGroup::new();
    add_menu_action(&actions, "generate-speech", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preferences = preferences.clone();
        move || {
            let url = preferences_store::snapshot(&preferences).compute_server_url;
            if url.is_empty() {
                show_error_dialog(
                    &area,
                    "Compute server is not configured",
                    "Set the Server URL in Preferences before generating speech.",
                );
                return;
            }
            let jobs = caption_tts::jobs_for_track(&project.borrow(), key.track_index);
            caption_tts::show_dialog(
                &area,
                &project,
                &player_state,
                &selection_state,
                preferences.clone(),
                jobs,
            );
        }
    });
    popup_timeline_context_menu(area, runtime, &menu, &actions, None, x, y);
    area.queue_render();
}

#[allow(clippy::too_many_arguments)]
fn add_create_track_action(
    actions: &gio::SimpleActionGroup,
    name: &str,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    kind: TrackKind,
    index: Option<usize>,
) {
    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    add_menu_action(actions, name, move || {
        create_track(
            &area,
            &project,
            &player_state,
            &selection_state,
            kind,
            index,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn create_track(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    kind: TrackKind,
    index: Option<usize>,
) {
    create_track_core(project, player_state, selection_state, kind, index);
    area.queue_render();
}

pub(crate) fn create_track_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    kind: TrackKind,
    index: Option<usize>,
) {
    let index = {
        let mut project_state = project.borrow_mut();
        let track_count = match kind {
            TrackKind::Caption => project_state.caption_tracks.len(),
            TrackKind::Video => project_state.video_tracks.len(),
            TrackKind::Audio => project_state.audio_tracks.len(),
        };
        let index = index.unwrap_or(track_count);
        if index > track_count {
            return;
        }
        match kind {
            TrackKind::Caption => project_state
                .caption_tracks
                .insert(index, Default::default()),
            TrackKind::Video => project_state.video_tracks.insert(index, Default::default()),
            TrackKind::Audio => project_state.audio_tracks.insert(index, Default::default()),
        }
        crate::project::commit_edit(&project_state, "create-timeline-track");
        index
    };

    select_track(
        selection_state,
        TrackKey {
            kind,
            track_index: index,
        },
        false,
        false,
    );
    player_state::refresh_project(
        player_state,
        ProjectChange {
            audio: kind == TrackKind::Audio,
            video: kind == TrackKind::Video,
            captions: kind == TrackKind::Caption,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

fn prepare_empty_track_context_menu(
    project: &Rc<RefCell<Project>>,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    selection_state: &SharedSelectionState,
) {
    set_timeline_selection(&project.borrow(), selection_state, Vec::new(), None);
    let mut runtime = runtime.borrow_mut();
    runtime.dragged_group = None;
    runtime.resize_drag = None;
    runtime.transition_drag = None;
    runtime.cut_preview = None;
    runtime.view.selection = None;
    runtime.view.drag_mode = DragMode::None;
    if let Some(existing) = runtime.active_context_menu.take() {
        existing.popdown();
    }
}

fn prepare_track_context_menu(
    runtime: &Rc<RefCell<TimelineRuntime>>,
    selection_state: &SharedSelectionState,
    key: TrackKey,
) {
    let mut runtime = runtime.borrow_mut();
    runtime.dragged_group = None;
    runtime.resize_drag = None;
    runtime.transition_drag = None;
    runtime.cut_preview = None;
    runtime.view.selection = None;
    runtime.view.drag_mode = DragMode::None;
    if !selected_timeline_tracks(selection_state).contains(&key) {
        select_track(selection_state, key, false, false);
    }
    if let Some(existing) = runtime.active_context_menu.take() {
        existing.popdown();
    }
}

fn popup_timeline_context_menu(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    custom_child: Option<&gtk::Widget>,
    x: f64,
    y: f64,
) {
    let popover = context_menu::popup(area, menu, actions, custom_child, x, y);
    runtime.borrow_mut().active_context_menu = Some(popover.upcast());
}

pub(super) fn add_menu_action<F>(group: &gio::SimpleActionGroup, name: &str, activate: F)
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    group.add_action(&action);
}

fn add_menu_action_enabled<F>(
    group: &gio::SimpleActionGroup,
    name: &str,
    enabled: bool,
    activate: F,
) where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.set_enabled(enabled);
    action.connect_activate(move |_, _| activate());
    group.add_action(&action);
}

pub(super) fn copy_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    if crate::toolkit_context_menu::copy_selected_timeline_items_core(
        project,
        player_state,
        selection_state,
        runtime,
    ) {
        area.display()
            .clipboard()
            .set_text(crate::clipboard::TIMELINE_MARKER);
    }
}

pub(super) fn cut_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    copy_selected_timeline_items(area, project, player_state, selection_state, runtime);
    if runtime.borrow().clipboard.is_some() {
        delete_selected_addressed_items(area, project, player_state, selection_state, false);
    }
}

pub(crate) fn item_file_path(project: &Project, key: ItemKey) -> Option<PathBuf> {
    match key.kind {
        TrackKind::Caption => None,
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .and_then(|item| {
                (item.is_media()
                    || matches!(item.content, crate::project::VideoItemContent::Manim(_)))
                .then(|| item.file.path().to_path_buf())
            }),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .and_then(|item| {
                (matches!(
                    &item.source,
                    crate::project::AudioSource::Media | crate::project::AudioSource::Tts(_)
                ) && !item.file.as_os_str().is_empty())
                .then(|| item.file.path().to_path_buf())
            }),
    }
}

fn show_path_in_folder(area: &gtk::GLArea, path: PathBuf) {
    if let Err(error) = desktop_open::show_path_in_folder(area.upcast_ref(), path) {
        show_error_dialog(area, "Could not show file", &error);
    }
}
