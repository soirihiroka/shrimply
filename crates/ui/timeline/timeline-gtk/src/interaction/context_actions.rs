use super::*;
use crate::RenderedVideoFrame;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nAlertDialogExt;
use shrimply_components_gtk::ui::I18nFileFilterExt;
use shrimply_export_core::job::{Event, Job, Outcome};
use shrimply_timeline_skia::{ContextMenuControl, ContextMenuRequest, VideoFrameSelection};
use std::sync::atomic::Ordering;

pub(crate) fn show_timeline_item_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    (point, touch): (Vec2, bool),
) {
    let existing = runtime.borrow_mut().active_context_menu.take();
    if let Some(existing) = existing {
        existing.close();
    }
    let contract = runtime
        .borrow_mut()
        .scene
        .prepare_context_menu_for_input(point, touch);
    if contract.sections.is_empty() {
        return;
    }
    let menu = crate::native_menu::menu_model(&contract);
    let actions = gio::SimpleActionGroup::new();
    for item in contract.actions() {
        let area = area.downgrade();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = Rc::downgrade(runtime);
        let preferences = preferences.clone();
        let action =
            shrimply_components_gtk::ui::menu_action(item.action.id(), item.checked, move || {
                let (Some(runtime), Some(area)) = (runtime.upgrade(), area.upgrade()) else {
                    return None;
                };
                let result = runtime
                    .borrow_mut()
                    .scene
                    .activate_context_menu_action(item.action);
                let checked = runtime.borrow().scene.context_menu_checked(item.action);
                handle_action_result(
                    &area,
                    &project,
                    &player_state,
                    &selection_state,
                    &runtime,
                    &preferences,
                    result,
                );
                checked
            });
        action.set_enabled(item.enabled);
        actions.add_action(&action);
    }
    let controls = contract
        .sections
        .iter()
        .flatten()
        .filter_map(|entry| match entry {
            crate::ContextMenuEntry::Control(control) => Some(*control),
            _ => None,
        });
    let custom_children = controls
        .map(|control| {
            let row = adw::ActionRow::builder()
                .title(tr!(control.label()).as_ref())
                .build();
            let scale = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                control.minimum(),
                control.maximum(),
                control.step(),
            );
            scale.set_width_request(260);
            scale.set_draw_value(false);
            if matches!(control, ContextMenuControl::PlaybackSpeed { .. }) {
                for (value, label) in [
                    (-2.0, "0.25×"),
                    (-1.0, "0.5×"),
                    (0.0, "1×"),
                    (1.0, "2×"),
                    (2.0, "4×"),
                ] {
                    scale.add_mark(value, gtk::PositionType::Bottom, Some(label));
                }
            }
            scale.set_value(control.value());
            if control.mixed() {
                scale.add_css_class("dim-label");
            }
            let area = area.downgrade();
            let runtime = Rc::downgrade(runtime);
            scale.connect_value_changed(move |scale| {
                let (Some(runtime), Some(area)) = (runtime.upgrade(), area.upgrade()) else {
                    return;
                };
                scale.remove_css_class("dim-label");
                let result = runtime
                    .borrow_mut()
                    .scene
                    .set_context_menu_control(control, scale.value());
                if let Err(error) = result {
                    show_error_dialog(&area, "Timeline edit failed", &error);
                }
                area.queue_render();
            });
            row.add_suffix(&scale);
            row.upcast::<gtk::Widget>()
        })
        .collect::<Vec<_>>();
    let presented = if shrimply_components_gtk::layout::menu_uses_bottom_sheet(if touch {
        "touch"
    } else {
        "pointer"
    }) {
        context_menu::sheet(area, &menu, &actions, &custom_children)
    } else {
        context_menu::PresentedMenu::Popover(
            context_menu::popup(area, &menu, &actions, &custom_children, point).upcast(),
        )
    };
    runtime.borrow_mut().active_context_menu = Some(presented);
}

pub(super) fn handle_action_result(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
    result: Result<Option<ContextMenuRequest>, String>,
) {
    use ContextMenuRequest as R;
    match result {
        Ok(Some(R::SetTimelineClipboardMarker)) => area
            .display()
            .clipboard()
            .set_text(crate::clipboard::TIMELINE_MARKER),
        Ok(Some(R::PasteFromClipboard)) => crate::clipboard::paste(area, runtime),
        Ok(Some(R::CopyFrame(selection) | R::SaveFrame(selection))) => {
            let actions = gio::SimpleActionGroup::new();
            add_video_frame_context_actions(
                &actions,
                area,
                project,
                player_state,
                selection_state,
                preferences,
                selection,
            );
            let action = if matches!(result, Ok(Some(R::CopyFrame(_)))) {
                "copy-frame"
            } else {
                "save-frame"
            };
            actions.activate_action(action, None);
        }
        Ok(Some(R::ShowInFolder)) => {
            if let Some(path) = runtime.borrow().scene.context_file_path() {
                show_path_in_folder(area, path);
            }
        }
        Ok(Some(R::ExportAudio)) => show_export_audio_dialog(area, project, selection_state),
        Ok(Some(R::Transcribe)) => show_transcribe_dialog(area, runtime, preferences),
        Ok(Some(R::RemoveSilences)) => silence::show_dialog(area, runtime),
        Ok(Some(R::GenerateSpeech)) => {
            let actions = gio::SimpleActionGroup::new();
            add_caption_item_context_actions(&actions, area, runtime, preferences);
            actions.activate_action("generate-speech", None);
        }
        Ok(Some(R::DeleteFoldedTrack { clip_count } | R::DeleteTracks { clip_count })) => {
            let deletion = runtime.borrow().scene.track_deletion();
            let dialog = adw::AlertDialog::new(
                Some("Delete Tracks?"),
                Some(&format!(
                    "{clip_count} clips are about to be deleted, are you sure?"
                )),
            );
            dialog.add_responses_i18n(&[("cancel", "Cancel"), ("delete", "Delete")]);
            dialog.set_close_response("cancel");
            dialog.set_default_response(Some("delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let runtime = Rc::downgrade(runtime);
            let render_area = area.clone();
            dialog.choose(Some(area), None::<&gio::Cancellable>, move |response| {
                if response == "delete"
                    && let Some(runtime) = runtime.upgrade()
                {
                    let result = runtime
                        .borrow_mut()
                        .scene
                        .confirm_delete_selected_tracks(deletion);
                    if let Err(error) = result {
                        show_error_dialog(&render_area, "Timeline edit failed", &error);
                    }
                    render_area.queue_render();
                }
            });
        }
        Ok(None) => {}
        Err(error) => show_error_dialog(area, "Timeline edit failed", &error),
    }
    area.queue_render();
}

fn add_video_frame_context_actions(
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: &preferences_store::SharedPreferences,
    selection: VideoFrameSelection,
) {
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
                        area.display()
                            .clipboard()
                            .set_texture(&frame_texture(frame));
                        shrimply_components_gtk::toast::show_confirmation_for_widget(
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
            shrimply_components_gtk::file_picker::save(
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
                                    if let Err(error) = frame_texture(frame).save_to_png(&path) {
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

fn frame_texture(frame: RenderedVideoFrame) -> gdk::Texture {
    let stride = frame.width as usize * std::mem::size_of::<u32>();
    gdk::MemoryTexture::new(
        frame.width,
        frame.height,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(frame.pixels),
        stride,
    )
    .upcast()
}

fn render_selected_video_frame(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    selection: VideoFrameSelection,
    done: impl FnOnce(Result<RenderedVideoFrame, String>) + 'static,
) {
    let (project, position, item_ids) =
        shrimply_timeline_skia::video_selection::prepare_selected_video_frame(
            project,
            player_state,
            selection_state,
            selection,
        );
    let mut job = match Job::spawn(
        move |cancelled, _| {
            shrimply_visual_cuda::compositor::render_items_rgba(
                project, position, &item_ids, &cancelled,
            )
        },
        || {},
    ) {
        Ok(job) => job,
        Err(error) => {
            done(Err(error));
            return;
        }
    };
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let outcome = job.poll().find_map(|event| match event {
            Event::Finished(outcome) => Some(outcome),
            _ => None,
        });
        let result = match outcome {
            Some(Outcome::Complete(frame)) => Ok(frame),
            Some(Outcome::Failed(error)) => Err(error),
            Some(Outcome::Cancelled) => Err("Frame capture cancelled".into()),
            None => return glib::ControlFlow::Continue,
        };
        done.take().expect("frame callback must run once")(result);
        glib::ControlFlow::Break
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
    let Some(selection) = selection else {
        show_error_dialog(area, "Could not export audio", "No audio is selected.");
        return;
    };
    let export_project = selection.at_zero();

    let formats =
        gtk::StringList::new(&export::audio::Format::ALL.map(export::audio::Format::label));
    let format_row = adw::ComboRow::builder()
        .title(tr!("Format").as_ref())
        .model(&formats)
        .selected(0)
        .build();
    shrimply_components_gtk::ui::adaptive_choice(&format_row);
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
            let export_project = export_project.clone();
            shrimply_components_gtk::file_picker::save(
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
                    let exported_path = path.clone();
                    let job = Job::spawn(
                        move |cancelled, progress| {
                            export::audio::export_with_progress(
                                &export_project,
                                &path,
                                format,
                                |update| {
                                    progress(update);
                                    !cancelled.load(Ordering::Acquire)
                                },
                            )
                        },
                        || {},
                    );
                    let dialog_parent = area.clone();
                    export::present_export_job(
                        &area,
                        "Exporting Audio",
                        "Preparing audio",
                        job,
                        move |outcome| match outcome {
                            Outcome::Complete(()) => export::show_export_finished_for_widget(
                                &dialog_parent,
                                "Audio exported",
                                &exported_path,
                            ),
                            Outcome::Failed(error) => {
                                show_error_dialog(&dialog_parent, "Could not export audio", &error)
                            }
                            Outcome::Cancelled => {}
                        },
                    );
                },
            );
        },
    );
}

pub(super) fn add_menu_action<F>(group: &gio::SimpleActionGroup, name: &str, activate: F)
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    group.add_action(&action);
}

fn show_path_in_folder(area: &gtk::GLArea, path: PathBuf) {
    if let Err(error) = desktop_open::show_path_in_folder(area.upcast_ref(), path) {
        show_error_dialog(area, "Could not show file", &error);
    }
}
