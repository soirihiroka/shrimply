use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rmcp::handler::server::wrapper::Json;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, handler::server::wrapper::Parameters,
    model::*, service::RequestContext, tool, tool_handler, tool_router,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use shrimply_project_document::project::{
    AudioTrack, COMMON_FRAME_RATES, CaptionTrack, DEFAULT_CANVAS_SIZE, DEFAULT_PROJECT_FPS,
    Project, VisualTrack, fraction_new,
};

use crate::bridge::{Bridge, BridgeError};
use crate::protocol::*;
use crate::query;

const MCP_INSTRUCTIONS: &str = "Shrimply MCP controls the Shrimply video editor for creating, inspecting, and editing its native .shrimp video project files. Call create_project with an absolute new .shrimp path to create, open, and connect a blank project, or call connect_project with a .shrimp file already open in Shrimply. Editing tools and resources operate on the connected editor's live in-memory project rather than editing its file directly.";

const DEFAULT_PROJECT_NAME: &str = "Untitled Project";
const MAX_CANVAS_DIMENSION: u32 = 16_384;

const EDIT_API: &str = r#"Shrimply MCP controls the Shrimply video editor for native .shrimp video project files.
Call create_project to create, open, and connect a blank project, or call connect_project with the
absolute path of a .shrimp file already open in Shrimply. Edits operate on the connected editor's
live in-memory project, not directly on the file.
All public times are zero-based integer frames. Clip and track mutations require full concrete
addresses. Direct edits create one undoable history action. run_edit_script validates its ordered,
typed operations against a clone and installs them atomically as one history action. File imports
copy into media/imported/<uuid> by default; set link=true explicitly to retain external paths.
Imports without targets use an existing compatible track with room and do not create tracks.
Use create_track explicitly, or collision=new_track to allow insertion to create one as a fallback.
Undo removes imported clips while retaining their durable project-media files so redo remains valid.
Caption text is the text field of set_clip_properties. Query expressions to obtain their stable IDs,
then use set_expression with the owning clip address and expression ID. Expression edits can also be
included in run_edit_script for one atomic, undoable history action.
set_video_transform sets constant transform values, fit mode, and raster crop. upsert_keyframes and
delete_keyframes address scalar or vec2 TimelineValues by the JSON Pointer shown in get_clip metadata;
their frames are projected timeline frames. upsert_property_expression creates or updates an
expression directly on the same property path, and delete_property_expression removes it.
set_clip_transitions applies a list of intro/outro updates; a null transition removes that side.
insert_captions bulk-inserts exact frame ranges into an existing caption track, or creates a new
track when track is omitted. It can set the track language and copy styling from source captions.
insert_text creates native editable text video clips with the project's standard text defaults.
insert_tts creates an empty TTS audio item for inspector configuration. list_tts_models describes
the current compute server's model inputs, and generate_tts synthesizes and inserts speech directly.
list_stt_models lists the current compute server's speech-to-text models. transcribe_audio accepts
one audio track or one or more fully addressed audio clips and inserts timed captions on a new track.
get_track returns one fully addressed track and up to 500 timeline-ordered clips in one call.
Python files import as Manim video clips. get_manim_clip returns discovered scenes, reflected
parameter controls, and the current render error. set_manim_clip validates scene and parameter
changes. reload_manim_source invalidates cached scene states and rebuilds from the Python source.

Example direct move:
{"address":{"kind":"video","sequence_path":[],"track_id":"…","item_id":"…"},"start_frame":120}

Example script:
{"frame":120,"operations":[{"type":"move_clip","args":{"address":{"kind":"video","sequence_path":[],"track_id":"…","item_id":"…"},"offset_frames":24,"collision":"reject"}}]}"#;

#[derive(Clone, Default)]
pub struct ShrimplyServer {
    bridge: Arc<RwLock<Option<Bridge>>>,
}

impl ShrimplyServer {
    pub fn new() -> Self {
        Self::default()
    }

    fn connected_bridge(&self) -> Result<Bridge, McpError> {
        self.bridge
            .read()
            .expect("Shrimply MCP project connection lock was poisoned")
            .clone()
            .ok_or_else(|| {
                mcp_error("no project is connected; call create_project or connect_project first")
            })
    }

    async fn request(
        &self,
        command: BridgeCommand,
        context: &RequestContext<RoleServer>,
    ) -> Result<serde_json::Value, McpError> {
        let bridge = self.connected_bridge()?;
        let canceled = Arc::new(AtomicBool::new(false));
        let worker_canceled = canceled.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            bridge.request_with_cancel(command, worker_canceled)
        });
        tokio::select! {
            result = &mut worker => result
                .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                .map_err(bridge_error),
            () = context.ct.cancelled() => {
                canceled.store(true, Ordering::Release);
                worker.await
                    .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                    .map_err(bridge_error)
            }
        }
    }

    async fn snapshot(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<LiveSnapshot, McpError> {
        let value = self.request(BridgeCommand::Snapshot, context).await?;
        serde_json::from_value(value).map_err(|error| {
            internal_error(format!("editor returned an invalid snapshot: {error}"))
        })
    }

    async fn edit(
        &self,
        request: EditRequest,
        context: &RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        let value = self.request(BridgeCommand::Apply(request), context).await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!("editor returned an invalid edit result: {error}"))
        })
    }
}

#[tool_router]
impl ShrimplyServer {
    #[tool(
        description = "Create a new native .shrimp video project without overwriting an existing file, open it in Shrimply, and connect this MCP session to it"
    )]
    async fn create_project(
        &self,
        Parameters(request): Parameters<CreateProjectRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<CreateProjectResponse>, McpError> {
        let project_path = PathBuf::from(request.project_path);
        if !project_path.is_absolute() {
            return Err(mcp_error("project_path must be an absolute path"));
        }
        let name = request
            .name
            .as_deref()
            .unwrap_or(DEFAULT_PROJECT_NAME)
            .trim()
            .to_string();
        if name.is_empty() {
            return Err(mcp_error("name must not be empty"));
        }
        let width = request.width.unwrap_or(DEFAULT_CANVAS_SIZE.width);
        let height = request.height.unwrap_or(DEFAULT_CANVAS_SIZE.height);
        if !(1..=MAX_CANVAS_DIMENSION).contains(&width)
            || !(1..=MAX_CANVAS_DIMENSION).contains(&height)
        {
            return Err(mcp_error(format!(
                "width and height must be between 1 and {MAX_CANVAS_DIMENSION}"
            )));
        }
        let fps = match request.fps {
            None => DEFAULT_PROJECT_FPS,
            Some(fps) if fps.numerator <= 0 || fps.denominator <= 0 => {
                return Err(mcp_error("fps numerator and denominator must be positive"));
            }
            Some(fps) => {
                let value = fraction_new(fps.numerator, fps.denominator);
                if !COMMON_FRAME_RATES.iter().any(|rate| rate.value == value) {
                    return Err(mcp_error(
                        "fps must match one of Shrimply's supported frame rates",
                    ));
                }
                value
            }
        };
        let project = Project {
            name,
            fps,
            canvas_size: shrimply_project_document::project::CanvasSize { width, height },
            caption_tracks: vec![CaptionTrack::default()],
            video_tracks: vec![VisualTrack::default()],
            audio_tracks: vec![AudioTrack::default()],
            ..Default::default()
        };
        let worker_path = project_path.clone();
        let canceled = Arc::new(AtomicBool::new(false));
        let worker_canceled = canceled.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            if worker_canceled.load(Ordering::Acquire) {
                return Err(BridgeError::Rejected(
                    "MCP request was canceled".to_string(),
                ));
            }
            shrimply_project_document::project::create_new_project_file(&worker_path, &project)
                .map_err(BridgeError::Rejected)?;
            Bridge::launch_and_connect_with_cancel(&worker_path, worker_canceled).map_err(|error| {
                match error {
                    BridgeError::Transport(error) => BridgeError::Transport(format!(
                        "project was created at {}, but {error}",
                        worker_path.display()
                    )),
                    BridgeError::Rejected(error) => BridgeError::Rejected(format!(
                        "project was created at {}, but {error}",
                        worker_path.display()
                    )),
                }
            })
        });
        let bridge = tokio::select! {
            result = &mut worker => result
                .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                .map_err(bridge_error)?,
            () = context.ct.cancelled() => {
                canceled.store(true, Ordering::Release);
                worker.await
                    .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                    .map_err(bridge_error)?
            }
        };
        let project_path = bridge
            .project_path()
            .to_str()
            .expect("project path was validated when the bridge connected")
            .to_string();
        *self
            .bridge
            .write()
            .expect("Shrimply MCP project connection lock was poisoned") = Some(bridge);
        Ok(Json(CreateProjectResponse { project_path }))
    }

    #[tool(
        description = "Connect this MCP session to a native .shrimp video project already open in the Shrimply editor, using its absolute file path. Calling it again switches projects"
    )]
    async fn connect_project(
        &self,
        Parameters(request): Parameters<ConnectProjectRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ConnectProjectResponse>, McpError> {
        let project_path = PathBuf::from(request.project_path);
        if !project_path.is_absolute() {
            return Err(mcp_error("project_path must be an absolute path"));
        }

        let canceled = Arc::new(AtomicBool::new(false));
        let worker_canceled = canceled.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            Bridge::connect_with_cancel(&project_path, worker_canceled)
        });
        let bridge = tokio::select! {
            result = &mut worker => result
                .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                .map_err(bridge_error)?,
            () = context.ct.cancelled() => {
                canceled.store(true, Ordering::Release);
                worker.await
                    .map_err(|error| internal_error(format!("editor bridge task failed: {error}")))?
                    .map_err(bridge_error)?
            }
        };
        let project_path = bridge
            .project_path()
            .to_str()
            .expect("project path was validated when the bridge connected")
            .to_string();
        *self
            .bridge
            .write()
            .expect("Shrimply MCP project connection lock was poisoned") = Some(bridge);
        Ok(Json(ConnectProjectResponse { project_path }))
    }

    #[tool(
        description = "Return live project, playhead, selection, active scope, and track state",
        annotations(read_only_hint = true)
    )]
    async fn get_editor_state(
        &self,
        Parameters(_): Parameters<GetEditorStateRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditorState>, McpError> {
        query::editor_state(&self.snapshot(&context).await?)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "List root and concrete folded-sequence presentation scopes with tracks",
        annotations(read_only_hint = true)
    )]
    async fn list_scopes(
        &self,
        Parameters(_): Parameters<ListScopesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ListScopesResponse>, McpError> {
        query::list_scopes(&self.snapshot(&context).await?)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Query live clip presentations. The optional half-open frame range is stateless and independent from editor selection",
        annotations(read_only_hint = true)
    )]
    async fn query_clips(
        &self,
        Parameters(request): Parameters<QueryClipsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<QueryClipsResponse>, McpError> {
        query::query_clips(&self.snapshot(&context).await?, &request)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Get full live metadata for one concrete clip address or every presentation of an item UUID",
        annotations(read_only_hint = true)
    )]
    async fn get_clip(
        &self,
        Parameters(request): Parameters<GetClipRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ClipMetadata>, McpError> {
        query::get_clip(
            &self.snapshot(&context).await?,
            request.address.as_ref(),
            request.item_id.as_deref(),
        )
        .map(Json)
        .map_err(mcp_error)
    }

    #[tool(
        description = "Return a deep inspector-style dump for a clip, including filesystem, container, stream, tag, artwork, and image EXIF metadata. Embedded artwork bytes are returned as image content only when include_artwork is true",
        annotations(read_only_hint = true)
    )]
    async fn get_clip_info(
        &self,
        Parameters(request): Parameters<GetClipInfoRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let snapshot = self.snapshot(&context).await?;
        let include_artwork = request.include_artwork;
        let address = request.address;
        let item_id = request.item_id;
        let mut worker = tokio::task::spawn_blocking(move || {
            query::get_clip_info(&snapshot, address.as_ref(), item_id.as_deref())
        });
        let info = tokio::select! {
            result = &mut worker => result
                .map_err(|error| internal_error(format!("clip info task failed: {error}")))?
                .map_err(mcp_error)?,
            () = context.ct.cancelled() => {
                return Err(mcp_error("MCP request was canceled"));
            }
        };
        let value = serde_json::to_value(&info)
            .map_err(|error| internal_error(format!("could not encode clip info: {error}")))?;
        let mut result = CallToolResult::structured(value);
        if include_artwork && let Some(source) = &info.source {
            result
                .content
                .extend(source.artwork.iter().filter_map(|artwork| {
                    let mime_type = artwork.mime_type.as_deref()?;
                    mime_type.starts_with("image/").then(|| {
                        ContentBlock::image(BASE64.encode(&artwork.data), mime_type.to_string())
                    })
                }));
        }
        Ok(result)
    }

    #[tool(
        description = "Inspect a Manim clip's discovered scenes, reflected parameter controls and values, and current render error",
        annotations(read_only_hint = true)
    )]
    async fn get_manim_clip(
        &self,
        Parameters(request): Parameters<GetManimClipRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ManimClipResponse>, McpError> {
        let value = self
            .request(BridgeCommand::GetManimClip(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned invalid Manim clip metadata: {error}"
            ))
        })
    }

    #[tool(
        description = "Return one fully addressed track and up to 500 of its clips in projected timeline order",
        annotations(read_only_hint = true)
    )]
    async fn get_track(
        &self,
        Parameters(request): Parameters<GetTrackRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<TrackMetadata>, McpError> {
        query::get_track(&self.snapshot(&context).await?, &request)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(
        description = "Query expression IDs, property paths, enabled state, and source across live clip metadata",
        annotations(read_only_hint = true)
    )]
    async fn query_expressions(
        &self,
        Parameters(request): Parameters<QueryExpressionsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<QueryExpressionsResponse>, McpError> {
        query::query_expressions(&self.snapshot(&context).await?, &request)
            .map(Json)
            .map_err(mcp_error)
    }

    #[tool(description = "Seek the visible editor playhead to a project frame")]
    async fn seek_playhead(
        &self,
        Parameters(request): Parameters<SeekPlayheadRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<FrameTime>, McpError> {
        let value = self
            .request(
                BridgeCommand::Seek {
                    frame: request.frame,
                },
                &context,
            )
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!("editor returned an invalid seek result: {error}"))
        })
    }

    #[tool(
        description = "Render a project frame with Shrimply's native compositor and return it as a PNG without changing the playhead",
        annotations(read_only_hint = true)
    )]
    async fn view_frame(
        &self,
        Parameters(request): Parameters<ViewFrameRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let value = self
            .request(
                BridgeCommand::ViewFrame {
                    frame: request.frame,
                },
                &context,
            )
            .await?;
        let response: ViewFrameResponse = serde_json::from_value(value).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid rendered frame: {error}"
            ))
        })?;
        let metadata = serde_json::to_string(&response.frame)
            .map_err(|error| internal_error(format!("could not encode frame metadata: {error}")))?;
        Ok(CallToolResult::success(vec![
            ContentBlock::text(metadata),
            ContentBlock::image(response.png, "image/png"),
        ]))
    }

    #[tool(
        description = "Analyze a Transparent Fill modifier and wait until all frame masks are cached"
    )]
    async fn analyze_transparent_fill(
        &self,
        Parameters(request): Parameters<AnalyzeTransparentFillRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<AnalyzeTransparentFillResponse>, McpError> {
        let value = self
            .request(BridgeCommand::AnalyzeTransparentFill(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid Transparent Fill analysis result: {error}"
            ))
        })
    }

    #[tool(description = "Create one caption, video, or audio track as an explicit undoable edit")]
    async fn create_track(
        &self,
        Parameters(request): Parameters<CreateTrackRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            EditRequest {
                history_label: "MCP create track".to_string(),
                frame: None,
                scope: request.scope,
                operations: vec![EditOperation::CreateTrack(CreateTrackOperation {
                    kind: request.kind,
                    enabled: request.enabled,
                })],
            },
            &context,
        )
        .await
    }

    #[tool(
        description = "Import one or more native files atomically. Omitted targets choose an existing compatible track with room and never create one unless collision=new_track is explicit. Copying into project media is the preferred default; link=true retains external paths"
    )]
    async fn insert_files(
        &self,
        Parameters(request): Parameters<InsertFilesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            EditRequest {
                history_label: "MCP insert files".to_string(),
                frame: request.frame,
                scope: request.scope.clone(),
                operations: vec![EditOperation::InsertFiles(request)],
            },
            &context,
        )
        .await
    }

    #[tool(
        description = "Bulk-insert captions into an existing root caption track or create a new one, with exact frame ranges, optional language, collision handling, and source style copying"
    )]
    async fn insert_captions(
        &self,
        Parameters(request): Parameters<InsertCaptionsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP insert captions",
                EditOperation::InsertCaptions(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Insert an empty text-to-speech item on an explicit or automatically selected audio track for later inspector configuration"
    )]
    async fn insert_tts(
        &self,
        Parameters(request): Parameters<InsertTtsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            EditRequest {
                history_label: "MCP insert TTS".to_string(),
                frame: request.frame,
                scope: request.scope.clone(),
                operations: vec![EditOperation::InsertTts(request)],
            },
            &context,
        )
        .await
    }

    #[tool(
        description = "Insert a native editable text video clip at an exact projected frame and duration, choosing a video track with room or creating one when omitted"
    )]
    async fn insert_text(
        &self,
        Parameters(request): Parameters<InsertTextRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            EditRequest {
                history_label: "MCP insert text".to_string(),
                frame: request.frame,
                scope: request.scope,
                operations: vec![EditOperation::InsertText(InsertTextOperation {
                    text: request.text,
                    track: request.track,
                    duration_frames: request.duration_frames,
                    collision: request.collision,
                })],
            },
            &context,
        )
        .await
    }

    #[tool(
        description = "List text-to-speech models and their dynamic input definitions from the editor's current compute server",
        annotations(read_only_hint = true)
    )]
    async fn list_tts_models(
        &self,
        Parameters(_): Parameters<ListTtsModelsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ListTtsModelsResponse>, McpError> {
        let value = self.request(BridgeCommand::ListTtsModels, &context).await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid text-to-speech model list: {error}"
            ))
        })
    }

    #[tool(
        description = "List speech-to-text models advertised by the editor's current compute server",
        annotations(read_only_hint = true)
    )]
    async fn list_stt_models(
        &self,
        Parameters(_): Parameters<ListSttModelsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ListSttModelsResponse>, McpError> {
        let value = self.request(BridgeCommand::ListSttModels, &context).await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid speech-to-text model list: {error}"
            ))
        })
    }

    #[tool(
        description = "Transcribe one audio track or one or more fully addressed audio clips with the current compute server and insert timed captions on a new track"
    )]
    async fn transcribe_audio(
        &self,
        Parameters(request): Parameters<TranscribeAudioRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        let value = self
            .request(BridgeCommand::TranscribeAudio(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid transcription edit result: {error}"
            ))
        })
    }

    #[tool(
        description = "Generate speech with the editor's current compute server and insert it on an explicit or automatically selected audio track"
    )]
    async fn generate_tts(
        &self,
        Parameters(request): Parameters<GenerateTtsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        let value = self
            .request(BridgeCommand::GenerateTts(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid text-to-speech edit result: {error}"
            ))
        })
    }

    #[tool(
        description = "Move a fully addressed clip to a projected frame and optional compatible track/scope"
    )]
    async fn move_clip(
        &self,
        Parameters(request): Parameters<MoveClipRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single("MCP move clip", EditOperation::MoveClip(request)),
            &context,
        )
        .await
    }

    #[tool(
        description = "Trim a fully addressed clip using projected frame bounds while preserving source offset"
    )]
    async fn trim_clip(
        &self,
        Parameters(request): Parameters<TrimClipRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single("MCP trim clip", EditOperation::TrimClip(request)),
            &context,
        )
        .await
    }

    #[tool(
        description = "Delete fully addressed clips as one undoable history action",
        annotations(destructive_hint = true)
    )]
    async fn delete_clips(
        &self,
        Parameters(request): Parameters<DeleteClipsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single("MCP delete clips", EditOperation::DeleteClips(request)),
            &context,
        )
        .await
    }

    #[tool(
        description = "Set typed clip properties: caption text, audio enabled/gain, or video/audio playback speed/repeat strategy"
    )]
    async fn set_clip_properties(
        &self,
        Parameters(request): Parameters<SetClipPropertiesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP set clip properties",
                EditOperation::SetClipProperties(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Set constant position, scale, rotation, anchor, shear, fit mode, and/or raster crop on a video clip. Supplied transform fields replace any keyframes on those fields"
    )]
    async fn set_video_transform(
        &self,
        Parameters(request): Parameters<SetVideoTransformRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP set video transform",
                EditOperation::SetVideoTransform(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Bulk-create or replace scalar or vec2 keyframes by JSON property path and projected timeline frame"
    )]
    async fn upsert_keyframes(
        &self,
        Parameters(request): Parameters<UpsertKeyframesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP upsert keyframes",
                EditOperation::UpsertKeyframes(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Delete scalar or vec2 keyframes by JSON property path and projected timeline frame; deleting the last keyframe preserves the property's first keyed value as a constant"
    )]
    async fn delete_keyframes(
        &self,
        Parameters(request): Parameters<DeleteKeyframesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP delete keyframes",
                EditOperation::DeleteKeyframes(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Create or update an expression directly on a scalar or vec2 TimelineValue addressed by its get_clip JSON property path"
    )]
    async fn upsert_property_expression(
        &self,
        Parameters(request): Parameters<UpsertPropertyExpressionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP upsert property expression",
                EditOperation::UpsertPropertyExpression(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Remove the expression from a scalar or vec2 TimelineValue addressed by its get_clip JSON property path"
    )]
    async fn delete_property_expression(
        &self,
        Parameters(request): Parameters<DeletePropertyExpressionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP delete property expression",
                EditOperation::DeletePropertyExpression(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Add, edit, or remove per-clip intro/outro transitions. Each update names one side; a null transition removes it"
    )]
    async fn set_clip_transitions(
        &self,
        Parameters(request): Parameters<SetClipTransitionsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP set clip transitions",
                EditOperation::SetClipTransitions(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Set a Manim clip's discovered scene and/or typed reflected parameter overrides as one validated undoable edit; null resets a parameter to its scene default"
    )]
    async fn set_manim_clip(
        &self,
        Parameters(request): Parameters<SetManimClipRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        let value = self
            .request(BridgeCommand::SetManimClip(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid Manim edit result: {error}"
            ))
        })
    }

    #[tool(
        description = "Invalidate a Manim clip's cached Python scene state and rebuild it from the current source file"
    )]
    async fn reload_manim_source(
        &self,
        Parameters(request): Parameters<ReloadManimSourceRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ReloadManimSourceResponse>, McpError> {
        let value = self
            .request(BridgeCommand::ReloadManimSource(request), &context)
            .await?;
        serde_json::from_value(value).map(Json).map_err(|error| {
            internal_error(format!(
                "editor returned an invalid Manim reload result: {error}"
            ))
        })
    }

    #[tool(
        description = "Set an expression's source and/or enabled state by stable ID on its owning video or audio clip"
    )]
    async fn set_expression(
        &self,
        Parameters(request): Parameters<SetExpressionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single("MCP set expression", EditOperation::SetExpression(request)),
            &context,
        )
        .await
    }

    #[tool(description = "Enable or disable a fully addressed caption, visual, or audio track")]
    async fn set_track_enabled(
        &self,
        Parameters(request): Parameters<SetTrackEnabledRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP set track enabled",
                EditOperation::SetTrackEnabled(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Set or clear a caption track's CLDR locale identifier, such as en_US, en_GB, or ja_JP"
    )]
    async fn set_caption_track_language(
        &self,
        Parameters(request): Parameters<SetCaptionTrackLanguageRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single(
                "MCP set caption track language",
                EditOperation::SetCaptionTrackLanguage(request),
            ),
            &context,
        )
        .await
    }

    #[tool(
        description = "Delete a fully addressed caption, visual, or audio track and all of its clips",
        annotations(destructive_hint = true)
    )]
    async fn delete_track(
        &self,
        Parameters(request): Parameters<DeleteTrackRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            single("MCP delete track", EditOperation::DeleteTrack(request)),
            &context,
        )
        .await
    }

    #[tool(
        description = "Run an ordered typed edit program atomically as one MCP edit script history action"
    )]
    async fn run_edit_script(
        &self,
        Parameters(request): Parameters<RunEditScriptRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<EditResponse>, McpError> {
        self.edit(
            EditRequest {
                history_label: "MCP edit script".to_string(),
                frame: request.frame,
                scope: request.scope,
                operations: request.operations,
            },
            &context,
        )
        .await
    }
}

#[tool_handler]
impl ServerHandler for ShrimplyServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_instructions(MCP_INSTRUCTIONS.to_string())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("shrimply://editor/state", "editor-state")
                .with_description("Live Shrimply .shrimp project/editor/player/selection state")
                .with_mime_type("application/json"),
            Resource::new("shrimply://project/clips", "project-clips")
                .with_description("All current root and nested clip presentations")
                .with_mime_type("application/json"),
            Resource::new("shrimply://edit-api", "edit-api")
                .with_description("Typed editing API for the connected .shrimp video project")
                .with_mime_type("text/plain"),
        ]))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("shrimply://project/clips/{item_id}", "project-clip")
                .with_description("Full metadata plus every concrete presentation of an item UUID")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let value = match request.uri.as_str() {
            "shrimply://editor/state" => serde_json::to_value(
                query::editor_state(&self.snapshot(&context).await?).map_err(mcp_error)?,
            ),
            "shrimply://project/clips" => serde_json::to_value(
                query::all_clips(&self.snapshot(&context).await?).map_err(mcp_error)?,
            ),
            "shrimply://edit-api" => {
                return Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    EDIT_API,
                    request.uri,
                )])
                .into());
            }
            uri if uri.starts_with("shrimply://project/clips/") => {
                let item_id = uri.trim_start_matches("shrimply://project/clips/");
                serde_json::to_value(
                    query::get_clip(&self.snapshot(&context).await?, None, Some(item_id))
                        .map_err(mcp_error)?,
                )
            }
            _ => {
                return Err(McpError::resource_not_found(
                    "Shrimply resource was not found",
                    Some(json!({ "uri": request.uri })),
                ));
            }
        }
        .map_err(|error| internal_error(format!("could not encode resource: {error}")))?;
        let text = serde_json::to_string_pretty(&value)
            .map_err(|error| internal_error(format!("could not encode resource: {error}")))?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri).with_mime_type("application/json"),
        ])
        .into())
    }
}

fn single(label: &str, operation: EditOperation) -> EditRequest {
    EditRequest {
        history_label: label.to_string(),
        frame: None,
        scope: None,
        operations: vec![operation],
    }
}

fn mcp_error(error: impl ToString) -> McpError {
    McpError::invalid_params(error.to_string(), None)
}

fn internal_error(error: impl ToString) -> McpError {
    McpError::internal_error(error.to_string(), None)
}

fn bridge_error(error: BridgeError) -> McpError {
    match error {
        BridgeError::Rejected(error) => mcp_error(error),
        BridgeError::Transport(error) => internal_error(error),
    }
}
