//! The installed application's LSP uses the same read-only project bridge as the CLI.
use serde_json::{Value, json};
use shrimply_editing_api::{
    bridge::Bridge,
    protocol::{ApiCall, BridgeCommand, QueryClipsResponse, SanimClipResponse},
};
use shrimply_project_document::project;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SUMMARY: &str = "Serve Sanim language features over stdio, with live project reflection values; --project selects an active project explicitly";
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

pub fn schema() -> Value {
    json!({"command":"shrimply sanim lsp","summary":SUMMARY,"read_only":true,
        "transport":"LSP Content-Length framed JSON-RPC on stdin/stdout; diagnostics on stderr",
        "project":"Optional --project PATH; otherwise discover active projects in source-file ancestor directories",
        "runtime":"Read-only active-owner queries; never opens an editor or starts a headless owner"})
}

pub fn run(project: Option<PathBuf>) -> u8 {
    let mut query = |source: &Path| runtime(project.as_deref(), source);
    match sanim_lsp::run_with_runtime(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        Some(&mut query),
    ) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("shrimply sanim lsp: {error}");
            1
        }
    }
}

fn runtime(explicit_project: Option<&Path>, source: &Path) -> Result<Value, String> {
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let remaining = || {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            Err("Runtime reflection query timed out".to_owned())
        } else {
            Ok(timeout)
        }
    };
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let mut projects = std::collections::BTreeSet::new();
    if let Some(path) = explicit_project {
        projects.insert(path.canonicalize().map_err(|error| error.to_string())?);
    } else {
        for directory in source.ancestors().skip(1) {
            for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "shrimp" | "sjson" | "json"
                        )
                    })
                    && project::project_lock_owner(&path).ok().flatten().is_some()
                {
                    projects.insert(project::normalized_project_path(&path));
                }
            }
        }
    }
    let mut contexts = Vec::new();
    for project in projects {
        let clips: QueryClipsResponse = serde_json::from_value(
            Bridge::connect(&project)
                .map_err(|error| error.to_string())?
                .request_with_timeout(
                    BridgeCommand::Execute(ApiCall {
                        name: "query_clips".into(),
                        arguments: json!({"all":true,"recursive":true,"source_kind":"sanim"}),
                        dry_run: false,
                        if_revision: None,
                    }),
                    remaining()?,
                )
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        for clip in clips.clips {
            if clip
                .asset_path
                .as_ref()
                .is_none_or(|path| Path::new(path).canonicalize().ok().as_ref() != Some(&source))
            {
                continue;
            }
            let state: SanimClipResponse = serde_json::from_value(
                Bridge::connect(&project)
                    .map_err(|error| error.to_string())?
                    .request_with_timeout(
                        BridgeCommand::Execute(ApiCall {
                            name: "get_sanim_clip".into(),
                            arguments: json!({"address":clip.address}),
                            dry_run: false,
                            if_revision: None,
                        }),
                        remaining()?,
                    )
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if Path::new(&state.source).canonicalize().ok().as_ref() != Some(&source) {
                return Err("Sanim clip source changed during runtime inspection; retry".into());
            }
            contexts.push(json!({"project":project,"label":clip.label,"state":state}));
        }
    }
    Ok(json!({"contexts":contexts}))
}
