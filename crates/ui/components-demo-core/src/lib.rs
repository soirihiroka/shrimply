use rhai::{Engine, Scope};
use shrimply_core::modifier_model::ModifierModel;
use shrimply_interpolation::Interpolation;
use shrimply_keyframe_graph_core::{
    FrameGraphComponents, FrameGraphState, KeyframeGraph, KeyframePoint, RawSegment,
};
use shrimply_math_core::Time;
use shrimply_video_modifiers::{ModifierEffect, ModifierSource, ModifierState, VisualKind};
use uuid::Uuid;

pub const EXPRESSION_SOURCE: &str = "value * 2.0";
pub const EXPRESSION_INPUT: f64 = 42.0;

const GRAPH_TIMES: [(i64, i64); 4] = [(1, 2), (3, 2), (5, 2), (7, 2)];
const GRAPH_OWNER_IDS: [u128; 3] = [1, 2, 3];
const GRAPH_INTERPOLATIONS: [Interpolation; 3] = [
    Interpolation::SineInOut,
    Interpolation::ManimSmooth,
    Interpolation::SineOut,
];

pub fn modifier_names() -> Vec<&'static str> {
    ModifierEffect::catalog()
        .filter_map(|effect| effect.adapted_for(demo_modifier_state()))
        .map(|effect| effect.display_name())
        .collect()
}

fn demo_modifier_state() -> ModifierState {
    ModifierState {
        source: ModifierSource::Image,
        kind: VisualKind::Raster,
        pristine: true,
    }
}

pub fn expression_output(source: &str) -> String {
    let mut engine = Engine::new();
    engine.set_max_call_levels(16).set_max_operations(10_000);
    let mut scope = Scope::new();
    scope.push("value", EXPRESSION_INPUT);
    match engine.eval_with_scope::<f64>(&mut scope, source) {
        Ok(value) => format!("Output · {value:.1}"),
        Err(error) => format!("Error · {error}"),
    }
}

pub fn property_graph(value: f64) -> KeyframeGraph {
    let spread = value.abs().max(1.0) * 0.25;
    let values = [value, value + spread, value - spread, value];
    let points = GRAPH_TIMES
        .into_iter()
        .zip(values)
        .map(|((numerator, denominator), value)| KeyframePoint {
            time: Time::from_fraction(numerator, denominator),
            value,
        })
        .collect::<Vec<_>>();
    let segments = points
        .windows(2)
        .zip(GRAPH_OWNER_IDS)
        .zip(GRAPH_INTERPOLATIONS)
        .map(|((pair, owner_id), interpolation)| RawSegment {
            owner_id: Uuid::from_u128(owner_id),
            start: pair[0].time,
            end: pair[1].time,
            start_value: pair[0].value,
            end_value: pair[1].value,
            interpolation,
        })
        .collect();
    KeyframeGraph::RawValue {
        points,
        segments,
        static_value: value,
    }
}

pub fn property_graph_state(value: f64) -> FrameGraphState {
    FrameGraphState::new(
        property_graph(value),
        (Time::ZERO, Time::from_seconds(4)),
        Time::from_fraction(1, 30),
        Time::from_fraction(3, 2),
    )
}

pub fn property_graph_components(values: &[f64], active_component: usize) -> FrameGraphComponents {
    FrameGraphComponents::new(
        values.iter().copied().map(property_graph_state).collect(),
        active_component,
    )
}
