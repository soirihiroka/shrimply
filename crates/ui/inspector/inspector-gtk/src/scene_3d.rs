use adw::prelude::AdwDialogExt;
use gtk::prelude::*;
use shrimply_components_gtk::{tr, ui::I18nFileFilterExt};
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorControlAction, InspectorTarget, NumberMapping,
    NumberSpec, VideoCard, scene_3d as shared,
};
use shrimply_project_document::project::{
    Project, Time, VideoItemContent, generated_item_keyframe_span,
};
use shrimply_property_model::timeline_value::TimelineValue;
use shrimply_scene_3d::{Camera3d, Environment3d, ObjScene, PbrMaterial};

use crate::{
    InspectedItem as SelectedItem, InspectorContext,
    item::{DefaultInspectorItem, InspectorListItem},
    player_state::ProjectChange,
    section::InspectorSection,
    timeline_value::{
        color::{ColorAccess, ColorTarget, color_control},
        scalar::{ScalarAccess, ScalarClamp, ScalarSpec, ScalarTarget, scalar_control},
        vector::vec3::{Vec3Target, control as vec3_control},
    },
};

pub(super) fn items(scene: &ObjScene, context: &InspectorContext) -> Vec<InspectorListItem> {
    let cards = shared_cards(scene, context);
    vec![
        DefaultInspectorItem::new(
            cards[0].key,
            cards[0].title,
            scene.camera.clone(),
            camera_controls,
            |context, _: Camera3d| reset(context, 0),
        )
        .boxed(),
        DefaultInspectorItem::new(
            cards[1].key,
            cards[1].title,
            scene.material.clone(),
            render_controls,
            |context, _: PbrMaterial| reset(context, 1),
        )
        .boxed(),
        DefaultInspectorItem::new(
            cards[2].key,
            cards[2].title,
            scene.environment.clone(),
            environment_controls,
            |context, _: Environment3d| reset(context, 2),
        )
        .boxed(),
    ]
}

fn shared_cards(scene: &ObjScene, context: &InspectorContext) -> [VideoCard; 3] {
    let key = context
        .selected_item
        .clone()
        .expect("scene 3D inspector requires a selected item");
    let server_url =
        shrimply_editor_state::preferences::snapshot(&context.preferences).compute_server_url;
    let models = shrimply_inspector_core::camera_source::cached_tracking_models(&server_url);
    shared::cards(
        &context.project.borrow(),
        &key,
        scene,
        context.inspector_core.snapshot().runtime,
        models.as_ref(),
    )
}

fn selected_scene(context: &InspectorContext) -> ObjScene {
    let key = context
        .selected_item
        .clone()
        .expect("scene 3D inspector requires a selected item");
    let project = context.project.borrow();
    let VideoItemContent::Obj(scene) = &project
        .video_item(&key)
        .expect("selected scene 3D item must remain available")
        .content
    else {
        panic!("selected scene 3D item changed content type")
    };
    scene.as_ref().clone()
}

fn camera_controls(_: &Camera3d, context: &InspectorContext) -> Vec<gtk::Widget> {
    let scene = selected_scene(context);
    let card = shared_cards(&scene, context)[0].clone();
    let section = InspectorSection::controls();
    crate::camera_source::add_controls(&section, &scene.camera.source, context);
    for control in card.section.controls {
        if !control
            .path
            .starts_with(shrimply_inspector_core::camera_source::SOURCE_PATH)
        {
            section.add_wide_control(&scene_control(&scene, &control, context));
        }
    }
    vec![section.into_widget()]
}

fn render_controls(_: &PbrMaterial, context: &InspectorContext) -> Vec<gtk::Widget> {
    controls(context, 1)
}
fn environment_controls(_: &Environment3d, context: &InspectorContext) -> Vec<gtk::Widget> {
    controls(context, 2)
}

fn controls(context: &InspectorContext, index: usize) -> Vec<gtk::Widget> {
    let scene = selected_scene(context);
    let card = shared_cards(&scene, context)[index].clone();
    let section = InspectorSection::controls();
    let mut controls = card.section.controls.into_iter().peekable();
    while let Some(control) = controls.next() {
        if control.action == Some(InspectorControlAction::SelectScene3dEnvironment) {
            let clear = controls
                .next()
                .expect("shared environment image controls must include clear");
            section.add_wide_control(&environment_picker(&control, &clear, context));
        } else {
            section.add_wide_control(&scene_control(&scene, &control, context));
        }
    }
    vec![section.into_widget()]
}

fn scene_control(
    scene: &ObjScene,
    control: &InspectorControl,
    context: &InspectorContext,
) -> gtk::Widget {
    match control.kind {
        ControlKind::Selector => dropdown(control, context),
        ControlKind::LayeredNumber => {
            let id = control
                .timeline_id
                .expect("shared scene scalar must identify its timeline");
            scene_scalar(
                control,
                shared::number(scene, id).expect("shared scene scalar must resolve by stable ID"),
                context,
            )
        }
        ControlKind::LayeredVector3 => {
            let id = control
                .timeline_id
                .expect("shared scene vector must identify its timeline");
            scene_vector(
                control,
                shared::vector3(scene, id).expect("shared scene vector must resolve by stable ID"),
                context,
            )
        }
        ControlKind::LayeredColor => {
            let id = control
                .timeline_id
                .expect("shared scene color must identify its timeline");
            scene_color(
                control,
                shared::color(scene, id).expect("shared scene color must resolve by stable ID"),
                context,
            )
        }
        kind => panic!("unsupported shared scene 3D control: {kind:?}"),
    }
}

fn dropdown(control: &InspectorControl, context: &InspectorContext) -> gtk::Widget {
    assert_eq!(control.values.len(), control.labels.len());
    let target = InspectorTarget::Item(
        context
            .selected_item
            .clone()
            .expect("scene 3D selector requires a selected item"),
    );
    let controller = context.inspector_core.clone();
    let path = control.path.clone();
    let commit = control.commit_name.clone();
    let immediate = control.commit_immediately;
    crate::selector::labeled_string_selector(
        &control.label,
        &control.value,
        control
            .values
            .iter()
            .cloned()
            .zip(control.labels.iter().cloned())
            .map(|(value, label)| crate::selector::StringChoice { value, label })
            .collect(),
        move |value| {
            if let Err(error) =
                controller.set_video_field(&target, &path, &value, &commit, immediate)
            {
                tracing::error!(%error, "Could not update GTK scene 3D selector");
            }
        },
    )
    .widget()
    .clone()
}

fn scene_scalar(
    control: &InspectorControl,
    value: &TimelineValue<f32>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(control.commit_name, shared::SCALAR_COMMIT);
    let mapping = control.number_mapping;
    scalar_control(
        &control.label,
        value,
        context,
        ScalarTarget {
            access: ScalarAccess::Scene3dScoped { value_id: value.id },
            scope_id: Some(value.id),
            local_time: crate::video::visual_local_time,
            duration: scene_duration,
            refresh: video_change(),
            commit_name: shared::SCALAR_COMMIT,
        },
        ScalarSpec {
            drag_step: control.number.drag_step,
            digits: usize::try_from(control.number.digits)
                .expect("scene 3D scalar digits must be nonnegative"),
            integer: control.integer,
            width_chars: control.width_characters,
            minimum: optional_minimum(&control.number),
            maximum: optional_maximum(&control.number),
            unit_name: (!control.number.unit.is_empty()).then_some(control.number.unit),
            rotating_icon: None,
            display: match mapping {
                NumberMapping::Linear => linear_display,
                NumberMapping::FocalLengthMillimeters => focal_display,
            },
            store: match mapping {
                NumberMapping::Linear => linear_store,
                NumberMapping::FocalLengthMillimeters => focal_store,
            },
            clamp: ScalarClamp::Number(control.number_constraint),
        },
    )
}

fn scene_vector(
    control: &InspectorControl,
    value: &TimelineValue<glam::Vec3>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(control.commit_name, shared::VECTOR_COMMIT);
    let mut target = Vec3Target::scene_builder(value.id);
    if control.number.unit == "°" {
        target = target.degrees();
    }
    let target = target.build().presentation(
        control,
        shared::VECTOR_COMMIT,
        shared::VECTOR_EXPRESSION_COMMIT,
    );
    vec3_control(&control.label, value, context, target)
}

fn scene_color(
    control: &InspectorControl,
    value: &TimelineValue<shrimply_property_model::Color<u8>>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(control.commit_name, shared::COLOR_COMMIT);
    color_control(
        &control.label,
        value,
        context,
        ColorTarget {
            access: ColorAccess::Scene3dScoped { value_id: value.id },
            scope_id: Some(value.id),
            local_time: crate::video::visual_local_time,
            duration: scene_duration,
            refresh: video_change(),
            commit_name: shared::COLOR_COMMIT,
        },
    )
}

fn environment_picker(
    select: &InspectorControl,
    clear: &InspectorControl,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(
        select.action,
        Some(InspectorControlAction::SelectScene3dEnvironment)
    );
    assert_eq!(
        clear.action,
        Some(InspectorControlAction::ClearScene3dEnvironment)
    );
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let choose = gtk::Button::with_label(&select.value);
    let clear_button = gtk::Button::with_label(&clear.value);
    clear_button.set_sensitive(clear.sensitive);
    row.append(&choose);
    row.append(&clear_button);
    let target = InspectorTarget::Item(
        context
            .selected_item
            .clone()
            .expect("scene environment picker requires a selected item"),
    );
    let choose_controller = context.inspector_core.clone();
    let choose_target = target.clone();
    choose.connect_clicked(move |_| {
        let label = "Select environment image";
        let filter = gtk::FileFilter::new();
        filter.set_name_i18n("Environment images");
        for pattern in [
            "*.png", "*.jpg", "*.jpeg", "*.webp", "*.avif", "*.hdr", "*.exr",
        ] {
            filter.add_pattern(pattern);
        }
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr!(label).as_ref())
            .filters(&filters)
            .build();
        let controller = choose_controller.clone();
        let target = choose_target.clone();
        shrimply_components_gtk::file_picker::open(
            label,
            &dialog,
            None::<&gtk::Window>,
            move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                if let Err(error) = controller.set_scene_3d_environment(&target, &path) {
                    adw::AlertDialog::new(Some("Could not use environment image"), Some(&error))
                        .present(None::<&gtk::Widget>);
                }
            },
        );
    });
    let clear_controller = context.inspector_core.clone();
    clear_button.connect_clicked(move |_| {
        if let Err(error) = clear_controller.clear_scene_3d_environment(&target) {
            tracing::error!(%error, "Could not clear GTK scene environment image");
        }
    });
    crate::ui::control_row(&select.label, &row)
}

fn reset(context: &InspectorContext, index: usize) {
    let scene = selected_scene(context);
    let reset = shared_cards(&scene, context)[index]
        .reset
        .clone()
        .expect("shared scene 3D card must have reset behavior");
    let target = InspectorTarget::Item(
        context
            .selected_item
            .clone()
            .expect("scene 3D reset requires a selected item"),
    );
    if let Err(error) = context.inspector_core.reset_video(&target, &reset) {
        tracing::error!(%error, "Could not reset GTK scene 3D inspector card");
    }
}

fn optional_minimum(number: &NumberSpec) -> Option<f64> {
    (number.minimum != NumberSpec::default().minimum).then_some(number.minimum)
}
fn optional_maximum(number: &NumberSpec) -> Option<f64> {
    (number.maximum != NumberSpec::default().maximum).then_some(number.maximum)
}
fn linear_display(value: f32) -> f64 {
    f64::from(value)
}
fn linear_store(value: f64) -> f32 {
    value as f32
}
fn focal_display(value: f32) -> f64 {
    NumberMapping::FocalLengthMillimeters.display(f64::from(value), 1.0)
}
fn focal_store(value: f64) -> f32 {
    NumberMapping::FocalLengthMillimeters.store(value, 1.0) as f32
}
fn scene_duration(project: &Project, key: SelectedItem) -> Option<Time> {
    let item = project.video_item(&key)?;
    generated_item_keyframe_span(item)
        .map(|(start, end)| end.saturating_sub(start))
        .or_else(|| crate::video::visual_duration(project, key))
}

fn video_change() -> ProjectChange {
    ProjectChange {
        video: true,
        ..ProjectChange::default()
    }
}
