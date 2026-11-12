use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shrimply_math_core::Time;
use shrimply_project_document::project::{
    Interpolation, Project, TransitionSide, VisualTransitionKind,
};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExactFraction {
    pub numerator: i64,
    pub denominator: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FrameTime {
    pub frame: u64,
    pub seconds: ExactFraction,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FrameRange {
    pub start_frame: u64,
    pub end_frame: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScopeRef {
    /// Concrete folded-sequence presenter item IDs. Empty means the root scope.
    #[serde(default)]
    pub sequence_path: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Comment,
    Caption,
    Video,
    Audio,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TrackAddress {
    pub kind: ClipKind,
    #[serde(default)]
    pub sequence_path: Vec<String>,
    pub track_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ClipAddress {
    pub kind: ClipKind,
    #[serde(default)]
    pub sequence_path: Vec<String>,
    pub track_id: String,
    pub item_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub position: Time,
    pub duration: Time,
    pub playing: bool,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActiveScopeSnapshot {
    pub instance_path: Vec<String>,
    pub video_paths: Vec<ScopeRef>,
    pub audio_paths: Vec<ScopeRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiveSnapshot {
    pub project_path: String,
    pub project: Project,
    pub player: PlayerSnapshot,
    pub active_scope: ActiveScopeSnapshot,
    pub focused_item: Option<ClipAddress>,
    pub selected_items: Vec<ClipAddress>,
    pub focused_track: Option<TrackAddress>,
    pub selected_tracks: Vec<TrackAddress>,
    pub asset_revisions: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiCall {
    pub name: String,
    #[serde(default = "empty_arguments")]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    pub if_revision: Option<String>,
}

fn empty_arguments() -> Value {
    serde_json::json!({})
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettingsRequest {
    pub name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<ExactFraction>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplitClipRequest {
    pub address: ClipAddress,
    pub frame: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateClipRequest {
    pub address: ClipAddress,
    pub start_frame: u64,
    pub destination: Option<TrackAddress>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReorderTrackRequest {
    pub address: TrackAddress,
    /// Zero-based destination within the same track kind and sequence scope.
    pub index: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextReplaceRequest {
    pub addresses: Option<Vec<ClipAddress>>,
    pub track_id: Option<String>,
    pub range: Option<FrameRange>,
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub all: bool,
    pub find: Option<String>,
    pub replace: Option<String>,
    /// Rust regex syntax in a sed-style s/pattern/replacement/gi expression.
    pub expression: Option<String>,
    #[serde(default)]
    pub global: bool,
    #[serde(default)]
    pub ignore_case: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeRequest {
    pub project_path: String,
    pub command: BridgeCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command", content = "payload")]
pub enum BridgeCommand {
    Handshake,
    Version,
    Snapshot,
    Execute(ApiCall),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RangeMatch {
    #[default]
    Overlaps,
    Contained,
    StartsIn,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectProjectRequest {
    /// Existing native .shrimp project. Reuses its owner when already open.
    pub project_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    /// Absolute path for the new native .shrimp video project.
    pub project_path: String,
    /// Defaults to "Untitled Project".
    pub name: Option<String>,
    /// Defaults to 1920.
    pub width: Option<u32>,
    /// Defaults to 1080.
    pub height: Option<u32>,
    /// Defaults to 30/1 and must match a frame rate supported by Shrimply.
    pub fps: Option<ExactFraction>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEditorStateRequest {}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetVersionRequest {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct BuildVersion {
    pub version: String,
    /// Full source commit SHA; absent only when source provenance was unavailable at build time.
    pub git_hash: Option<String>,
}

impl BuildVersion {
    pub fn current(version: &str) -> Self {
        Self {
            version: version.into(),
            git_hash: option_env!("SHRIMPLY_BUILD_GIT_HASH").map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct VersionResponse {
    pub cli: BuildVersion,
    pub editor: Option<BuildVersion>,
    /// Includes errors from an older editor that does not implement this query.
    pub editor_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListScopesRequest {}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryClipsRequest {
    /// Return every match; conflicts with offset and limit. Otherwise returns up to 100.
    #[serde(default)]
    pub all: bool,
    /// Defaults to the root sequence scope.
    pub scope: Option<ScopeRef>,
    /// Include clips inside folded sequences; defaults to false.
    #[serde(default)]
    pub recursive: bool,
    pub kind: Option<ClipKind>,
    /// Filter the source type, for example tts, media, caption, or text.
    pub source_kind: Option<String>,
    pub track_id: Option<String>,
    pub item_id: Option<String>,
    pub enabled: Option<bool>,
    pub caption_text: Option<String>,
    pub source_filename: Option<String>,
    /// Independent, stateless half-open frame range selector.
    pub range: Option<FrameRange>,
    #[serde(default)]
    pub range_match: RangeMatch,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetClipRequest {
    pub address: Option<ClipAddress>,
    pub item_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetClipInfoRequest {
    pub address: Option<ClipAddress>,
    pub item_id: Option<String>,
    /// Include embedded artwork metadata, without image bytes.
    #[serde(default)]
    pub include_artwork: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetManimClipRequest {
    pub address: ClipAddress,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ManimColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ManimParameterValue {
    Integer(i64),
    Float(f64),
    Fraction(ExactFraction),
    Color(ManimColor),
    Option(String),
    Boolean(bool),
    String(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetManimClipRequest {
    pub address: ClipAddress,
    /// Select a discovered scene. Changing scenes clears all parameter overrides.
    pub scene: Option<String>,
    /// Set an override by reflected key, or use null to reset that key to its scene default.
    #[serde(default)]
    pub parameters: BTreeMap<String, Option<ManimParameterValue>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReloadManimSourceRequest {
    pub address: ClipAddress,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ManimClipResponse {
    pub address: ClipAddress,
    pub source: String,
    pub source_revision: u64,
    pub scene: String,
    pub scenes: Vec<String>,
    /// True once the current scene has rendered and reflected its controls.
    pub parameters_ready: bool,
    pub parameters: Vec<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReloadManimSourceResponse {
    pub address: ClipAddress,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTrackRequest {
    pub address: TrackAddress,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryExpressionsRequest {
    pub address: Option<ClipAddress>,
    pub source_contains: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeekPlayheadRequest {
    pub frame: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewFrameRequest {
    pub frame: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewFrameResponse {
    pub frame: FrameTime,
    pub png: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeTransparentFillRequest {
    pub address: ClipAddress,
    pub modifier_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeTransparentFillResponse {
    pub address: ClipAddress,
    pub modifier_id: String,
    pub analysis_generation: u64,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollisionBehavior {
    #[default]
    Reject,
    NewTrack,
    Overwrite,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct InitialClipProperties {
    pub text: Option<String>,
    pub enabled: Option<bool>,
    pub gain_db: Option<f32>,
    pub playback_speed: Option<ExactFraction>,
    pub repeat_strategy: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportEntry {
    pub source: String,
    #[serde(default)]
    pub offset_frames: i64,
    /// Optional explicit compatible tracks. When omitted, an existing track with room is chosen.
    #[serde(default)]
    pub targets: Vec<TrackAddress>,
    #[serde(default)]
    pub properties: InitialClipProperties,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertFilesRequest {
    pub files: Vec<ImportEntry>,
    pub frame: Option<u64>,
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub link: bool,
    pub copy_root: Option<String>,
    /// new_track reuses a compatible track with room before creating one.
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceClipSourceRequest {
    /// Existing video clip; all timeline placement and effect properties are retained.
    pub address: ClipAddress,
    /// New local source file, such as a .sanim, image or video, with exactly one visual stream.
    pub source: String,
    /// Retain an external source path instead of copying it into project media.
    #[serde(default)]
    pub link: bool,
    /// Copy this directory with the source to retain sibling assets/imports. Conflicts with link.
    pub copy_root: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionCueInput {
    pub start_frame: u64,
    pub end_frame: u64,
    pub text: String,
    /// Optional caption whose styling and layout should be copied.
    pub copy_style_from: Option<ClipAddress>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertCaptionsRequest {
    /// Existing root caption track. Omit to create a new caption track.
    pub track: Option<TrackAddress>,
    pub captions: Vec<CaptionCueInput>,
    /// CLDR locale identifier such as en_US, zh_CN, or ja_JP.
    pub language: Option<String>,
    /// Sets the resolved track state. New tracks default to enabled.
    pub enabled: Option<bool>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertTtsRequest {
    /// Existing audio track. Omit to choose an audio track with room.
    pub track: Option<TrackAddress>,
    /// Defaults to the edit-script anchor or current playhead.
    pub frame: Option<u64>,
    /// Defaults to the editor's configured visual item duration.
    pub duration_frames: Option<u64>,
    /// Defaults to the edit-script scope or root sequence.
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertTextRequest {
    pub text: String,
    /// Existing video track. Omit to choose one with room or create a new track.
    pub track: Option<TrackAddress>,
    /// Defaults to the edit-script anchor or current playhead.
    pub frame: Option<u64>,
    pub duration_frames: u64,
    /// Defaults to the edit-script scope or root sequence.
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertTextOperation {
    pub text: String,
    pub track: Option<TrackAddress>,
    pub duration_frames: u64,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TtsInputValue {
    Text { value: String },
    Select { value: String },
    Audio { path: String },
    Toggle { value: bool },
    Number { value: ExactFraction },
    Table { rows: Vec<BTreeMap<String, String>> },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTtsModelsRequest {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListTtsModelsResponse {
    pub models: Vec<Value>,
    pub default_model: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSttModelsRequest {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListSttModelsResponse {
    pub models: Vec<String>,
    pub default_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TranscribeAudioRequest {
    /// Transcribe every audio clip on this track. Provide exactly one of track or clips.
    pub track: Option<TrackAddress>,
    /// Transcribe these fully addressed audio clips. Provide exactly one of track or clips.
    #[serde(default)]
    pub clips: Vec<ClipAddress>,
    /// Defaults to the editor's last selected model, then the server's first STT model.
    pub model: Option<String>,
    /// Optional CLDR locale identifier for the new caption track.
    pub language: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateTtsRequest {
    pub text: String,
    /// Defaults to the editor's last selected model, then the server's first model.
    pub model: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, TtsInputValue>,
    /// Existing audio track. Omit to choose an audio track with room.
    pub track: Option<TrackAddress>,
    /// Defaults to the current playhead.
    pub frame: Option<u64>,
    /// Defaults to the root sequence scope.
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveClipRequest {
    pub address: ClipAddress,
    /// Absolute projected frame. Provide exactly one of this and offset_frames.
    pub start_frame: Option<u64>,
    /// Signed offset from the edit-script anchor frame.
    pub offset_frames: Option<i64>,
    pub destination: Option<TrackAddress>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrimClipRequest {
    pub address: ClipAddress,
    pub start_frame: Option<u64>,
    pub end_frame: Option<u64>,
    /// Signed replacement start relative to the edit-script anchor.
    pub start_offset_frames: Option<i64>,
    /// Signed replacement end relative to the edit-script anchor.
    pub end_offset_frames: Option<i64>,
    #[serde(default)]
    pub collision: CollisionBehavior,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteClipsRequest {
    pub addresses: Vec<ClipAddress>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClipPropertiesRequest {
    pub address: ClipAddress,
    pub text: Option<String>,
    pub enabled: Option<bool>,
    pub gain_db: Option<f32>,
    pub playback_speed: Option<ExactFraction>,
    pub repeat_strategy: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Vec2Value {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VideoFitMode {
    Natural,
    Contain,
    Cover,
    Stretch,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CropMode {
    Percentage,
    Pixels,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CropEdges {
    pub mode: CropMode,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVideoTransformRequest {
    pub address: ClipAddress,
    /// Applied first; explicitly supplied fields then override the fitted transform.
    pub fit_mode: Option<VideoFitMode>,
    pub position: Option<Vec2Value>,
    pub scale: Option<Vec2Value>,
    pub rotation_degrees: Option<f32>,
    pub anchor: Option<Vec2Value>,
    pub shear: Option<Vec2Value>,
    /// Adds or updates the clip's raster Crop modifier.
    pub crop: Option<CropEdges>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum KeyframeValue {
    Scalar(f32),
    Vec2(Vec2Value),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct KeyframeInput {
    /// Projected timeline frame, converted to the addressed clip's local animation time.
    pub frame: u64,
    pub value: KeyframeValue,
    pub interpolation: Interpolation,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertKeyframesRequest {
    pub address: ClipAddress,
    /// JSON Pointer to a scalar or vec2 TimelineValue in get_clip metadata.
    pub property_path: String,
    pub keyframes: Vec<KeyframeInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteKeyframesRequest {
    pub address: ClipAddress,
    /// JSON Pointer to a scalar or vec2 TimelineValue in get_clip metadata.
    pub property_path: String,
    /// Projected timeline frames to delete.
    pub frames: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertPropertyExpressionRequest {
    pub address: ClipAddress,
    /// JSON Pointer to a scalar or vec2 TimelineValue in get_clip metadata.
    pub property_path: String,
    pub source: String,
    /// Defaults to true for a newly created expression.
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletePropertyExpressionRequest {
    pub address: ClipAddress,
    /// JSON Pointer to a scalar or vec2 TimelineValue in get_clip metadata.
    pub property_path: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipTransitionInput {
    pub duration_frames: u64,
    /// Visual clips default to fade. Audio clips only support fade.
    pub kind: Option<VisualTransitionKind>,
    pub interpolation: Option<Interpolation>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipTransitionUpdate {
    pub side: TransitionSide,
    /// Null removes the transition on this side.
    pub transition: Option<ClipTransitionInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetClipTransitionsRequest {
    pub address: ClipAddress,
    pub updates: Vec<ClipTransitionUpdate>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetExpressionRequest {
    pub address: ClipAddress,
    pub expression_id: String,
    pub source: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTrackEnabledRequest {
    pub address: TrackAddress,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCaptionTrackLanguageRequest {
    pub address: TrackAddress,
    /// CLDR locale identifier such as en_US, en_GB, or ja_JP. Null clears it.
    pub language: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteTrackRequest {
    pub address: TrackAddress,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTrackRequest {
    pub kind: ClipKind,
    pub scope: Option<ScopeRef>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTrackOperation {
    pub kind: ClipKind,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "args")]
pub enum EditOperation {
    SetProjectSettings(ProjectSettingsRequest),
    SplitClip(SplitClipRequest),
    DuplicateClip(DuplicateClipRequest),
    ReorderTrack(ReorderTrackRequest),
    ReplaceText(TextReplaceRequest),
    InsertFiles(InsertFilesRequest),
    ReplaceClipSource(ReplaceClipSourceRequest),
    InsertCaptions(InsertCaptionsRequest),
    InsertTts(InsertTtsRequest),
    InsertText(InsertTextOperation),
    CreateTrack(CreateTrackOperation),
    MoveClip(MoveClipRequest),
    TrimClip(TrimClipRequest),
    DeleteClips(DeleteClipsRequest),
    SetClipProperties(SetClipPropertiesRequest),
    SetVideoTransform(SetVideoTransformRequest),
    UpsertKeyframes(UpsertKeyframesRequest),
    DeleteKeyframes(DeleteKeyframesRequest),
    UpsertPropertyExpression(UpsertPropertyExpressionRequest),
    DeletePropertyExpression(DeletePropertyExpressionRequest),
    SetClipTransitions(SetClipTransitionsRequest),
    SetExpression(SetExpressionRequest),
    SetTrackEnabled(SetTrackEnabledRequest),
    SetCaptionTrackLanguage(SetCaptionTrackLanguageRequest),
    DeleteTrack(DeleteTrackRequest),
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunEditScriptRequest {
    pub frame: Option<u64>,
    pub scope: Option<ScopeRef>,
    pub operations: Vec<EditOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub history_label: String,
    pub frame: Option<u64>,
    pub scope: Option<ScopeRef>,
    pub operations: Vec<EditOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimeSpan {
    pub start: FrameTime,
    pub end: FrameTime,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrackSummary {
    pub address: TrackAddress,
    pub enabled: bool,
    /// Set only for caption tracks.
    pub language: Option<String>,
    pub clip_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrackMetadata {
    pub track: TrackSummary,
    pub clips: Vec<ClipSummary>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipSummary {
    pub address: ClipAddress,
    pub label: String,
    pub source_kind: String,
    pub asset_path: Option<String>,
    pub enabled: bool,
    pub local: TimeSpan,
    pub projected: TimeSpan,
    pub state: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AssetMetadata {
    pub path: String,
    pub canonical_path: Option<String>,
    pub exists: bool,
    pub size: Option<u64>,
    pub modified_unix_seconds: Option<u64>,
    pub asset_revision: Option<u64>,
    pub inside_project_media: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipMetadata {
    pub metadata: Value,
    pub owning_track: TrackSummary,
    pub asset: Option<AssetMetadata>,
    pub presentations: Vec<ClipSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipInfo {
    pub clip: ClipMetadata,
    pub source: Option<shrimply_media_info::FileInfo>,
    pub selected_stream_index: Option<usize>,
    pub source_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CanvasSummary {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditorState {
    pub project_path: String,
    pub project_name: String,
    pub fps: ExactFraction,
    pub canvas: CanvasSummary,
    pub duration: FrameTime,
    pub playhead: FrameTime,
    pub playing: bool,
    pub revision: u64,
    pub active_scope: ActiveScopeSummary,
    pub focused_item: Option<ClipAddress>,
    pub selected_items: Vec<ClipAddress>,
    pub focused_track: Option<TrackAddress>,
    pub selected_tracks: Vec<TrackAddress>,
    pub tracks: Vec<TrackSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ActiveScopeSummary {
    pub instance_path: Vec<String>,
    pub video_presentations: Vec<ScopeRef>,
    pub audio_presentations: Vec<ScopeRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScopeSummary {
    pub scope: ScopeRef,
    pub tracks: Vec<TrackSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListScopesResponse {
    pub scopes: Vec<ScopeSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryClipsResponse {
    pub clips: Vec<ClipSummary>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExpressionSummary {
    pub address: ClipAddress,
    pub expression_id: String,
    /// JSON Pointer into the clip metadata returned by get_clip.
    pub property_path: String,
    pub enabled: bool,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryExpressionsResponse {
    pub expressions: Vec<ExpressionSummary>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProjectClipsResource {
    pub clips: Vec<ClipSummary>,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditOperationResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub index: usize,
    pub operation: String,
    pub changed_addresses: Vec<ClipAddress>,
    pub deleted_addresses: Vec<ClipAddress>,
    pub changed_tracks: Vec<TrackAddress>,
    pub presentations: Vec<ClipSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditResponse {
    pub operations: Vec<EditOperationResult>,
    pub duration: FrameTime,
    pub revision: u64,
}
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderProjectRequest {
    /// Destination file. Existing files require overwrite=true.
    pub output: String,
    /// h264, h265, or gif.
    pub codec: Option<String>,
    /// mp4, mkv, or gif; defaults to the output extension.
    pub container: Option<String>,
    /// Positive exact rate; defaults to the project frame rate. Named flags also accept decimals or ratios such as 30000/1001.
    pub fps: Option<ExactFraction>,
    /// Video bitrate in Kbps; defaults to 10000.
    pub video_bitrate: Option<u32>,
    /// aac (default), fdk-aac, or opus; macOS also supports audiotoolbox-aac.
    pub audio_encoder: Option<String>,
    /// Audio bitrate in Kbps; defaults to 192.
    pub audio_bitrate: Option<u32>,
    /// Audio sample rate in Hz; defaults to 48000.
    pub audio_sample_rate: Option<u32>,
    /// Replace an existing destination file.
    #[serde(default)]
    pub overwrite: bool,
}
