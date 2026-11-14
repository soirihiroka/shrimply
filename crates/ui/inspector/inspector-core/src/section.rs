const DEFAULT_MINIMUM: f64 = -1_000_000.0;
const DEFAULT_MAXIMUM: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlKind {
    Boolean,
    Number,
    Fraction,
    Text,
    MultilineText,
    ReadOnly,
    Color,
    LayeredColor,
    LayeredText,
    LayeredDrawing,
    FontFamilies,
    VoiceModel,
    Selector,
    OptionalSelector,
    OptionalNumberSelector,
    Vector2,
    Vector3,
    LayeredNumber,
    LayeredBoolean,
    LayeredSelector,
    AudioCache,
    AudioCachePreset,
    VisualCache,
    VisualCacheQuality,
    Analysis,
    AudioModifierMenu,
    VisualModifierMenu,
    TtsEditor,
    BeatDetection,
    LayeredVector2,
    LayeredVector3,
    ProjectSettings,
    Performance,
    InfoHeading,
    InfoArtwork,
    FileLocation,
    InfoLoading,
    Action,
}

impl ControlKind {
    pub const fn default_expression(self) -> &'static str {
        match self {
            Self::LayeredVector2 => crate::timeline_value::VECTOR2_EXPRESSION_DEFAULT,
            Self::LayeredVector3 => crate::timeline_value::VECTOR3_EXPRESSION_DEFAULT,
            _ => crate::timeline_value::SCALAR_EXPRESSION_DEFAULT,
        }
    }
}

/// Storage type is independent of whether the editor displays whole numbers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScalarStorage {
    #[default]
    Float,
    UnsignedInteger,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ControlRowRole {
    #[default]
    Standalone,
    Primary,
    Auxiliary,
    TrailingAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisControlPresentation {
    pub label: String,
    pub progress: f64,
    pub tooltip: AnalysisTooltip,
    pub sensitive: bool,
    pub running: bool,
    pub cancelling: bool,
    pub terminal: bool,
    pub suggested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisTooltip {
    MessageKey(&'static str),
    RawError(String),
}

impl AnalysisTooltip {
    pub fn as_str(&self) -> &str {
        match self {
            Self::MessageKey(message) => message,
            Self::RawError(error) => error,
        }
    }
}

impl AnalysisControlPresentation {
    pub fn active(&self) -> bool {
        self.running || self.cancelling
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectorControlAction {
    RebuildVideoStabilization,
    ClearMaskSource {
        modifier_id: uuid::Uuid,
    },
    SelectObject3dModel {
        modifier_id: uuid::Uuid,
    },
    ClearObject3dModel {
        modifier_id: uuid::Uuid,
    },
    SelectScene3dEnvironment,
    ClearScene3dEnvironment,
    AddDitheringPaletteColor {
        modifier_id: uuid::Uuid,
    },
    RemoveDitheringPaletteColor {
        modifier_id: uuid::Uuid,
        color_id: uuid::Uuid,
    },
    AddPaintPaletteColor,
    RemovePaintPaletteColor {
        color_id: uuid::Uuid,
    },
    SelectPaintTexture {
        color_id: uuid::Uuid,
    },
    ClearPaintTexture {
        color_id: uuid::Uuid,
    },
    RemoveSam2Point {
        modifier_id: uuid::Uuid,
        point_id: uuid::Uuid,
    },
    SetSam2PointLabel {
        modifier_id: uuid::Uuid,
        point_id: uuid::Uuid,
    },
    SetSam2Model {
        modifier_id: uuid::Uuid,
    },
    SetSam2PointPosition {
        modifier_id: uuid::Uuid,
        point_id: uuid::Uuid,
    },
    RemoveSam2Box {
        modifier_id: uuid::Uuid,
        box_id: uuid::Uuid,
    },
    ToggleSam2Analysis {
        modifier_id: uuid::Uuid,
        generation: u64,
        prompt_signature: u64,
        can_analyze: bool,
    },
    RemoveTransparentFillPoint {
        modifier_id: uuid::Uuid,
        point_id: uuid::Uuid,
    },
    ToggleTransparentFillAnalysis {
        modifier_id: uuid::Uuid,
    },
    ToggleCameraAnalysis,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumberSpec {
    pub minimum: f64,
    pub maximum: f64,
    pub drag_step: f64,
    pub digits: i32,
    pub unit: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NumberConstraint {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub integer: bool,
}

impl NumberConstraint {
    pub fn clamp(self, value: f64) -> f64 {
        if !value.is_finite() {
            return value;
        }
        let value = if self.integer { value.round() } else { value };
        let value = self.minimum.map_or(value, |minimum| value.max(minimum));
        self.maximum.map_or(value, |maximum| value.min(maximum))
    }

    pub fn clamp_f32(self, value: f32) -> f32 {
        self.clamp(f64::from(value)) as f32
    }
}

impl Default for NumberSpec {
    fn default() -> Self {
        Self {
            minimum: DEFAULT_MINIMUM,
            maximum: DEFAULT_MAXIMUM,
            drag_step: 1.0,
            digits: 2,
            unit: "",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NumberMapping {
    #[default]
    Linear,
    FocalLengthMillimeters,
}

impl NumberMapping {
    pub fn display(self, stored: f64, store_multiplier: f64) -> f64 {
        match self {
            Self::Linear => {
                stored
                    / if store_multiplier == 0.0 {
                        1.0
                    } else {
                        store_multiplier
                    }
            }
            Self::FocalLengthMillimeters => {
                assert_eq!(
                    store_multiplier, 1.0,
                    "focal-length mapping cannot be scaled"
                );
                shrimply_3dgs_core::focal_length_mm(stored)
            }
        }
    }

    pub fn store(self, displayed: f64, store_multiplier: f64) -> f64 {
        match self {
            Self::Linear => displayed * store_multiplier,
            Self::FocalLengthMillimeters => {
                assert_eq!(
                    store_multiplier, 1.0,
                    "focal-length mapping cannot be scaled"
                );
                shrimply_3dgs_core::vertical_fov_degrees(displayed)
            }
        }
    }
}

use crate::LayeredState;
use crate::keyframe_graph::ScalarGraph;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextKeyframeCommits {
    pub toggle: &'static str,
    pub add: &'static str,
    pub delete: &'static str,
    pub move_keyframe: &'static str,
    pub paste: &'static str,
    pub interpolation: &'static str,
    pub text_interpolation: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyframeCommits {
    pub toggle: &'static str,
    pub edit: &'static str,
    pub add: &'static str,
    pub delete: &'static str,
    pub move_keyframe: &'static str,
    pub paste: &'static str,
    pub interpolation: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorControl {
    pub path: String,
    pub timeline_path: Option<String>,
    pub label: String,
    pub subtitle: String,
    pub tooltip: String,
    /// Scalar numeric values are in display units, like NumberSpec and scalar_graph.
    /// Convert user edits back to storage with store_number.
    pub value: String,
    pub image_bytes: Option<std::sync::Arc<[u8]>>,
    pub components: Vec<String>,
    pub kind: ControlKind,
    pub editable: bool,
    pub sensitive: bool,
    pub visible: bool,
    pub number: NumberSpec,
    pub number_constraint: NumberConstraint,
    pub accepted_range: Option<(f64, f64)>,
    pub integer: bool,
    pub scalar_storage: ScalarStorage,
    pub width_characters: i32,
    pub prefixes: Vec<String>,
    pub suffix: String,
    pub prefix_icon: String,
    pub prefix_icon_rotates: bool,
    pub prefix_icon_rotation_offset_degrees: f64,
    pub values: Vec<String>,
    pub labels: Vec<String>,
    pub icons: Vec<String>,
    pub search_terms: Vec<String>,
    pub layered: LayeredState,
    pub lock: bool,
    pub with_alpha: bool,
    pub store_multiplier: f64,
    pub number_mapping: NumberMapping,
    pub target_id: Option<uuid::Uuid>,
    pub audio_modifier: bool,
    pub commit_name: String,
    pub commit_immediately: bool,
    pub keyframe_commit_name: String,
    pub expression_commit_name: String,
    pub keyframe_commits: Option<KeyframeCommits>,
    pub text_keyframe_commits: Option<TextKeyframeCommits>,
    pub timeline_id: Option<uuid::Uuid>,
    pub scalar_graph: Option<ScalarGraph>,
    pub analysis: Option<AnalysisControlPresentation>,
    pub tts: Option<Box<crate::tts::EditorPresentation>>,
    pub action: Option<InspectorControlAction>,
    pub action_sensitive: bool,
    pub secondary_action: Option<InspectorControlAction>,
    pub action_icon: String,
    pub action_tooltip: String,
    pub drag_payload: String,
    pub row_group: Option<uuid::Uuid>,
    pub row_role: ControlRowRole,
    pub busy: bool,
    pub preview_focus: Option<crate::item::ControlPreviewFocus>,
}

impl InspectorControl {
    pub fn background_integer(&self) -> bool {
        let path = self.timeline_path.as_deref().unwrap_or(&self.path);
        self.integer && (path == "/content/star_points" || path.starts_with("/content/generator/"))
    }
    pub fn new(kind: ControlKind, path: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            timeline_path: None,
            label: label.into(),
            subtitle: String::new(),
            tooltip: String::new(),
            value: String::new(),
            image_bytes: None,
            components: Vec::new(),
            kind,
            editable: true,
            sensitive: true,
            visible: true,
            number: NumberSpec::default(),
            number_constraint: NumberConstraint::default(),
            accepted_range: None,
            integer: false,
            scalar_storage: ScalarStorage::Float,
            width_characters: 8,
            prefixes: Vec::new(),
            suffix: String::new(),
            prefix_icon: String::new(),
            prefix_icon_rotates: false,
            prefix_icon_rotation_offset_degrees: 0.0,
            values: Vec::new(),
            labels: Vec::new(),
            icons: Vec::new(),
            search_terms: Vec::new(),
            layered: LayeredState::default(),
            lock: false,
            with_alpha: true,
            store_multiplier: 1.0,
            number_mapping: NumberMapping::Linear,
            target_id: None,
            audio_modifier: false,
            commit_name: String::new(),
            commit_immediately: false,
            keyframe_commit_name: String::new(),
            expression_commit_name: String::new(),
            keyframe_commits: None,
            text_keyframe_commits: None,
            timeline_id: None,
            scalar_graph: None,
            analysis: None,
            tts: None,
            action: None,
            action_sensitive: true,
            secondary_action: None,
            action_icon: String::new(),
            action_tooltip: String::new(),
            drag_payload: String::new(),
            row_group: None,
            row_role: ControlRowRole::Standalone,
            busy: false,
            preview_focus: None,
        }
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    pub fn immediate_commit(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.keyframe_commit_name.clone_from(&name);
        self.expression_commit_name.clone_from(&name);
        self.commit_name = name;
        self.commit_immediately = true;
        self
    }

    pub fn live_commit(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.keyframe_commit_name.clone_from(&name);
        self.expression_commit_name.clone_from(&name);
        self.commit_name = name;
        self.commit_immediately = false;
        self
    }

    pub fn timeline_commits(
        mut self,
        keyframes: impl Into<String>,
        expression: impl Into<String>,
    ) -> Self {
        self.keyframe_commit_name = keyframes.into();
        self.expression_commit_name = expression.into();
        self
    }

    pub fn text_keyframe_commits(mut self, commits: TextKeyframeCommits) -> Self {
        self.keyframe_commit_name = commits.toggle.to_string();
        self.text_keyframe_commits = Some(commits);
        self
    }

    pub fn keyframe_commits(mut self, commits: KeyframeCommits) -> Self {
        self.keyframe_commit_name = commits.toggle.to_string();
        self.keyframe_commits = Some(commits);
        self
    }

    pub fn action(mut self, action: InspectorControlAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn action_sensitive(mut self, sensitive: bool) -> Self {
        self.action_sensitive = sensitive;
        self
    }

    pub fn secondary_action(mut self, action: InspectorControlAction) -> Self {
        self.secondary_action = Some(action);
        self
    }

    pub fn action_icon(mut self, icon: impl Into<String>, tooltip: impl Into<String>) -> Self {
        self.action_icon = icon.into();
        self.action_tooltip = tooltip.into();
        self
    }

    pub fn drag_payload(mut self, payload: impl Into<String>) -> Self {
        self.drag_payload = payload.into();
        self
    }

    pub fn row_group(mut self, group: uuid::Uuid, role: ControlRowRole) -> Self {
        assert_ne!(
            role,
            ControlRowRole::Standalone,
            "grouped control needs a row role"
        );
        self.row_group = Some(group);
        self.row_role = role;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    pub fn components(mut self, components: Vec<String>) -> Self {
        self.components = components;
        self
    }

    pub fn read_only(mut self) -> Self {
        self.editable = false;
        self
    }

    pub fn sensitive(mut self, sensitive: bool) -> Self {
        self.sensitive = sensitive;
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn number(mut self, number: NumberSpec) -> Self {
        self.number = number;
        self
    }

    pub fn accepted_range(mut self, minimum: f64, maximum: f64) -> Self {
        assert!(minimum <= maximum, "accepted range must be ordered");
        self.accepted_range = Some((minimum, maximum));
        self
    }

    pub fn number_constraint(mut self, constraint: NumberConstraint) -> Self {
        assert!(
            match (constraint.minimum, constraint.maximum) {
                (Some(minimum), Some(maximum)) => minimum <= maximum,
                _ => true,
            },
            "stored number constraint must be ordered",
        );
        self.number_constraint = constraint;
        self
    }

    pub fn integer(mut self) -> Self {
        self.integer = true;
        self.number_constraint.integer = true;
        self
    }

    pub fn unsigned_integer_timeline(mut self) -> Self {
        self.integer = true;
        self.scalar_storage = ScalarStorage::UnsignedInteger;
        self.integer()
    }

    pub fn width_characters(mut self, width: i32) -> Self {
        assert!(width > 0, "control width must be positive");
        self.width_characters = width;
        self
    }

    pub fn prefixes(mut self, prefixes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.prefixes = prefixes.into_iter().map(Into::into).collect();
        self
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = suffix.into();
        self
    }

    pub fn rotating_icon(mut self, icon: impl Into<String>, rotation_offset_degrees: f64) -> Self {
        assert!(
            rotation_offset_degrees.is_finite(),
            "icon rotation offset must be finite",
        );
        self.prefix_icon = icon.into();
        self.prefix_icon_rotates = true;
        self.prefix_icon_rotation_offset_degrees = rotation_offset_degrees;
        self
    }

    pub fn choices(mut self, values: Vec<String>, labels: Vec<String>) -> Self {
        assert_eq!(
            values.len(),
            labels.len(),
            "selector values must have labels"
        );
        self.values = values;
        self.labels = labels;
        self
    }

    pub fn choice_icons(mut self, icons: Vec<String>) -> Self {
        assert_eq!(
            self.values.len(),
            icons.len(),
            "selector values must have icon entries",
        );
        self.icons = icons;
        self
    }

    pub fn choice_search_terms(mut self, search_terms: Vec<String>) -> Self {
        assert_eq!(
            self.values.len(),
            search_terms.len(),
            "selector values must have search entries",
        );
        self.search_terms = search_terms;
        self
    }

    pub fn layered(mut self, timeline_path: impl Into<String>, layered: LayeredState) -> Self {
        self.timeline_path = Some(timeline_path.into());
        self.layered = layered;
        self
    }

    pub fn lock(mut self) -> Self {
        self.lock = true;
        self
    }

    pub fn without_alpha(mut self) -> Self {
        self.with_alpha = false;
        self
    }

    pub fn store_multiplier(mut self, multiplier: f64) -> Self {
        assert!(multiplier.is_finite(), "storage multiplier must be finite");
        self.store_multiplier = multiplier;
        self
    }

    pub fn number_mapping(mut self, mapping: NumberMapping) -> Self {
        self.number_mapping = mapping;
        self
    }

    /// Scale a scalar presentation built in stored units, including its graph.
    /// NumberSpec already describes the desired display units.
    pub fn scale_number_display(mut self, multiplier: f64) -> Self {
        assert!(multiplier.is_finite() && multiplier > 0.0);
        assert!(matches!(
            self.kind,
            ControlKind::Number | ControlKind::LayeredNumber
        ));
        assert_eq!(self.number_mapping, NumberMapping::Linear);
        let value: f64 = self
            .value
            .parse()
            .expect("numeric presentation must contain a number");
        self.value = (value * multiplier).to_string();
        self.store_multiplier /= multiplier;
        if let Some(graph) = &mut self.scalar_graph {
            for point in &mut graph.points {
                point.value *= multiplier;
            }
            for segment in &mut graph.segments {
                segment.start_value *= multiplier;
                segment.end_value *= multiplier;
            }
        }
        self
    }

    pub fn display_number(&self, stored: f64) -> f64 {
        self.number_mapping.display(stored, self.store_multiplier)
    }

    pub fn map_number_for_storage(&self, displayed: f64) -> f64 {
        self.number_mapping.store(displayed, self.store_multiplier)
    }

    pub fn store_number(&self, displayed: f64) -> f64 {
        self.number_constraint
            .clamp(self.map_number_for_storage(displayed))
    }

    pub fn target(mut self, id: uuid::Uuid) -> Self {
        self.target_id = Some(id);
        self
    }

    pub fn timeline(mut self, id: uuid::Uuid, graph: Option<ScalarGraph>) -> Self {
        self.timeline_id = Some(id);
        self.scalar_graph = graph;
        self
    }

    pub fn graph(mut self, graph: Option<ScalarGraph>) -> Self {
        self.scalar_graph = graph;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorSection {
    pub controls: Vec<InspectorControl>,
}

impl InspectorSection {
    pub fn add(&mut self, control: InspectorControl) {
        self.controls.push(control);
    }

    pub fn set_target(&mut self, target_id: uuid::Uuid) {
        self.controls
            .iter_mut()
            .for_each(|control| control.target_id = Some(target_id));
    }

    pub fn set_preview_focus(&mut self, focus: crate::item::ControlPreviewFocus) {
        self.controls
            .iter_mut()
            .for_each(|control| control.preview_focus = Some(focus.clone()));
    }

    pub fn set_sensitive(&mut self, sensitive: bool) {
        for control in &mut self.controls {
            control.sensitive = sensitive;
        }
    }
}
