use crate::ui::{Number2Picker, SingleLineTextInput, dropdown};
use adw::prelude::*;
use shrimply_components_gtk::{tr, ui::I18nAlertDialogExt};
use shrimply_inspector_core::project::{MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION, presentation};
use shrimply_project_document::project::Project;
use std::{cell::Cell, rc::Rc};

use super::{Inspectable, InspectorContext, item::flat, list, section::InspectorSection};

impl Inspectable for Project {
    fn title(&self) -> &'static str {
        "Project"
    }

    fn add_rows(&self, _section: &InspectorSection, _context: &InspectorContext) {}

    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let project = presentation(self);
        let details = project.details();
        let config = InspectorSection::controls();

        let name_controller = context.inspector_core.clone();
        let name_commit_controller = context.inspector_core.clone();
        let name = SingleLineTextInput::builder(&project.name)
            .on_change(move |next| {
                name_controller.set_project_name(&next);
            })
            .on_commit(move |_| name_commit_controller.finish_live_edit())
            .build();
        config.add_control_row("Name", &name);
        let tags_controller = context.inspector_core.clone();
        config.add_wide_control(&shrimply_components_gtk::ui::tag_editor(
            project.tags,
            move |tags| tags_controller.set_project_tags(tags),
        ));

        let initial_fps = project.frame_rate;
        let initial_width = project.canvas_size.width;
        let initial_height = project.canvas_size.height;
        let staged_fps = Rc::new(Cell::new(initial_fps));
        let staged_width = Rc::new(Cell::new(initial_width));
        let staged_height = Rc::new(Cell::new(initial_height));

        let discard = gtk::Button::with_label(tr!("Discard").as_ref());
        let apply = gtk::Button::with_label(tr!("Apply").as_ref());
        apply.add_css_class("suggested-action");
        let actions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::End)
            .build();
        actions.append(&discard);
        actions.append(&apply);
        actions.set_visible(false);

        let update_actions: Rc<dyn Fn()> = {
            let actions = actions.clone();
            let staged_fps = staged_fps.clone();
            let staged_width = staged_width.clone();
            let staged_height = staged_height.clone();
            Rc::new(move || {
                actions.set_visible(
                    staged_fps.get() != initial_fps
                        || staged_width.get() != initial_width
                        || staged_height.get() != initial_height,
                );
            })
        };

        let mut fps_options = shrimply_project_document::project::COMMON_FRAME_RATES
            .iter()
            .map(|rate| (rate.value, rate.label.to_string()))
            .collect::<Vec<_>>();
        if fps_options
            .iter()
            .all(|(fps, _)| *fps != project.frame_rate)
        {
            fps_options.push((
                project.frame_rate,
                shrimply_project_document::project::fraction_as_label(project.frame_rate),
            ));
        }
        let fps = dropdown(project.frame_rate, fps_options, {
            let staged_fps = staged_fps.clone();
            let update_actions = update_actions.clone();
            move |next| {
                staged_fps.set(next);
                update_actions();
            }
        });
        config.add_control_row("FPS", &fps);

        let resolution = Number2Picker::builder(
            f64::from(project.canvas_size.width),
            f64::from(project.canvas_size.height),
        )
        .minimum(f64::from(MIN_CANVAS_DIMENSION))
        .maximum(f64::from(MAX_CANVAS_DIMENSION))
        .drag_step(1.0)
        .digits(0)
        .width_chars(7)
        .first_prefix("W")
        .second_prefix("H")
        .enable_lock()
        .on_first_change({
            let staged_width = staged_width.clone();
            let update_actions = update_actions.clone();
            move |next| {
                staged_width.set(next.round() as u32);
                update_actions();
            }
        })
        .on_second_change({
            let staged_height = staged_height.clone();
            let update_actions = update_actions.clone();
            move |next| {
                staged_height.set(next.round() as u32);
                update_actions();
            }
        })
        .build_with_handles();
        config.add_control_row("Resolution", &resolution.widget);
        config.add_wide_control(&actions);

        let refresh_inspector = context.refresh.clone();
        discard.connect_clicked(move |_| refresh_inspector());

        let apply_controller = context.inspector_core.clone();
        apply.connect_clicked(move |button| {
            let dialog = adw::AlertDialog::new(
                Some(tr!("Change Project Settings?").as_ref()),
                Some(
                    tr!("Changing the frame rate or resolution can affect timing, visual layout, and rendered output. Existing media and effects may no longer match the project.")
                        .as_ref(),
                ),
            );
            dialog.add_responses_i18n(&[("cancel", "Cancel"), ("apply", "Apply")]);
            dialog.set_close_response("cancel");
            dialog.set_default_response(Some("cancel"));
            dialog.set_response_appearance("apply", adw::ResponseAppearance::Destructive);

            let controller = apply_controller.clone();
            let next_fps = staged_fps.get();
            let next_width = staged_width.get();
            let next_height = staged_height.get();
            dialog.choose(
                Some(button.upcast_ref::<gtk::Widget>()),
                None::<&gtk::gio::Cancellable>,
                move |response| {
                    if response.as_str() != "apply" {
                        return;
                    }
                    controller
                        .apply_project_settings(
                            shrimply_project_document::project::CanvasSize {
                                width: next_width,
                                height: next_height,
                            },
                            next_fps,
                        )
                        .expect("validated project settings must apply");
                },
            );
        });

        let info = adw::PreferencesGroup::new();
        for detail in details
            .into_iter()
            .filter(|detail| detail.label != "Project File")
        {
            info.add(
                &adw::ActionRow::builder()
                    .title(tr!(detail.label).as_ref())
                    .subtitle(detail.value)
                    .build(),
            );
        }

        let project_path = project.file;
        let file = adw::ActionRow::builder()
            .title(tr!("Project File").as_ref())
            .subtitle(project_path.to_string_lossy())
            .build();
        let show_file = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(tr!("Show project file in folder").as_ref())
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let reveal_path = project_path.clone();
        show_file.connect_clicked(move |button| {
            if let Err(error) =
                crate::desktop_open::show_path_in_folder(button.upcast_ref(), reveal_path.clone())
            {
                let dialog =
                    adw::AlertDialog::new(Some("Could not show project file"), Some(&error));
                dialog.add_response("close", tr!("Close").as_ref());
                dialog.present(Some(button));
            }
        });
        file.add_suffix(&show_file);
        file.set_activatable_widget(Some(&show_file));
        info.add(&file);

        list::render_categories(
            vec![
                list::InspectorCategory {
                    key: "config",
                    label: "Project",
                    icon: "sliders-horizontal-symbolic",
                    items: vec![flat(config.into_widget())],
                },
                list::InspectorCategory {
                    key: "info",
                    label: "Info",
                    icon: "info-outline-symbolic",
                    items: vec![flat(info)],
                },
                list::InspectorCategory {
                    key: "performance",
                    label: "Performance",
                    icon: "speedometer-symbolic",
                    items: vec![flat(super::benchmarking::widget())],
                },
            ],
            context,
        )
    }
}
