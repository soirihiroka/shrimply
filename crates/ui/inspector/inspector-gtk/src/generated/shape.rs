use crate::InspectedItem as SelectedItem;
use crate::player_state::ProjectChange;
use crate::timeline_value::*;
use shrimply_inspector_core::generated::shape::{
    COLOR_EDIT_COMMIT, INTEGER_EDIT_COMMIT, INTEGER_EXPRESSION_COMMIT, INTEGER_KEYFRAME_COMMIT,
    ROUNDING_EDIT_COMMIT, SCALAR_EDIT_COMMIT, STEP_EDIT_COMMIT, VECTOR_EDIT_COMMIT,
};
use shrimply_inspector_core::{ControlKind, InspectorControl};
use shrimply_project_document::project::{
    Project, ShapeItem, ShapeKind, ShapeRoundingStrategy, Time, VideoItem, VideoItemContent,
    generated_item_keyframe_span,
};

use crate::{
    InspectorContext,
    item::{DefaultInspectorItem, InspectorListItem},
    section::InspectorSection,
    timeline_value::color::{ColorTarget, color_control},
    timeline_value::scalar::{ScalarSpec, ScalarTarget},
    timeline_value::vector::vec2::{VecSpec, VecTarget, vec_control},
};

pub(crate) fn shape_items(shape: &ShapeItem, context: &InspectorContext) -> Vec<InspectorListItem> {
    let cards = shrimply_inspector_core::generated::shape::cards(
        shape,
        context.project.borrow().canvas_size,
        context.inspector_core.snapshot().runtime,
    );
    vec![
        DefaultInspectorItem::new_with_default(
            cards[0].key,
            cards[0].title,
            shape.clone(),
            shape_content_controls,
            default_shape,
            |context, value: ShapeItem| reset_shape_card(context, &value, 0),
        )
        .preview_facet(
            cards[0]
                .preview_facet
                .expect("shared shape content card must have a preview facet"),
        )
        .boxed(),
        DefaultInspectorItem::new_with_default(
            cards[1].key,
            cards[1].title,
            shape.clone(),
            shape_appearance_controls,
            default_shape,
            |context, value: ShapeItem| reset_shape_card(context, &value, 1),
        )
        .preview_facet(
            cards[1]
                .preview_facet
                .expect("shared shape appearance card must have a preview facet"),
        )
        .boxed(),
    ]
}

fn reset_shape_card(context: &InspectorContext, shape: &ShapeItem, index: usize) {
    let Some(key) = context.selected_item.clone() else {
        return;
    };
    let reset = shrimply_inspector_core::generated::shape::cards(
        shape,
        context.project.borrow().canvas_size,
        context.inspector_core.snapshot().runtime,
    )[index]
        .reset
        .clone()
        .expect("shared shape card must have reset behavior");
    if let Err(error) = context
        .inspector_core
        .reset_video(&shrimply_inspector_core::InspectorTarget::Item(key), &reset)
    {
        tracing::error!(%error, "Could not reset GTK shape inspector card");
    }
}

fn shape_content_controls(value: &ShapeItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    shared_controls(value, context, 0)
}

fn shape_appearance_controls(value: &ShapeItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    shared_controls(value, context, 1)
}

fn shared_controls(
    shape: &ShapeItem,
    context: &InspectorContext,
    card_index: usize,
) -> Vec<gtk::Widget> {
    let card = shrimply_inspector_core::generated::shape::cards(
        shape,
        context.project.borrow().canvas_size,
        context.inspector_core.snapshot().runtime,
    )[card_index]
        .clone();
    let section = InspectorSection::controls();
    for control in card.section.controls {
        section.add_wide_control(&shape_control(shape, &control, context));
    }
    vec![section.into_widget()]
}

fn shape_control(
    shape: &ShapeItem,
    control: &InspectorControl,
    context: &InspectorContext,
) -> gtk::Widget {
    match control.path.as_str() {
        "/content/shape" => shape_dropdown(control, &shape.shape, context),
        "/content/size" => size_control(control, &shape.size, context),
        "/content/star_points" => integer_control(control, &shape.star_points, context),
        "/content/star_inner_radius_percent" => scalar_control(
            control,
            &shape.star_inner_radius_percent,
            context,
            ShapeField::StarInnerRadiusPercent,
        ),
        "/content/arrow_shaft_width_percent" => scalar_control(
            control,
            &shape.arrow_shaft_width_percent,
            context,
            ShapeField::ArrowShaftWidthPercent,
        ),
        "/content/arrow_head_length_percent" => scalar_control(
            control,
            &shape.arrow_head_length_percent,
            context,
            ShapeField::ArrowHeadLengthPercent,
        ),
        "/content/cross_arm_thickness_percent" => scalar_control(
            control,
            &shape.cross_arm_thickness_percent,
            context,
            ShapeField::CrossArmThicknessPercent,
        ),
        "/content/ellipse_inner_radius_percent" => scalar_control(
            control,
            &shape.ellipse_inner_radius_percent,
            context,
            ShapeField::EllipseInnerRadiusPercent,
        ),
        "/content/ellipse_completion_degrees" => scalar_control(
            control,
            &shape.ellipse_completion_degrees,
            context,
            ShapeField::EllipseCompletionDegrees,
        ),
        "/content/fill" => shape_color_control(control, &shape.fill, context, shape_fill),
        "/content/outline_color" => {
            shape_color_control(control, &shape.outline_color, context, shape_outline_color)
        }
        "/content/outline_width" => scalar_control(
            control,
            &shape.outline_width,
            context,
            ShapeField::OutlineWidth,
        ),
        "/content/corner_radius" => scalar_control(
            control,
            &shape.corner_radius,
            context,
            ShapeField::CornerRadius,
        ),
        "/content/rounding_strategy" => {
            rounding_strategy_dropdown(control, &shape.rounding_strategy, context)
        }
        "/content/shadow_color" => {
            shape_color_control(control, &shape.shadow_color, context, shape_shadow_color)
        }
        "/content/shadow_distance" => scalar_control(
            control,
            &shape.shadow_distance,
            context,
            ShapeField::ShadowDistance,
        ),
        "/content/shadow_direction_degrees" => scalar_control(
            control,
            &shape.shadow_direction_degrees,
            context,
            ShapeField::ShadowDirectionDegrees,
        ),
        "/content/shadow_width" => scalar_control(
            control,
            &shape.shadow_width,
            context,
            ShapeField::ShadowWidth,
        ),
        "/content/shadow_blur" => {
            scalar_control(control, &shape.shadow_blur, context, ShapeField::ShadowBlur)
        }
        path => panic!("unsupported shared shape control: {path}"),
    }
}

fn default_shape(context: &InspectorContext) -> ShapeItem {
    default_shape_for_canvas(context.project.borrow().canvas_size)
}

fn default_shape_for_canvas(
    canvas_size: shrimply_project_document::project::CanvasSize,
) -> ShapeItem {
    let VideoItemContent::Shape(shape) =
        VideoItem::shape_item(canvas_size, Time::ZERO, Time::ZERO).content
    else {
        unreachable!()
    };
    *shape
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ShapeField {
    #[expect(dead_code, reason = "the shared visual transform owns rotation")]
    RotationDegrees,
    StarInnerRadiusPercent,
    ArrowShaftWidthPercent,
    ArrowHeadLengthPercent,
    CrossArmThicknessPercent,
    EllipseInnerRadiusPercent,
    EllipseCompletionDegrees,
    OutlineWidth,
    CornerRadius,
    ShadowDistance,
    ShadowDirectionDegrees,
    ShadowWidth,
    ShadowBlur,
}

fn shape_dropdown(
    control: &InspectorControl,
    value: &TimelineValue<ShapeKind>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_timeline_control(
        control,
        ControlKind::LayeredSelector,
        value.id,
        STEP_EDIT_COMMIT,
    );
    let timeline_id = value.id;
    crate::timeline_value::step::step_control(
        &control.label,
        value,
        context,
        crate::timeline_value::step::StepTarget::new(
            move |project, key| {
                let VideoItemContent::Shape(shape) = &project.video_item(&key)?.content else {
                    return None;
                };
                (shape.shape.id == timeline_id).then_some(&shape.shape)
            },
            move |project, key| {
                let VideoItemContent::Shape(shape) = &mut project.video_item_mut(&key)?.content
                else {
                    return None;
                };
                (shape.shape.id == timeline_id).then_some(&mut shape.shape)
            },
            STEP_EDIT_COMMIT,
            ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        ),
    )
}

fn rounding_strategy_dropdown(
    control: &InspectorControl,
    value: &TimelineValue<ShapeRoundingStrategy>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_timeline_control(
        control,
        ControlKind::LayeredSelector,
        value.id,
        ROUNDING_EDIT_COMMIT,
    );
    let timeline_id = value.id;
    crate::timeline_value::step::step_control(
        &control.label,
        value,
        context,
        crate::timeline_value::step::StepTarget::new(
            move |project, key| {
                let VideoItemContent::Shape(shape) = &project.video_item(&key)?.content else {
                    return None;
                };
                (shape.rounding_strategy.id == timeline_id).then_some(&shape.rounding_strategy)
            },
            move |project, key| {
                let VideoItemContent::Shape(shape) = &mut project.video_item_mut(&key)?.content
                else {
                    return None;
                };
                (shape.rounding_strategy.id == timeline_id).then_some(&mut shape.rounding_strategy)
            },
            ROUNDING_EDIT_COMMIT,
            ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        ),
    )
}

fn scalar_control(
    control: &InspectorControl,
    value: &TimelineValue<f32>,
    context: &InspectorContext,
    field: ShapeField,
) -> gtk::Widget {
    assert_timeline_control(
        control,
        ControlKind::LayeredNumber,
        value.id,
        SCALAR_EDIT_COMMIT,
    );
    crate::timeline_value::scalar::scalar_control(
        &control.label,
        value,
        context,
        shape_scalar_target(field, value.id),
        shape_scalar_spec(control, field),
    )
}

fn integer_control(
    control: &InspectorControl,
    value: &TimelineValue<u32>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(control.kind, ControlKind::LayeredNumber);
    assert_eq!(control.timeline_id, Some(value.id));
    assert_eq!(control.commit_name, INTEGER_EDIT_COMMIT);
    assert_eq!(control.keyframe_commit_name, INTEGER_KEYFRAME_COMMIT);
    assert_eq!(control.expression_commit_name, INTEGER_EXPRESSION_COMMIT);
    assert!(!control.commit_immediately);
    assert!(control.integer);
    let section = InspectorSection::controls();
    crate::control::add_control(&section, context, &context.list_target, control.clone());
    section.into_widget()
}

fn size_control(
    control: &InspectorControl,
    value: &TimelineValue<glam::Vec2>,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_timeline_control(
        control,
        ControlKind::LayeredVector2,
        value.id,
        VECTOR_EDIT_COMMIT,
    );
    assert_eq!(control.prefixes, ["W", "H"]);
    assert!(!control.lock);
    vec_control(
        &control.label,
        value,
        context,
        vec_target(value.id, shape_size, shape_size_mut),
        VecSpec {
            first_prefix: "W",
            second_prefix: "H",
            drag_step: control.number.drag_step,
            digits: usize::try_from(control.number.digits)
                .expect("shape vector digits must be non-negative"),
            width_chars: control.width_characters,
            minimum: Some(control.number.minimum),
            maximum: None,
            unit_name: control.number.unit,
        },
    )
}

fn vec_target(
    value_id: uuid::Uuid,
    get: fn(&Project, SelectedItem) -> Option<&TimelineValue<glam::Vec2>>,
    get_mut: fn(&mut Project, SelectedItem) -> Option<&mut TimelineValue<glam::Vec2>>,
) -> VecTarget {
    VecTarget {
        access: crate::timeline_value::vector::vec2::VecAccess::ItemScoped {
            get,
            get_mut,
            value_id,
        },
        scope_id: Some(value_id),
        local_time: video_local_time_for_key,
        duration: video_duration_for_key,
        refresh: ProjectChange {
            video: true,
            inspector: true,
            ..ProjectChange::default()
        },
        commit_name: VECTOR_EDIT_COMMIT,
    }
}

fn shape_scalar_target(field: ShapeField, value_id: uuid::Uuid) -> ScalarTarget {
    ScalarTarget {
        access: crate::timeline_value::scalar::ScalarAccess::ItemScoped {
            get: match field {
                ShapeField::RotationDegrees => shape_rotation_degrees,
                ShapeField::StarInnerRadiusPercent => shape_star_inner_radius_percent,
                ShapeField::ArrowShaftWidthPercent => shape_arrow_shaft_width_percent,
                ShapeField::ArrowHeadLengthPercent => shape_arrow_head_length_percent,
                ShapeField::CrossArmThicknessPercent => shape_cross_arm_thickness_percent,
                ShapeField::EllipseInnerRadiusPercent => shape_ellipse_inner_radius_percent,
                ShapeField::EllipseCompletionDegrees => shape_ellipse_completion_degrees,
                ShapeField::OutlineWidth => shape_outline_width,
                ShapeField::CornerRadius => shape_corner_radius,
                ShapeField::ShadowDistance => shape_shadow_distance,
                ShapeField::ShadowDirectionDegrees => shape_shadow_direction_degrees,
                ShapeField::ShadowWidth => shape_shadow_width,
                ShapeField::ShadowBlur => shape_shadow_blur,
            },
            get_mut: match field {
                ShapeField::RotationDegrees => shape_rotation_degrees_mut,
                ShapeField::StarInnerRadiusPercent => shape_star_inner_radius_percent_mut,
                ShapeField::ArrowShaftWidthPercent => shape_arrow_shaft_width_percent_mut,
                ShapeField::ArrowHeadLengthPercent => shape_arrow_head_length_percent_mut,
                ShapeField::CrossArmThicknessPercent => shape_cross_arm_thickness_percent_mut,
                ShapeField::EllipseInnerRadiusPercent => shape_ellipse_inner_radius_percent_mut,
                ShapeField::EllipseCompletionDegrees => shape_ellipse_completion_degrees_mut,
                ShapeField::OutlineWidth => shape_outline_width_mut,
                ShapeField::CornerRadius => shape_corner_radius_mut,
                ShapeField::ShadowDistance => shape_shadow_distance_mut,
                ShapeField::ShadowDirectionDegrees => shape_shadow_direction_degrees_mut,
                ShapeField::ShadowWidth => shape_shadow_width_mut,
                ShapeField::ShadowBlur => shape_shadow_blur_mut,
            },
            value_id,
        },
        scope_id: Some(value_id),
        local_time: video_local_time_for_key,
        duration: video_duration_for_key,
        refresh: ProjectChange {
            video: true,
            inspector: true,
            ..ProjectChange::default()
        },
        commit_name: SCALAR_EDIT_COMMIT,
    }
}

fn shape_scalar_spec(control: &InspectorControl, field: ShapeField) -> ScalarSpec {
    ScalarSpec {
        drag_step: control.number.drag_step,
        digits: usize::try_from(control.number.digits)
            .expect("shape scalar digits must be non-negative"),
        integer: false,
        width_chars: control.width_characters,
        minimum: match field {
            ShapeField::RotationDegrees | ShapeField::ShadowDirectionDegrees => None,
            ShapeField::StarInnerRadiusPercent
            | ShapeField::ArrowShaftWidthPercent
            | ShapeField::ArrowHeadLengthPercent
            | ShapeField::CrossArmThicknessPercent => Some(control.number.minimum),
            ShapeField::EllipseInnerRadiusPercent | ShapeField::EllipseCompletionDegrees => {
                Some(control.number.minimum)
            }
            _ => Some(control.number.minimum),
        },
        maximum: match field {
            ShapeField::StarInnerRadiusPercent
            | ShapeField::ArrowShaftWidthPercent
            | ShapeField::ArrowHeadLengthPercent
            | ShapeField::CrossArmThicknessPercent => Some(control.number.maximum),
            ShapeField::EllipseInnerRadiusPercent => Some(control.number.maximum),
            ShapeField::EllipseCompletionDegrees => Some(control.number.maximum),
            _ => None,
        },
        unit_name: Some(control.number.unit),
        rotating_icon: match field {
            ShapeField::RotationDegrees => Some(("arrow3-up-symbolic", 0.0)),
            ShapeField::ShadowDirectionDegrees => Some(("arrow3-up-symbolic", 90.0)),
            _ => None,
        },
        display: |value| value as f64,
        store: |value| value as f32,
        clamp: crate::timeline_value::scalar::ScalarClamp::Function(match field {
            ShapeField::StarInnerRadiusPercent
            | ShapeField::ArrowShaftWidthPercent
            | ShapeField::ArrowHeadLengthPercent
            | ShapeField::CrossArmThicknessPercent => |value| value.clamp(5.0, 95.0),
            ShapeField::EllipseInnerRadiusPercent => |value| value.clamp(0.0, 95.0),
            ShapeField::EllipseCompletionDegrees => |value| value.clamp(0.0, 360.0),
            ShapeField::OutlineWidth
            | ShapeField::CornerRadius
            | ShapeField::ShadowDistance
            | ShapeField::ShadowWidth
            | ShapeField::ShadowBlur => |value| value.max(0.0),
            ShapeField::RotationDegrees | ShapeField::ShadowDirectionDegrees => |value| value,
        }),
    }
}

fn shape_color_control(
    control: &InspectorControl,
    value: &TimelineValue<shrimply_property_model::Color<u8>>,
    context: &InspectorContext,
    get_mut: fn(
        &mut Project,
        SelectedItem,
    ) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>>,
) -> gtk::Widget {
    assert_timeline_control(
        control,
        ControlKind::LayeredColor,
        value.id,
        COLOR_EDIT_COMMIT,
    );
    color_control(
        &control.label,
        value,
        context,
        ColorTarget {
            access: crate::timeline_value::color::ColorAccess::ItemScoped {
                get_mut,
                value_id: value.id,
            },
            scope_id: Some(value.id),
            local_time: video_local_time_for_key,
            duration: video_duration_for_key,
            refresh: ProjectChange {
                video: true,
                inspector: true,
                ..ProjectChange::default()
            },
            commit_name: COLOR_EDIT_COMMIT,
        },
    )
}

fn assert_timeline_control(
    control: &InspectorControl,
    kind: ControlKind,
    timeline_id: uuid::Uuid,
    commit_name: &str,
) {
    assert_eq!(control.kind, kind);
    assert_eq!(control.timeline_id, Some(timeline_id));
    assert_eq!(control.commit_name, commit_name);
    assert_eq!(control.keyframe_commit_name, commit_name);
    assert_eq!(control.expression_commit_name, commit_name);
    assert!(!control.commit_immediately);
}

fn selected_shape(project: &Project, key: SelectedItem) -> Option<&ShapeItem> {
    let item = project.video_item(&key)?;
    let VideoItemContent::Shape(shape) = &item.content else {
        return None;
    };
    Some(shape)
}

fn selected_shape_mut(project: &mut Project, key: SelectedItem) -> Option<&mut ShapeItem> {
    let item = project.video_item_mut(&key)?;
    let VideoItemContent::Shape(shape) = &mut item.content else {
        return None;
    };
    Some(shape)
}

fn shape_size_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<glam::Vec2>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.size)
}

fn shape_size(project: &Project, key: SelectedItem) -> Option<&TimelineValue<glam::Vec2>> {
    selected_shape(project, key.clone()).map(|shape| &shape.size)
}

fn shape_rotation_degrees(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_transform(project, key.clone()).map(|transform| &transform.rotation_degrees)
}

fn shape_rotation_degrees_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_transform_mut(project, key.clone()).map(|transform| &mut transform.rotation_degrees)
}

fn selected_transform(
    project: &Project,
    key: SelectedItem,
) -> Option<&shrimply_project_document::project::Transform> {
    project.video_item(&key).map(|item| &item.transform)
}

fn selected_transform_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut shrimply_project_document::project::Transform> {
    project.video_item_mut(&key).map(|item| &mut item.transform)
}

fn shape_outline_width(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.outline_width)
}

fn shape_star_inner_radius_percent(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.star_inner_radius_percent)
}

fn shape_star_inner_radius_percent_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.star_inner_radius_percent)
}

fn shape_arrow_shaft_width_percent(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.arrow_shaft_width_percent)
}

fn shape_arrow_shaft_width_percent_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.arrow_shaft_width_percent)
}

fn shape_arrow_head_length_percent(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.arrow_head_length_percent)
}

fn shape_arrow_head_length_percent_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.arrow_head_length_percent)
}

fn shape_cross_arm_thickness_percent(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.cross_arm_thickness_percent)
}

fn shape_cross_arm_thickness_percent_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.cross_arm_thickness_percent)
}

fn shape_ellipse_inner_radius_percent(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.ellipse_inner_radius_percent)
}

fn shape_ellipse_inner_radius_percent_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.ellipse_inner_radius_percent)
}

fn shape_ellipse_completion_degrees(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.ellipse_completion_degrees)
}

fn shape_ellipse_completion_degrees_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.ellipse_completion_degrees)
}

fn shape_outline_width_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.outline_width)
}

fn shape_corner_radius(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.corner_radius)
}

fn shape_corner_radius_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.corner_radius)
}

fn shape_shadow_distance(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.shadow_distance)
}

fn shape_shadow_distance_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.shadow_distance)
}

fn shape_shadow_direction_degrees(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.shadow_direction_degrees)
}

fn shape_shadow_direction_degrees_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.shadow_direction_degrees)
}

fn shape_shadow_width(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.shadow_width)
}

fn shape_shadow_width_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.shadow_width)
}

fn shape_shadow_blur(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    selected_shape(project, key.clone()).map(|shape| &shape.shadow_blur)
}

fn shape_shadow_blur_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.shadow_blur)
}

fn shape_fill(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.fill)
}

fn shape_outline_color(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.outline_color)
}

fn shape_shadow_color(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
    selected_shape_mut(project, key.clone()).map(|shape| &mut shape.shadow_color)
}

fn video_local_time_for_key(project: &Project, key: SelectedItem, position: Time) -> Option<Time> {
    crate::video::visual_local_time(project, key, position)
}

fn video_duration_for_key(project: &Project, key: SelectedItem) -> Option<Time> {
    let item = project.video_item(&key)?;
    generated_item_keyframe_span(item)
        .map(|(_, end)| end)
        .or_else(|| crate::video::visual_duration(project, key))
}
