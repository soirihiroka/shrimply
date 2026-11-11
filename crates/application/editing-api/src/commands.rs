use crate::protocol::*;
use schemars::{JsonSchema, Schema};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub struct Command {
    pub name: &'static str,
    pub path: &'static str,
    pub summary: &'static str,
    pub read_only: bool,
    pub schema: fn() -> Schema,
    pub validate: fn(Value) -> Result<Value, String>,
}

fn validate<T: DeserializeOwned + Serialize>(arguments: Value) -> Result<Value, String> {
    serde_json::from_value::<T>(arguments)
        .and_then(serde_json::to_value)
        .map_err(|error| error.to_string())
}

fn schema<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

macro_rules! command {
    ($name:literal, $path:literal, $summary:literal, $request:ty, $read:literal) => {
        Command {
            name: $name,
            path: $path,
            summary: $summary,
            read_only: $read,
            schema: schema::<$request>,
            validate: validate::<$request>,
        }
    };
}

pub static COMMANDS: &[Command] = &[
    command!(
        "project_serve",
        "project serve",
        "Own a project in a foreground headless session until project close",
        GetEditorStateRequest,
        false
    ),
    command!(
        "render_project",
        "render",
        "Render a saved project to video or GIF without opening an editor",
        RenderProjectRequest,
        false
    ),
    command!(
        "create_project",
        "project create",
        "Create a native .shrimp project without overwriting an existing file",
        CreateProjectRequest,
        false
    ),
    command!(
        "connect_project",
        "project open",
        "Open a project in the native editor or start a managed headless session",
        ConnectProjectRequest,
        false
    ),
    command!(
        "get_editor_state",
        "editor get",
        "Return live project, playhead, selection, active scope, and track state",
        GetEditorStateRequest,
        true
    ),
    command!(
        "get_version",
        "version",
        "Return the numbered version and full Git commit hash of this CLI and the connected editor. Works without a connected project; editor is then null",
        GetVersionRequest,
        true
    ),
    command!(
        "list_scopes",
        "scopes list",
        "List root and concrete folded-sequence presentation scopes with tracks",
        ListScopesRequest,
        true
    ),
    command!(
        "query_clips",
        "clips list",
        "Query live clip presentations. The optional half-open frame range is stateless and independent from editor selection",
        QueryClipsRequest,
        true
    ),
    command!(
        "get_clip",
        "clips get",
        "Get full live metadata for one concrete clip address or every presentation of an item UUID",
        GetClipRequest,
        true
    ),
    command!(
        "get_clip_info",
        "clips inspect",
        "Return a deep inspector-style dump for a clip, including filesystem, container, stream, tag, artwork, and image EXIF metadata. Embedded artwork bytes are returned as image content only when include_artwork is true",
        GetClipInfoRequest,
        true
    ),
    command!(
        "get_manim_clip",
        "manim get",
        "Inspect a Manim clip's discovered scenes, reflected parameter controls and values, and current render error",
        GetManimClipRequest,
        true
    ),
    command!(
        "get_track",
        "tracks get",
        "Return one fully addressed track and up to 500 of its clips in projected timeline order",
        GetTrackRequest,
        true
    ),
    command!(
        "query_expressions",
        "expressions list",
        "Query expression IDs, property paths, enabled state, and source across live clip metadata",
        QueryExpressionsRequest,
        true
    ),
    command!(
        "seek_playhead",
        "player seek",
        "Seek the visible editor playhead to a project frame",
        SeekPlayheadRequest,
        false
    ),
    command!(
        "view_frame",
        "frames render",
        "Render a project frame with Shrimply's native compositor and return it as a PNG without changing the playhead",
        ViewFrameRequest,
        false
    ),
    command!(
        "analyze_transparent_fill",
        "transparent-fill analyze",
        "Analyze a Transparent Fill modifier and wait until all frame masks are cached",
        AnalyzeTransparentFillRequest,
        false
    ),
    command!(
        "create_track",
        "tracks create",
        "Create one caption, video, or audio track as an explicit undoable edit",
        CreateTrackRequest,
        false
    ),
    command!(
        "insert_files",
        "clips insert-files",
        "Import one or more native files atomically. Omitted targets choose an existing compatible track with room and never create one unless collision=new_track is explicit. Copying into project media is the preferred default; link=true retains external paths",
        InsertFilesRequest,
        false
    ),
    command!(
        "insert_captions",
        "clips insert-captions",
        "Bulk-insert captions into an existing root caption track or create a new one, with exact frame ranges, optional language, collision handling, and source style copying",
        InsertCaptionsRequest,
        false
    ),
    command!(
        "insert_tts",
        "clips insert-tts",
        "Insert an empty text-to-speech item on an explicit or automatically selected audio track for later inspector configuration",
        InsertTtsRequest,
        false
    ),
    command!(
        "insert_text",
        "clips insert-text",
        "Insert a native editable text video clip at an exact projected frame and duration, choosing a video track with room or creating one when omitted",
        InsertTextRequest,
        false
    ),
    command!(
        "list_tts_models",
        "tts models list",
        "List text-to-speech models and their dynamic input definitions from the editor's current compute server",
        ListTtsModelsRequest,
        true
    ),
    command!(
        "list_stt_models",
        "stt models list",
        "List speech-to-text models advertised by the editor's current compute server",
        ListSttModelsRequest,
        true
    ),
    command!(
        "transcribe_audio",
        "stt transcribe",
        "Transcribe one audio track or one or more fully addressed audio clips with the current compute server and insert timed captions on a new track",
        TranscribeAudioRequest,
        false
    ),
    command!(
        "generate_tts",
        "tts generate",
        "Generate speech with the editor's current compute server and insert it on an explicit or automatically selected audio track",
        GenerateTtsRequest,
        false
    ),
    command!(
        "move_clip",
        "clips move",
        "Move a fully addressed clip to a projected frame and optional compatible track/scope",
        MoveClipRequest,
        false
    ),
    command!(
        "trim_clip",
        "clips trim",
        "Trim a fully addressed clip using projected frame bounds while preserving source offset",
        TrimClipRequest,
        false
    ),
    command!(
        "delete_clips",
        "clips delete",
        "Delete fully addressed clips as one undoable history action",
        DeleteClipsRequest,
        false
    ),
    command!(
        "set_clip_properties",
        "clips set",
        "Set typed clip properties: caption text, audio enabled/gain, or video/audio playback speed/repeat strategy",
        SetClipPropertiesRequest,
        false
    ),
    command!(
        "set_video_transform",
        "clips transform",
        "Set constant position, scale, rotation, anchor, shear, fit mode, and/or raster crop on a video clip. Supplied transform fields replace any keyframes on those fields",
        SetVideoTransformRequest,
        false
    ),
    command!(
        "upsert_keyframes",
        "keyframes upsert",
        "Bulk-create or replace scalar or vec2 keyframes by JSON property path and projected timeline frame",
        UpsertKeyframesRequest,
        false
    ),
    command!(
        "delete_keyframes",
        "keyframes delete",
        "Delete scalar or vec2 keyframes by JSON property path and projected timeline frame; deleting the last keyframe preserves the property's first keyed value as a constant",
        DeleteKeyframesRequest,
        false
    ),
    command!(
        "upsert_property_expression",
        "property-expressions upsert",
        "Create or update an expression directly on a scalar or vec2 TimelineValue addressed by its get_clip JSON property path",
        UpsertPropertyExpressionRequest,
        false
    ),
    command!(
        "delete_property_expression",
        "property-expressions delete",
        "Remove the expression from a scalar or vec2 TimelineValue addressed by its get_clip JSON property path",
        DeletePropertyExpressionRequest,
        false
    ),
    command!(
        "set_clip_transitions",
        "clips set-transitions",
        "Add, edit, or remove per-clip intro/outro transitions. Each update names one side; a null transition removes it",
        SetClipTransitionsRequest,
        false
    ),
    command!(
        "set_manim_clip",
        "manim set",
        "Set a Manim clip's discovered scene and/or typed reflected parameter overrides as one validated undoable edit; null resets a parameter to its scene default",
        SetManimClipRequest,
        false
    ),
    command!(
        "reload_manim_source",
        "manim reload",
        "Invalidate a Manim clip's cached Python scene state and rebuild it from the current source file",
        ReloadManimSourceRequest,
        false
    ),
    command!(
        "set_expression",
        "expressions set",
        "Set an expression's source and/or enabled state by stable ID on its owning video or audio clip",
        SetExpressionRequest,
        false
    ),
    command!(
        "set_track_enabled",
        "tracks set-enabled",
        "Enable or disable a fully addressed caption, visual, or audio track",
        SetTrackEnabledRequest,
        false
    ),
    command!(
        "set_caption_track_language",
        "tracks set-language",
        "Set or clear a caption track's CLDR locale identifier, such as en_US, en_GB, or ja_JP",
        SetCaptionTrackLanguageRequest,
        false
    ),
    command!(
        "delete_track",
        "tracks delete",
        "Delete a fully addressed caption, visual, or audio track and all of its clips",
        DeleteTrackRequest,
        false
    ),
    command!(
        "run_edit_script",
        "edit apply",
        "Run an ordered typed edit program atomically as one editing edit script history action",
        RunEditScriptRequest,
        false
    ),
    command!(
        "project_get",
        "project get",
        "Get project name, canvas, frame rate, duration, and content revision",
        GetEditorStateRequest,
        true
    ),
    command!(
        "project_set",
        "project set",
        "Update project name, canvas size, or an empty timeline frame rate",
        ProjectSettingsRequest,
        false
    ),
    command!(
        "project_save",
        "project save",
        "Persist the current project",
        GetEditorStateRequest,
        false
    ),
    command!(
        "project_close",
        "project close",
        "Save and close a managed headless session",
        GetEditorStateRequest,
        false
    ),
    command!(
        "history_undo",
        "history undo",
        "Undo one project edit and save",
        GetEditorStateRequest,
        false
    ),
    command!(
        "history_redo",
        "history redo",
        "Redo one project edit and save",
        GetEditorStateRequest,
        false
    ),
    command!(
        "split_clip",
        "clips split",
        "Split a clip at an exact timeline frame",
        SplitClipRequest,
        false
    ),
    command!(
        "duplicate_clip",
        "clips duplicate",
        "Duplicate a clip with fresh IDs at a timeline frame",
        DuplicateClipRequest,
        false
    ),
    command!(
        "reorder_track",
        "tracks reorder",
        "Move a track to a zero-based index within its kind and scope",
        ReorderTrackRequest,
        false
    ),
    command!(
        "replace_text",
        "text replace",
        "Find and replace caption, comment, and animated text strings; supports sed substitution and regex captures",
        TextReplaceRequest,
        false
    ),
    command!(
        "tracks_list",
        "tracks list",
        "List all tracks, including empty tracks, in root and nested scopes",
        ListScopesRequest,
        true
    ),
    command!(
        "player_get",
        "player get",
        "Read playhead position, duration, and playback state",
        GetEditorStateRequest,
        true
    ),
];

pub fn get(name: &str) -> Result<&'static Command, String> {
    COMMANDS
        .iter()
        .find(|command| command.name == name || command.path == name)
        .ok_or_else(|| {
            format!("unknown command {name:?}; use shrimply cli search to find an operation")
        })
}
