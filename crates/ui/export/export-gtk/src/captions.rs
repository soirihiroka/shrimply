use super::*;
use shrimply_caption_export_core::{
    CaptionFormat, CaptionOutput, ExportMode, ExportSettings, prepare_export, write_export,
};

pub(super) fn open_dialog(parent: &adw::ApplicationWindow, project: Rc<RefCell<project::Project>>) {
    let format = adw::ComboRow::builder()
        .title(tr!("Format").as_ref())
        .model(&gtk::StringList::new(
            &CaptionFormat::ALL.map(CaptionFormat::label),
        ))
        .selected(0)
        .build();
    let mode = adw::ComboRow::builder()
        .title(tr!("Tracks").as_ref())
        .model(&gtk::StringList::new(&[
            tr!("Merge into one file").as_ref(),
            tr!("Export each track separately").as_ref(),
        ]))
        .selected(0)
        .build();
    shrimply_components_gtk::ui::adaptive_choice(&format);
    shrimply_components_gtk::ui::adaptive_choice(&mode);
    let options = gtk::ListBox::new();
    options.add_css_class("boxed-list");
    options.set_selection_mode(gtk::SelectionMode::None);
    options.append(&format);
    options.append(&mode);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.append(&options);
    let export = gtk::Button::with_label(tr!("Choose File").as_ref());
    export.add_css_class("suggested-action");
    export.add_css_class("pill");
    export.set_halign(gtk::Align::End);
    content.append(&export);
    let dialog = adw::Dialog::builder()
        .title(tr!("Export Captions").as_ref())
        .content_width(440)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    let dialog_parent = parent.clone();
    let parent = parent.clone();
    let close = dialog.clone();
    export.connect_clicked(move |_| {
        let settings = ExportSettings {
            format: CaptionFormat::ALL[format.selected() as usize],
            mode: if mode.selected() == 0 {
                ExportMode::Merge
            } else {
                ExportMode::Separate
            },
        };
        let project = project.borrow().clone();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(settings.format.label()));
        filter.add_suffix(settings.format.extension());
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let file_dialog = gtk::FileDialog::builder()
            .title(tr!("Export Captions").as_ref())
            .initial_name(output::default_filename(
                &project,
                settings.format.extension(),
            ))
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let result_parent = parent.clone();
        shrimply_components_gtk::file_picker::save(
            "Export Captions",
            &file_dialog,
            Some(&parent),
            move |result| {
                let file = match result {
                    Ok(file) => file,
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled) =>
                    {
                        return;
                    }
                    Err(error) => {
                        show_export_error(
                            &result_parent,
                            "Could not export captions",
                            &error.to_string(),
                        );
                        return;
                    }
                };
                let Some(path) = file.path() else {
                    show_export_error(
                        &result_parent,
                        "Could not export captions",
                        "Captions must be saved to a local file.",
                    );
                    return;
                };
                let outputs = match prepare_export(&project, &path, settings) {
                    Ok(outputs) => outputs,
                    Err(error) => {
                        show_export_error(&result_parent, "Could not export captions", &error);
                        return;
                    }
                };
                let replacements = outputs
                    .iter()
                    .filter(|output| output.path != path && output.path.exists())
                    .map(|output| output.path.display().to_string())
                    .collect::<Vec<_>>();
                if replacements.is_empty() {
                    finish_export(&result_parent, outputs);
                    return;
                }
                let confirm = adw::AlertDialog::new(
                    Some(tr!("Replace existing caption files?").as_ref()),
                    Some(&replacements.join("\n")),
                );
                confirm.add_response("cancel", tr!("Cancel").as_ref());
                confirm.add_response("replace", tr!("Replace").as_ref());
                confirm.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
                confirm.set_close_response("cancel");
                confirm.set_default_response(Some("cancel"));
                let parent = result_parent.clone();
                confirm.choose(
                    Some(&result_parent),
                    None::<&gio::Cancellable>,
                    move |response| {
                        if response == "replace" {
                            finish_export(&parent, outputs);
                        }
                    },
                );
            },
        );
        close.close();
    });
    dialog.present(Some(&dialog_parent));
}

fn finish_export(parent: &adw::ApplicationWindow, outputs: Vec<CaptionOutput>) {
    shrimply_export_core::job::run(
        Default::default(),
        |_, _| write_export(outputs),
        |event| match event {
            Event::Finished(Outcome::Complete(paths)) => {
                let title = if paths.len() == 1 {
                    tr!("Captions exported").into_owned()
                } else {
                    shrimply_components_gtk::i18n::text_args(
                        "%{count} caption files exported",
                        &[("count", paths.len().to_string())],
                    )
                };
                shrimply_components_gtk::export_feedback::show_export_finished(
                    parent, &title, &paths[0],
                );
            }
            Event::Finished(Outcome::Failed(error)) => {
                show_export_error(parent, "Could not export captions", &error)
            }
            _ => {}
        },
    );
}
