use shrimply_project_document::project::{
    CanvasSize, SHAPE_APPEARANCE_PREVIEW_FACET, SHAPE_CONTENT_PREVIEW_FACET, ShapeItem, ShapeKind,
    Time, VideoItem, VideoItemContent,
};
use shrimply_property_model::{
    Color,
    timeline_value::{TimelineBase, TimelineValue},
};

use crate::{
    ControlKind, InspectorControl, InspectorRuntime, InspectorSection, LayeredState, NumberSpec,
    VideoCard,
};

pub const INTEGER_EDIT_COMMIT: &str = "edit-shape-points";
pub const INTEGER_KEYFRAME_COMMIT: &str = "edit-shape-points-keyframes";
pub const INTEGER_EXPRESSION_COMMIT: &str = "edit-shape-points-expression";
pub const STEP_EDIT_COMMIT: &str = "edit-shape-kind";
pub const ROUNDING_EDIT_COMMIT: &str = "edit-shape-rounding-strategy";
pub const SCALAR_EDIT_COMMIT: &str = "edit-shape-scalar";
pub const VECTOR_EDIT_COMMIT: &str = "edit-shape-vector";
pub const COLOR_EDIT_COMMIT: &str = "edit-shape-color";

pub(super) const SHAPE_PATH: &str = "/content/shape";
const SIZE_PATH: &str = "/content/size";
pub(super) const STAR_POINTS_PATH: &str = "/content/star_points";
pub(super) const ROUNDING_PATH: &str = "/content/rounding_strategy";

pub fn cards(
    shape: &ShapeItem,
    canvas_size: CanvasSize,
    runtime: InspectorRuntime,
) -> [VideoCard; 2] {
    let default = default_shape(canvas_size);
    [
        content_card(shape, &default, runtime),
        appearance_card(shape, &default, runtime),
    ]
}

fn content_card(shape: &ShapeItem, default: &ShapeItem, runtime: InspectorRuntime) -> VideoCard {
    let mut section = InspectorSection::default();
    section.add(
        crate::selector::layered_step_selector(SHAPE_PATH, "Shape", &shape.shape, runtime)
            .live_commit(STEP_EDIT_COMMIT)
            .timeline_commits(STEP_EDIT_COMMIT, STEP_EDIT_COMMIT),
    );
    section.add(vector_control(
        SIZE_PATH,
        "Size",
        &shape.size,
        runtime,
        NumberSpec {
            minimum: 1.0,
            drag_step: 1.0,
            digits: 0,
            unit: "px",
            ..NumberSpec::default()
        },
        ["W", "H"],
    ));

    match shape
        .shape
        .value_at(runtime.local_time.unwrap_or(Time::ZERO))
    {
        ShapeKind::Star => {
            section.add(integer_control(&shape.star_points, runtime));
            section.add(number_control(
                "/content/star_inner_radius_percent",
                "Inner radius",
                &shape.star_inner_radius_percent,
                runtime,
                percent_spec(5.0, 95.0),
            ));
        }
        ShapeKind::Arrow => {
            section.add(number_control(
                "/content/arrow_shaft_width_percent",
                "Shaft width",
                &shape.arrow_shaft_width_percent,
                runtime,
                percent_spec(5.0, 95.0),
            ));
            section.add(number_control(
                "/content/arrow_head_length_percent",
                "Head length",
                &shape.arrow_head_length_percent,
                runtime,
                percent_spec(5.0, 95.0),
            ));
        }
        ShapeKind::Cross => section.add(number_control(
            "/content/cross_arm_thickness_percent",
            "Arm thickness",
            &shape.cross_arm_thickness_percent,
            runtime,
            percent_spec(5.0, 95.0),
        )),
        ShapeKind::Ellipse => {
            section.add(number_control(
                "/content/ellipse_completion_degrees",
                "Completion",
                &shape.ellipse_completion_degrees,
                runtime,
                NumberSpec {
                    minimum: 0.0,
                    maximum: 360.0,
                    drag_step: 1.0,
                    digits: 0,
                    unit: "deg",
                },
            ));
            section.add(number_control(
                "/content/ellipse_inner_radius_percent",
                "Inner radius",
                &shape.ellipse_inner_radius_percent,
                runtime,
                percent_spec(0.0, 95.0),
            ));
        }
        ShapeKind::Rect
        | ShapeKind::Triangle
        | ShapeKind::Diamond
        | ShapeKind::Pentagon
        | ShapeKind::Hexagon
        | ShapeKind::Heart
        | ShapeKind::Octagon => {}
    }

    VideoCard::new("shape", "Shape", section)
        .reset_fields(
            [
                (SHAPE_PATH, json(&default.shape)),
                (SIZE_PATH, json(&default.size)),
                (STAR_POINTS_PATH, json(&default.star_points)),
                (
                    "/content/star_inner_radius_percent",
                    json(&default.star_inner_radius_percent),
                ),
                (
                    "/content/arrow_shaft_width_percent",
                    json(&default.arrow_shaft_width_percent),
                ),
                (
                    "/content/arrow_head_length_percent",
                    json(&default.arrow_head_length_percent),
                ),
                (
                    "/content/cross_arm_thickness_percent",
                    json(&default.cross_arm_thickness_percent),
                ),
                (
                    "/content/ellipse_inner_radius_percent",
                    json(&default.ellipse_inner_radius_percent),
                ),
                (
                    "/content/ellipse_completion_degrees",
                    json(&default.ellipse_completion_degrees),
                ),
            ],
            "reset-shape-content",
        )
        .preview_facet(SHAPE_CONTENT_PREVIEW_FACET)
}

fn appearance_card(shape: &ShapeItem, default: &ShapeItem, runtime: InspectorRuntime) -> VideoCard {
    let mut section = InspectorSection::default();
    section.add(color_control("/content/fill", "Fill", &shape.fill, runtime));
    section.add(color_control(
        "/content/outline_color",
        "Outline",
        &shape.outline_color,
        runtime,
    ));
    section.add(number_control(
        "/content/outline_width",
        "Outline width",
        &shape.outline_width,
        runtime,
        pixels_spec(),
    ));
    section.add(number_control(
        "/content/corner_radius",
        "Rounded",
        &shape.corner_radius,
        runtime,
        pixels_spec(),
    ));
    section.add(
        crate::selector::layered_step_selector(
            ROUNDING_PATH,
            "Rounding",
            &shape.rounding_strategy,
            runtime,
        )
        .live_commit(ROUNDING_EDIT_COMMIT)
        .timeline_commits(ROUNDING_EDIT_COMMIT, ROUNDING_EDIT_COMMIT),
    );
    section.add(color_control(
        "/content/shadow_color",
        "Shadow color",
        &shape.shadow_color,
        runtime,
    ));
    section.add(number_control(
        "/content/shadow_distance",
        "Shadow distance",
        &shape.shadow_distance,
        runtime,
        pixels_spec(),
    ));
    section.add(
        number_control(
            "/content/shadow_direction_degrees",
            "Shadow direction",
            &shape.shadow_direction_degrees,
            runtime,
            NumberSpec {
                drag_step: 1.0,
                digits: 0,
                unit: "deg",
                ..NumberSpec::default()
            },
        )
        .rotating_icon("arrow3-up-symbolic", 90.0),
    );
    section.add(number_control(
        "/content/shadow_width",
        "Shadow width",
        &shape.shadow_width,
        runtime,
        pixels_spec(),
    ));
    section.add(number_control(
        "/content/shadow_blur",
        "Shadow blur",
        &shape.shadow_blur,
        runtime,
        pixels_spec(),
    ));

    VideoCard::new("shape-appearance", "Appearance", section)
        .reset_fields(
            [
                (ROUNDING_PATH, json(&default.rounding_strategy)),
                ("/content/fill", json(&default.fill)),
                ("/content/outline_color", json(&default.outline_color)),
                ("/content/outline_width", json(&default.outline_width)),
                ("/content/corner_radius", json(&default.corner_radius)),
                ("/content/shadow_color", json(&default.shadow_color)),
                ("/content/shadow_distance", json(&default.shadow_distance)),
                (
                    "/content/shadow_direction_degrees",
                    json(&default.shadow_direction_degrees),
                ),
                ("/content/shadow_width", json(&default.shadow_width)),
                ("/content/shadow_blur", json(&default.shadow_blur)),
            ],
            "reset-shape-appearance",
        )
        .preview_facet(SHAPE_APPEARANCE_PREVIEW_FACET)
}

fn number_control(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    number: NumberSpec,
) -> InspectorControl {
    let current = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredNumber, path, label)
        .value(current.to_string())
        .number(number)
        .width_characters(9)
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::transform::scalar_graph(timeline, current, runtime),
        )
        .live_commit(SCALAR_EDIT_COMMIT)
        .timeline_commits(SCALAR_EDIT_COMMIT, SCALAR_EDIT_COMMIT)
}

fn vector_control(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<glam::Vec2>,
    runtime: InspectorRuntime,
    number: NumberSpec,
    prefixes: [&'static str; 2],
) -> InspectorControl {
    let current = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredVector2, path, label)
        .components(vec![current.x.to_string(), current.y.to_string()])
        .number(number)
        .width_characters(7)
        .prefixes(prefixes)
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::transform::vector_speed_graph(timeline, runtime),
        )
        .live_commit(VECTOR_EDIT_COMMIT)
        .timeline_commits(VECTOR_EDIT_COMMIT, VECTOR_EDIT_COMMIT)
}

fn integer_control(value: &TimelineValue<u32>, runtime: InspectorRuntime) -> InspectorControl {
    InspectorControl::new(ControlKind::LayeredNumber, STAR_POINTS_PATH, "Points")
        .value(
            value
                .value_at(runtime.local_time.unwrap_or(Time::ZERO))
                .to_string(),
        )
        .number(NumberSpec {
            minimum: 3.0,
            maximum: 32.0,
            drag_step: 1.0,
            digits: 0,
            unit: "",
        })
        .unsigned_integer_timeline()
        .width_characters(9)
        .layered(STAR_POINTS_PATH, LayeredState::from(value))
        .timeline(value.id, integer_graph(value, runtime))
        .live_commit(INTEGER_EDIT_COMMIT)
        .timeline_commits(INTEGER_KEYFRAME_COMMIT, INTEGER_EXPRESSION_COMMIT)
}

fn color_control(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<Color<u8>>,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredColor, path, label)
        .components(vec![
            value.r.to_string(),
            value.g.to_string(),
            value.b.to_string(),
            value.a.to_string(),
        ])
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::timeline_color::speed_graph(timeline, runtime),
        )
        .live_commit(COLOR_EDIT_COMMIT)
        .timeline_commits(COLOR_EDIT_COMMIT, COLOR_EDIT_COMMIT)
}

fn percent_spec(minimum: f64, maximum: f64) -> NumberSpec {
    NumberSpec {
        minimum,
        maximum,
        drag_step: 1.0,
        digits: 0,
        unit: "%",
    }
}

fn pixels_spec() -> NumberSpec {
    NumberSpec {
        minimum: 0.0,
        drag_step: 1.0,
        digits: 0,
        unit: "px",
        ..NumberSpec::default()
    }
}

fn default_shape(canvas_size: CanvasSize) -> ShapeItem {
    let VideoItemContent::Shape(shape) =
        VideoItem::shape_item(canvas_size, Time::ZERO, Time::ZERO).content
    else {
        unreachable!("shape item constructor must create shape content")
    };
    *shape
}

fn json(value: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).expect("shape inspector value must serialize")
}

pub(crate) fn integer_graph(
    value: &TimelineValue<u32>,
    runtime: InspectorRuntime,
) -> Option<crate::ScalarGraph> {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return None;
    };
    Some(crate::ScalarGraph {
        points: keyframes
            .iter()
            .map(|keyframe| crate::GraphPoint {
                time: keyframe.time,
                value: f64::from(keyframe.value),
            })
            .collect(),
        segments: keyframes
            .windows(2)
            .map(|pair| crate::GraphSegment {
                owner_id: pair[0].id,
                start: pair[0].time,
                end: pair[1].time,
                start_value: f64::from(pair[0].value),
                end_value: f64::from(pair[1].value),
                interpolation: shrimply_property_model::timeline_value::Interpolation::KEYFRAME
                    .iter()
                    .position(|candidate| *candidate == pair[0].interpolation_to_next)
                    .expect("shape integer interpolation must be available"),
            })
            .collect(),
        range: runtime.keyframe_range.unwrap_or((Time::ZERO, Time::ZERO)),
        frame_step: runtime.frame_step,
        playhead: runtime.keyframe_playhead.unwrap_or(Time::ZERO),
    })
}

pub(crate) fn integer_value<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a TimelineValue<u32>> {
    if path != STAR_POINTS_PATH {
        return None;
    }
    let VideoItemContent::Shape(shape) = &item.content else {
        return None;
    };
    (shape.star_points.id == id).then_some(&shape.star_points)
}

pub fn is_timeline_path(path: &str) -> bool {
    matches!(
        path,
        SHAPE_PATH
            | SIZE_PATH
            | STAR_POINTS_PATH
            | "/content/star_inner_radius_percent"
            | "/content/arrow_shaft_width_percent"
            | "/content/arrow_head_length_percent"
            | "/content/cross_arm_thickness_percent"
            | "/content/ellipse_inner_radius_percent"
            | "/content/ellipse_completion_degrees"
            | "/content/fill"
            | "/content/outline_color"
            | "/content/outline_width"
            | "/content/corner_radius"
            | ROUNDING_PATH
            | "/content/shadow_color"
            | "/content/shadow_distance"
            | "/content/shadow_direction_degrees"
            | "/content/shadow_width"
            | "/content/shadow_blur"
    )
}

pub(crate) fn color_value<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a TimelineValue<Color<u8>>> {
    let VideoItemContent::Shape(shape) = &item.content else {
        return None;
    };
    let value = match path {
        "/content/fill" => &shape.fill,
        "/content/outline_color" => &shape.outline_color,
        "/content/shadow_color" => &shape.shadow_color,
        _ => return None,
    };
    (value.id == id).then_some(value)
}
