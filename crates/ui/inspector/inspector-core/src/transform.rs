use glam::{Vec2, Vec3};
use shrimply_project_document::project::{
    ItemAddress, Project, ResolvedTransform, Time, Transform, VideoItem, VideoItemContent,
};
use shrimply_property_model::timeline_value::{Interpolation, TimelineBase, TimelineValue};
use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect, VectorModifierEffect};

use crate::{
    ControlKind, GraphPoint, GraphSegment, InspectorCommit, InspectorControl, InspectorController,
    InspectorExpressionOutput, InspectorRuntime, InspectorSection, InspectorTarget,
    KeyframeCommits, LayeredState, NumberSpec, ScalarGraph, VideoCard,
};

pub mod expressions;

pub const TRANSFORM_CARD_KEY: &str = "transform";
pub const TRANSFORM_CARD_TITLE: &str = "Transform";
pub const TRANSFORM_LIVE_COMMIT: &str = "video-transform";
pub const RESET_TRANSFORM_COMMIT: &str = "reset-transform";
pub const TRANSFORM_KEYFRAME_COMMITS: KeyframeCommits = KeyframeCommits {
    toggle: "video-transform-keyframes",
    edit: "video-transform-keyframe",
    add: "add-transform-keyframe",
    delete: "delete-transform-keyframe",
    move_keyframe: "video-transform-keyframe-point",
    paste: "paste-transform-keyframes",
    interpolation: "video-transform-keyframe-interpolation",
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Vec2Field {
    Position,
    Anchor,
    Scale,
    Shear,
}

impl Vec2Field {
    pub const ALL: [Self; 4] = [Self::Position, Self::Anchor, Self::Scale, Self::Shear];

    pub const fn path(self) -> &'static str {
        match self {
            Self::Position => "/transform/position",
            Self::Anchor => "/transform/anchor",
            Self::Scale => "/transform/scale",
            Self::Shear => "/transform/shear",
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.path() == path)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Position => "Position",
            Self::Anchor => "Anchor",
            Self::Scale => "Scale",
            Self::Shear => "Shear",
        }
    }

    pub fn number(self) -> NumberSpec {
        match self {
            Self::Position | Self::Anchor => NumberSpec {
                drag_step: 1.0,
                digits: 0,
                unit: "px",
                ..NumberSpec::default()
            },
            Self::Scale => NumberSpec {
                minimum: 0.0,
                drag_step: 0.01,
                digits: 2,
                unit: "x",
                ..NumberSpec::default()
            },
            Self::Shear => NumberSpec {
                drag_step: 0.01,
                digits: 2,
                ..NumberSpec::default()
            },
        }
    }

    pub const fn lock(self) -> bool {
        matches!(self, Self::Scale)
    }

    pub const fn width_characters(self) -> i32 {
        7
    }

    pub const fn prefixes(self) -> [&'static str; 2] {
        ["X", "Y"]
    }

    pub fn value(self, transform: ResolvedTransform) -> Vec2 {
        match self {
            Self::Position => transform.position,
            Self::Anchor => transform.anchor,
            Self::Scale => transform.scale,
            Self::Shear => transform.shear,
        }
    }

    pub fn timeline(self, transform: &Transform) -> &TimelineValue<Vec2> {
        match self {
            Self::Position => &transform.position,
            Self::Anchor => &transform.anchor,
            Self::Scale => &transform.scale,
            Self::Shear => &transform.shear,
        }
    }

    pub fn timeline_mut(self, transform: &mut Transform) -> &mut TimelineValue<Vec2> {
        match self {
            Self::Position => &mut transform.position,
            Self::Anchor => &mut transform.anchor,
            Self::Scale => &mut transform.scale,
            Self::Shear => &mut transform.shear,
        }
    }

    pub fn normalize(self, value: Vec2) -> Vec2 {
        if self.lock() {
            value.max(Vec2::ZERO)
        } else {
            value
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarField {
    RotationDegrees,
}

impl ScalarField {
    pub const ALL: [Self; 1] = [Self::RotationDegrees];

    pub const fn path(self) -> &'static str {
        match self {
            Self::RotationDegrees => "/transform/rotation_degrees",
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.path() == path)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::RotationDegrees => "Rotation",
        }
    }

    pub fn number(self) -> NumberSpec {
        match self {
            Self::RotationDegrees => NumberSpec {
                drag_step: 0.1,
                digits: 1,
                unit: "°",
                ..NumberSpec::default()
            },
        }
    }

    pub const fn width_characters(self) -> i32 {
        9
    }

    pub fn value(self, transform: ResolvedTransform) -> f64 {
        match self {
            Self::RotationDegrees => f64::from(transform.rotation_degrees),
        }
    }

    pub fn timeline(self, transform: &Transform) -> &TimelineValue<f32> {
        match self {
            Self::RotationDegrees => &transform.rotation_degrees,
        }
    }

    pub fn timeline_mut(self, transform: &mut Transform) -> &mut TimelineValue<f32> {
        match self {
            Self::RotationDegrees => &mut transform.rotation_degrees,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformField {
    Vec2(Vec2Field),
    Scalar(ScalarField),
}

impl TransformField {
    pub const ALL: [Self; 5] = [
        Self::Vec2(Vec2Field::Position),
        Self::Vec2(Vec2Field::Anchor),
        Self::Vec2(Vec2Field::Scale),
        Self::Vec2(Vec2Field::Shear),
        Self::Scalar(ScalarField::RotationDegrees),
    ];

    pub const fn path(self) -> &'static str {
        match self {
            Self::Vec2(field) => field.path(),
            Self::Scalar(field) => field.path(),
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.path() == path)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vec2(field) => field.label(),
            Self::Scalar(field) => field.label(),
        }
    }

    pub fn timeline_id(self, transform: &Transform) -> uuid::Uuid {
        match self {
            Self::Vec2(field) => field.timeline(transform).id,
            Self::Scalar(field) => field.timeline(transform).id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransformLivePresentation {
    pub resolved: ResolvedTransform,
    pub vectors: Vec<(String, Vec2)>,
    pub numbers: Vec<(String, f64)>,
    pub graphs: Vec<(String, ScalarGraph)>,
}

impl TransformLivePresentation {
    pub fn vector(&self, path: &str) -> Option<Vec2> {
        self.vectors
            .iter()
            .find_map(|(candidate, value)| (candidate == path).then_some(*value))
    }

    pub fn graph(&self, path: &str) -> Option<&ScalarGraph> {
        self.graphs
            .iter()
            .find_map(|(candidate, graph)| (candidate == path).then_some(graph))
    }

    pub fn number(&self, path: &str) -> Option<f64> {
        self.numbers
            .iter()
            .find_map(|(candidate, value)| (candidate == path).then_some(*value))
    }
}

pub(crate) fn card(
    project: &Project,
    address: &ItemAddress,
    item: &VideoItem,
    runtime: InspectorRuntime,
) -> Option<VideoCard> {
    if matches!(
        item.content,
        VideoItemContent::Obj(_)
            | VideoItemContent::Gaussian(_)
            | VideoItemContent::Manim(_)
            | VideoItemContent::Background(_)
    ) {
        return None;
    }
    let display = project
        .timeline_time_to_sequence(&address.track(), runtime.position)
        .map(|position| {
            shrimply_project_evaluation::resolve_item_base_transform(project, item, position)
        })
        .unwrap_or_else(|| item.transform.fallback());
    let section = section(&item.transform, display, runtime);
    Some(
        VideoCard {
            key: TRANSFORM_CARD_KEY,
            title: TRANSFORM_CARD_TITLE,
            section,
            reset: None,
            alpha_mask: None,
            preview_facet: None,
            actions: Vec::new(),
            scope: crate::VideoCardScope::Common,
        }
        .reset(
            "/transform",
            serde_json::to_value(item.natural_transform(project.canvas_size))
                .expect("natural video transform must serialize"),
            RESET_TRANSFORM_COMMIT,
        ),
    )
}

pub fn section(
    transform: &Transform,
    display: ResolvedTransform,
    runtime: InspectorRuntime,
) -> InspectorSection {
    let mut section = InspectorSection::default();
    for field in TransformField::ALL {
        section.add(match field {
            TransformField::Vec2(field) => vector_control(
                field,
                field.timeline(transform),
                field.value(display),
                runtime,
            ),
            TransformField::Scalar(field) => scalar_control(
                field,
                field.timeline(transform),
                field.value(display),
                runtime,
            ),
        });
    }
    section
}

fn vector_control(
    field: Vec2Field,
    timeline: &TimelineValue<Vec2>,
    display: Vec2,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let path = field.path();
    let control = InspectorControl::new(ControlKind::LayeredVector2, path, field.label())
        .components(vec![display.x.to_string(), display.y.to_string()])
        .number(field.number())
        .width_characters(field.width_characters())
        .prefixes(field.prefixes())
        .layered(path, LayeredState::from(timeline))
        .timeline(timeline.id, vector_speed_graph(timeline, runtime))
        .live_commit(TRANSFORM_LIVE_COMMIT)
        .keyframe_commits(TRANSFORM_KEYFRAME_COMMITS);
    if field.lock() {
        control.lock()
    } else {
        control
    }
}

fn scalar_control(
    field: ScalarField,
    timeline: &TimelineValue<f32>,
    display: f64,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let path = field.path();
    InspectorControl::new(ControlKind::LayeredNumber, path, field.label())
        .value(display.to_string())
        .number(field.number())
        .width_characters(field.width_characters())
        .rotating_icon("arrow3-up-symbolic", 0.0)
        .layered(path, LayeredState::from(timeline))
        .timeline(timeline.id, scalar_graph(timeline, display as f32, runtime))
        .live_commit(TRANSFORM_LIVE_COMMIT)
        .keyframe_commits(TRANSFORM_KEYFRAME_COMMITS)
}

fn graph_shell(
    runtime: InspectorRuntime,
    points: Vec<GraphPoint>,
    segments: Vec<GraphSegment>,
) -> ScalarGraph {
    ScalarGraph {
        points,
        segments,
        range: runtime.keyframe_range.unwrap_or((Time::ZERO, Time::ZERO)),
        frame_step: runtime.frame_step,
        playhead: runtime.keyframe_playhead.unwrap_or(Time::ZERO),
    }
}

pub(crate) fn vector_speed_graph(
    timeline: &TimelineValue<Vec2>,
    runtime: InspectorRuntime,
) -> Option<ScalarGraph> {
    crate::timeline_value::vector::scalar_speed_graph(timeline, runtime)
}

pub(crate) fn scalar_graph(
    timeline: &TimelineValue<f32>,
    display: f32,
    runtime: InspectorRuntime,
) -> Option<ScalarGraph> {
    let crate::keyframe_graph::KeyframeGraph::RawValue {
        points, segments, ..
    } = crate::keyframe_model::scalar_graph(timeline, f64::from(display), f64::from)
    else {
        unreachable!("scalar transform must produce a raw graph")
    };
    matches!(timeline.base, TimelineBase::Keyframes(_)).then(|| {
        graph_shell(
            runtime,
            points
                .into_iter()
                .map(|point| GraphPoint {
                    time: point.time,
                    value: point.value,
                })
                .collect(),
            segments
                .into_iter()
                .map(|segment| GraphSegment {
                    owner_id: segment.owner_id,
                    start: segment.start,
                    end: segment.end,
                    start_value: segment.start_value,
                    end_value: segment.end_value,
                    interpolation: interpolation_index(segment.interpolation),
                })
                .collect(),
        )
    })
}

fn interpolation_index(interpolation: Interpolation) -> usize {
    Interpolation::KEYFRAME
        .iter()
        .position(|candidate| *candidate == interpolation)
        .expect("transform interpolation must be available")
}

impl InspectorController {
    pub fn resolved_transform(&self, target: &InspectorTarget) -> Option<ResolvedTransform> {
        let position = crate::model::player_snapshot(&self.player_state).position;
        let project = self.project.borrow();
        let (item, position) = transform_item_at(&project, target, position)?;
        Some(shrimply_project_evaluation::resolve_item_base_transform(
            &project, item, position,
        ))
    }

    pub fn transform_live_presentation(
        &self,
        target: &InspectorTarget,
    ) -> Option<TransformLivePresentation> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let (item, position) = transform_item_at(&project, target, runtime.position)?;
        let resolved =
            shrimply_project_evaluation::resolve_item_base_transform(&project, item, position);
        let mut vectors = Vec2Field::ALL
            .into_iter()
            .map(|field| (field.path().to_string(), field.value(resolved)))
            .collect::<Vec<_>>();
        let mut numbers = vec![(
            ScalarField::RotationDegrees.path().to_string(),
            f64::from(resolved.rotation_degrees),
        )];
        let mut graphs = Vec2Field::ALL
            .into_iter()
            .filter_map(|field| {
                vector_speed_graph(field.timeline(&item.transform), runtime)
                    .map(|graph| (field.path().to_string(), graph))
            })
            .collect::<Vec<_>>();
        if let Some(graph) = scalar_graph(
            &item.transform.rotation_degrees,
            resolved.rotation_degrees,
            runtime,
        ) {
            graphs.push((ScalarField::RotationDegrees.path().to_string(), graph));
        }
        let player = crate::model::player_snapshot(&self.player_state);
        let audio =
            self.audio_sampler
                .borrow_mut()
                .sample(&project, player.position, player.revision);
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
            &project, item, position, &audio,
        );
        let mut expression_cache = self.expression_cache.borrow_mut();
        for (index, modifier) in item.modifiers.iter().enumerate() {
            let transform = match &modifier.effect {
                ModifierEffect::Vector(effect) => match &**effect {
                    VectorModifierEffect::Transform(transform) => Some(transform.as_ref()),
                    _ => None,
                },
                ModifierEffect::Raster(effect) => match &**effect {
                    RasterModifierEffect::Transform(transform) => Some(transform.as_ref()),
                    _ => None,
                },
                ModifierEffect::Scene3d(_)
                | ModifierEffect::Vectorize(_)
                | ModifierEffect::Rasterize(_) => None,
            };
            let Some(transform) = transform else {
                continue;
            };
            let base = format!("/modifiers/{index}/effect/effect/config/transform");
            for (field, timeline) in [
                ("position", transform.position()),
                ("anchor", transform.anchor()),
                ("scale", transform.scale()),
                ("shear", transform.shear()),
            ] {
                let path = format!("{base}/{field}");
                vectors.push((
                    path.clone(),
                    shrimply_project_evaluation::resolve_with_error(
                        timeline,
                        &evaluation,
                        &mut expression_cache,
                    )
                    .value,
                ));
                if let Some(graph) = vector_speed_graph(timeline, runtime) {
                    graphs.push((path, graph));
                }
            }
            let rotation = shrimply_project_evaluation::resolve_with_error(
                transform.rotation_degrees(),
                &evaluation,
                &mut expression_cache,
            )
            .value;
            numbers.push((format!("{base}/rotation_degrees"), f64::from(rotation)));
            if let Some(graph) = scalar_graph(transform.rotation_degrees(), rotation, runtime) {
                graphs.push((format!("{base}/rotation_degrees"), graph));
            }
        }
        Some(TransformLivePresentation {
            resolved,
            vectors,
            numbers,
            graphs,
        })
    }

    pub fn set_vector2_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        first: f64,
        second: f64,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        if !first.is_finite() || !second.is_finite() {
            return Err("transform vector must be finite".to_string());
        }
        let (mut timeline, runtime) = self.vector2_timeline(target, path)?;
        let mut next = Vec2::new(first as f32, second as f32);
        if let Some(field) = Vec2Field::from_path(path) {
            next = field.normalize(next);
        }
        let time = if matches!(timeline.base, TimelineBase::Keyframes(_)) {
            runtime
                .keyframe_playhead
                .ok_or_else(|| "transform keyframe time is no longer available".to_string())?
        } else {
            Time::ZERO
        };
        if !crate::timeline_value::vector::vec2::set_value(&mut timeline, time, next) {
            return Ok(());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            commit,
        )
    }

    pub fn set_vector2_component_value(
        &self,
        target: &InspectorTarget,
        field: Vec2Field,
        timeline_id: uuid::Uuid,
        component: usize,
        value: f64,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("transform vector component must be finite".to_string());
        }
        if self.target() != *target {
            return Err("inspector target changed".to_string());
        }
        let path = field.path();
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let mut timeline = field
            .timeline(
                &project
                    .video_item(video_address(target)?)
                    .ok_or_else(|| "transform item is no longer available".to_string())?
                    .transform,
            )
            .clone();
        drop(project);
        if timeline.id != timeline_id {
            return Err("transform vector is no longer available".to_string());
        }
        let mut next = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
        match component {
            0 => next.x = value as f32,
            1 => next.y = value as f32,
            _ => return Err("transform vector component is invalid".to_string()),
        }
        let time = if matches!(timeline.base, TimelineBase::Keyframes(_)) {
            runtime
                .keyframe_playhead
                .ok_or_else(|| "transform keyframe time is no longer available".to_string())?
        } else {
            Time::ZERO
        };
        if !crate::timeline_value::vector::vec2::set_value(
            &mut timeline,
            time,
            field.normalize(next),
        ) {
            return Ok(());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            commit,
        )
    }

    pub fn set_transform_scalar_value(
        &self,
        target: &InspectorTarget,
        field: ScalarField,
        timeline_id: uuid::Uuid,
        value: f64,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err("transform scalar must be finite".to_string());
        }
        if self.target() != *target {
            return Err("inspector target changed".to_string());
        }
        let path = field.path();
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let mut timeline = field
            .timeline(
                &project
                    .video_item(video_address(target)?)
                    .ok_or_else(|| "transform item is no longer available".to_string())?
                    .transform,
            )
            .clone();
        drop(project);
        if timeline.id != timeline_id {
            return Err("transform scalar is no longer available".to_string());
        }
        let time = if matches!(timeline.base, TimelineBase::Keyframes(_)) {
            runtime
                .keyframe_playhead
                .ok_or_else(|| "transform keyframe time is no longer available".to_string())?
        } else {
            Time::ZERO
        };
        if !crate::timeline_value::scalar::set_displayed_value(
            &mut timeline,
            time,
            value,
            |value| value as f32,
            crate::timeline_value::scalar::ScalarConstraint::Function(|value| value),
        ) {
            return Ok(());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("transform scalar must serialize"),
            commit,
        )
    }

    pub fn set_vector3_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        first: f64,
        second: f64,
        third: f64,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        if !first.is_finite() || !second.is_finite() || !third.is_finite() {
            return Err("modifier vector must be finite".to_string());
        }
        let (mut timeline, runtime) = self.vector3_timeline(target, path)?;
        runtime
            .local_time
            .ok_or_else(|| "modifier evaluation time is no longer available".to_string())?;
        let time = if matches!(timeline.base, TimelineBase::Keyframes(_)) {
            runtime
                .keyframe_playhead
                .ok_or_else(|| "modifier keyframe time is no longer available".to_string())?
        } else {
            Time::ZERO
        };
        if !crate::timeline_value::vector::vec3::set_value(
            &mut timeline,
            time,
            Vec3::new(first as f32, second as f32, third as f32),
        ) {
            return Ok(());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            commit,
        )
    }

    pub fn set_vector2_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, runtime) = self.vector2_timeline(target, path)?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "transform keyframe time is no longer available".to_string())?;
        let evaluation_time = runtime.local_time.unwrap_or(Time::ZERO);
        let changed = if let Some(field) = Vec2Field::from_path(path) {
            let current = field.normalize(timeline.value_at(evaluation_time));
            crate::keyframe_model::set_keyframes_enabled(&mut timeline, time, current, enabled)
        } else {
            crate::timeline_value::vector::vec2::set_keyframes_enabled(
                &mut timeline,
                evaluation_time,
                time,
                enabled,
            )
        };
        if !changed {
            return Ok(());
        }
        if let Some(field) = Vec2Field::from_path(path) {
            normalize_vector_timeline(&mut timeline, field);
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn set_vector2_expression_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        if !crate::timeline_value::vector::vec2::set_expression_enabled(&mut timeline, enabled) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn set_vector2_expression_source(
        &self,
        target: &InspectorTarget,
        path: &str,
        source: String,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        if !crate::timeline_value::vector::vec2::set_expression_source(&mut timeline, source) {
            return Ok(());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            commit,
        )
    }

    pub fn set_vector3_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, runtime) = self.vector3_timeline(target, path)?;
        let evaluation_time = runtime
            .local_time
            .ok_or_else(|| "modifier evaluation time is no longer available".to_string())?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "modifier keyframe time is no longer available".to_string())?;
        if !crate::timeline_value::vector::vec3::set_keyframes_enabled(
            &mut timeline,
            evaluation_time,
            time,
            enabled,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn vector2_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: Option<uuid::Uuid>,
    ) -> Result<InspectorExpressionOutput<Vec2>, String> {
        if path.starts_with("/modifiers/")
            && let Some(timeline_id) = timeline_id
        {
            return self.video_modifier_expression_output(
                target,
                path,
                timeline_id,
                crate::visual_modifiers::visual_modifier_vector2,
            );
        }
        if let Some(timeline_id) = timeline_id {
            self.ensure_timeline(target, path, timeline_id)?;
        }
        self.video_expression_output(target, path)
    }

    pub fn vector3_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<InspectorExpressionOutput<Vec3>, String> {
        if path.starts_with("/modifiers/") {
            self.video_modifier_expression_output(
                target,
                path,
                timeline_id,
                crate::visual_modifiers::visual_modifier_vector3,
            )
        } else {
            self.ensure_timeline(target, path, timeline_id)?;
            self.video_expression_output(target, path)
        }
    }

    pub fn move_vector2_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        moves: &[(Time, Time)],
        commit: InspectorCommit<'_>,
    ) -> Result<Vec<Time>, String> {
        let moves = self.canonical_vector_moves(target, moves)?;
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        if !crate::timeline_value::vector::vec2::move_keyframes(&mut timeline, &moves) {
            return Err("transform keyframe move targets are no longer available".to_string());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            commit,
        )?;
        Ok(moves.into_iter().map(|(_, time)| time).collect())
    }

    pub fn delete_vector2_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.delete_vector2_keyframes(target, path, std::slice::from_ref(&time), commit)
    }

    pub fn delete_vector2_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        times: &[Time],
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, runtime) = self.vector2_timeline(target, path)?;
        let evaluation_time = runtime
            .local_time
            .ok_or_else(|| "vector evaluation time is no longer available".to_string())?;
        if !crate::keyframe_model::delete_keyframes(
            &mut timeline,
            times,
            evaluation_time,
            Time::ZERO,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn add_vector2_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let time = self.canonical_vector_time(target, time)?;
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        if !shrimply_property_model::timeline_value::add_curve_keyframe(
            &mut timeline,
            time,
            std::convert::identity,
        ) {
            return Ok(());
        }
        if let Some(field) = Vec2Field::from_path(path) {
            normalize_vector_timeline(&mut timeline, field);
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn copy_vector2_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (timeline, _) = self.vector2_timeline(target, path)?;
        let Some(mut clipboard) =
            crate::timeline_value::vector::vec2::copy_keyframes(&timeline, selected)
        else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_vector2_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("transform keyframes cannot be pasted at this time".to_string());
        };
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        let Some(pasted) =
            crate::timeline_value::vector::vec2::paste_keyframes(&mut timeline, &clipboard, &times)
        else {
            return Ok(0);
        };
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )?;
        Ok(pasted.len())
    }

    pub fn set_vector2_interpolation(
        &self,
        target: &InspectorTarget,
        path: &str,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let (mut timeline, _) = self.vector2_timeline(target, path)?;
        if !crate::timeline_value::vector::vec2::set_interpolation(
            &mut timeline,
            owner_id,
            interpolation,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("transform vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn move_vector3_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        moves: &[(Time, Time)],
        commit: InspectorCommit<'_>,
    ) -> Result<Vec<Time>, String> {
        let moves = self.canonical_vector_moves(target, moves)?;
        let (mut timeline, _) = self.vector3_timeline(target, path)?;
        if !crate::timeline_value::vector::vec3::move_keyframes(&mut timeline, &moves) {
            return Err("modifier keyframe move targets are no longer available".to_string());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            commit,
        )?;
        Ok(moves.into_iter().map(|(_, time)| time).collect())
    }

    pub fn delete_vector3_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.delete_vector3_keyframes(target, path, std::slice::from_ref(&time), commit)
    }

    pub fn delete_vector3_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        times: &[Time],
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut timeline, runtime) = self.vector3_timeline(target, path)?;
        let evaluation_time = runtime
            .local_time
            .ok_or_else(|| "vector evaluation time is no longer available".to_string())?;
        if !crate::keyframe_model::delete_keyframes(
            &mut timeline,
            times,
            evaluation_time,
            Time::ZERO,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn add_vector3_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let time = self.canonical_vector_time(target, time)?;
        let (mut timeline, _) = self.vector3_timeline(target, path)?;
        if !shrimply_property_model::timeline_value::add_curve_keyframe(
            &mut timeline,
            time,
            std::convert::identity,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn copy_vector3_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (timeline, _) = self.vector3_timeline(target, path)?;
        let Some(mut clipboard) =
            crate::timeline_value::vector::vec3::copy_keyframes(&timeline, selected)
        else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_vector3_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("modifier keyframes cannot be pasted at this time".to_string());
        };
        let (mut timeline, _) = self.vector3_timeline(target, path)?;
        let Some(pasted) =
            crate::timeline_value::vector::vec3::paste_keyframes(&mut timeline, &clipboard, &times)
        else {
            return Ok(0);
        };
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )?;
        Ok(pasted.len())
    }

    pub fn set_vector3_interpolation(
        &self,
        target: &InspectorTarget,
        path: &str,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let (mut timeline, _) = self.vector3_timeline(target, path)?;
        if !crate::timeline_value::vector::vec3::set_interpolation(
            &mut timeline,
            owner_id,
            interpolation,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(timeline).expect("modifier vector must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    fn vector2_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<(TimelineValue<Vec2>, InspectorRuntime), String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let timeline = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("vector value is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid vector value: {error}"))?;
        Ok((timeline, snapshot.runtime))
    }

    fn vector3_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<(TimelineValue<Vec3>, InspectorRuntime), String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let timeline = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("vector value is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid vector value: {error}"))?;
        Ok((timeline, snapshot.runtime))
    }

    fn canonical_vector_time(&self, target: &InspectorTarget, time: Time) -> Result<Time, String> {
        let project = self.project.borrow();
        let address = video_address(target)?;
        project
            .keyframe_timeline_time(address, time)
            .and_then(|timeline_time| project.keyframe_time(address, timeline_time))
            .ok_or_else(|| "transform keyframe time is no longer available".to_string())
    }

    fn canonical_vector_moves(
        &self,
        target: &InspectorTarget,
        moves: &[(Time, Time)],
    ) -> Result<Vec<(Time, Time)>, String> {
        moves
            .iter()
            .map(|&(old_time, time)| Ok((old_time, self.canonical_vector_time(target, time)?)))
            .collect()
    }
}

fn video_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
        return Err("transform target is not a video item".to_string());
    };
    Ok(address)
}

fn normalize_vector_timeline(timeline: &mut TimelineValue<Vec2>, field: Vec2Field) {
    match &mut timeline.base {
        TimelineBase::Const(value) => *value = field.normalize(*value),
        TimelineBase::Keyframes(keyframes) => keyframes
            .iter_mut()
            .for_each(|keyframe| keyframe.value = field.normalize(keyframe.value)),
    }
}

fn transform_item_at<'a>(
    project: &'a Project,
    target: &InspectorTarget,
    position: Time,
) -> Option<(&'a VideoItem, Time)> {
    let address = video_address(target).ok()?;
    let item = project.video_item(address)?;
    if matches!(
        item.content,
        VideoItemContent::Obj(_)
            | VideoItemContent::Gaussian(_)
            | VideoItemContent::Manim(_)
            | VideoItemContent::Background(_)
    ) {
        return None;
    }
    let position = project.timeline_time_to_sequence(&address.track(), position)?;
    Some((item, position))
}
