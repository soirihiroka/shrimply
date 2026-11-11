use crate::{catalog, search};
use clap::ArgMatches;
use serde_json::{Value, json};
use shrimply_editing_api::{bridge::Bridge, commands, preview, protocol::*};
use shrimply_project_document::project;
use std::{
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SESSION_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn run(arguments: Vec<std::ffi::OsString>) -> u8 {
    let matches = match catalog::cli().try_get_matches_from(arguments) {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return 0;
        }
        Err(error) => {
            output(&failure(
                "invalid_arguments",
                &error.to_string(),
                "Use shrimply cli search to discover commands, then inspect --help or schema.",
            ));
            return 2;
        }
    };
    let (path, leaf) = leaf(&matches);
    match execute(&path, leaf, &matches) {
        Ok(value) => {
            output(&value);
            0
        }
        Err(error) => {
            let invalid = error.starts_with("invalid arguments:");
            output(&failure(
                if invalid {
                    "invalid_arguments"
                } else if error.contains("revision conflict") {
                    "revision_conflict"
                } else {
                    "execution_failed"
                },
                &error,
                "Inspect the command schema and current project state before retrying.",
            ));
            if invalid { 2 } else { 1 }
        }
    }
}

fn leaf(matches: &ArgMatches) -> (String, &ArgMatches) {
    let mut current = matches;
    let mut parts = Vec::new();
    while let Some((name, child)) = current.subcommand() {
        parts.push(name);
        current = child;
    }
    (parts.join(" "), current)
}

fn execute(path: &str, leaf: &ArgMatches, root: &ArgMatches) -> Result<Value, String> {
    if path == "cli search" {
        return Ok(search::search(
            leaf.get_one::<String>("query").expect("query is required"),
        ));
    }
    if path == "schema" {
        let path = leaf
            .get_many::<String>("command")
            .expect("schema command is required")
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let definition = commands::get(&path)?;
        return Ok(
            json!({"command":format!("shrimply {}",definition.path),"summary":definition.summary,"read_only":definition.read_only,"request":(definition.schema)(),"inputs":"Named flags or --body JSON, @file, @-; complex fields accept JSON or @file","times":"Zero-based integer frames; exact numerator/denominator for frame rates"}),
        );
    }
    let positional = leaf
        .try_get_one::<String>("render_project_path")
        .ok()
        .flatten();
    if positional.is_some() && root.get_one::<String>("project").is_some() {
        return Err(
            "invalid arguments: specify the project once, positionally or with --project".into(),
        );
    }
    let project_path = root
        .get_one::<String>("project")
        .or(positional)
        .map(|path| std::path::absolute(path).map_err(|error| error.to_string()))
        .transpose()?;
    if path == "project serve" {
        let definition = commands::get(path)?;
        (definition.validate)(arguments(leaf, definition)?)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let path = project_path.ok_or("invalid arguments: --project is required")?;
        if root.get_flag("dry-run") {
            project::load_project_snapshot(&path)?;
            return Ok(
                json!({"success":true,"project":path,"dry_run":true,"planned":{"serve":"headless"}}),
            );
        }
        shrimply_process_reporting::diagnostics::init();
        shrimply_editing_runtime::serve(&path)?;
        return Ok(json!({"success":true,"project":path,"closed":true}));
    }
    let definition = commands::get(path)?;
    let dry_run = root.get_flag("dry-run");
    let if_revision = root.get_one::<String>("if-revision").cloned();
    let mut arguments = arguments(leaf, definition)?;
    if definition.name == "get_version" {
        (definition.validate)(arguments).map_err(|error| format!("invalid arguments: {error}"))?;
        let mut editor = None;
        let mut editor_error = None;
        if let Some(path) = &project_path {
            match Bridge::connect(path).and_then(|bridge| bridge.request(BridgeCommand::Version)) {
                Ok(value) => editor = Some(value),
                Err(error) => editor_error = Some(error.to_string()),
            }
        }
        return Ok(
            json!({"cli":BuildVersion::current(env!("CARGO_PKG_VERSION")),"editor":editor,"editor_error":editor_error}),
        );
    }
    let path = project_path
        .or_else(|| {
            arguments
                .get("project_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .ok_or("invalid arguments: --project is required")?;
    let path = project::normalized_project_path(
        &std::path::absolute(path).map_err(|error| error.to_string())?,
    );
    if matches!(definition.name, "create_project" | "connect_project") {
        arguments["project_path"] = Value::String(path.to_string_lossy().into_owned());
    }
    if definition.name == "render_project" {
        let request = (definition.validate)(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let request = serde_json::from_value::<RenderProjectRequest>(request)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let options = crate::render_options::from_request(path.clone(), request, if_revision)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        let result = if dry_run {
            crate::render::dry_run(&options)?
        } else {
            shrimply_process_reporting::diagnostics::init();
            crate::render::run(options)?;
            json!({"rendered":true})
        };
        return Ok(
            json!({"success":true,"command":"shrimply render","project":path,"result":result}),
        );
    }
    normalize_media_paths(definition.name, &mut arguments)?;
    resolve_addresses(leaf, definition, &path, &mut arguments)?;
    arguments =
        (definition.validate)(arguments).map_err(|error| format!("invalid arguments: {error}"))?;
    if definition.name == "create_project" {
        let request: CreateProjectRequest =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        let mut project = project::Project {
            caption_tracks: vec![project::CaptionTrack::default()],
            video_tracks: vec![project::VisualTrack::default()],
            audio_tracks: vec![project::AudioTrack::default()],
            ..Default::default()
        };
        shrimply_editing_api::extra::settings(
            &mut project,
            &ProjectSettingsRequest {
                name: request.name,
                width: request.width,
                height: request.height,
                fps: request.fps,
            },
        )?;
        if path.extension().and_then(|extension| extension.to_str()) != Some("shrimp") {
            return Err("new projects require a .shrimp extension".into());
        }
        if path.try_exists().map_err(|error| error.to_string())? {
            return Err("project already exists; creation never overwrites a file".into());
        }
        if dry_run {
            return Ok(
                json!({"success":true,"command":path_string(definition),"project":path,"dry_run":true,"planned":{"create":project}}),
            );
        }
        project::create_new_project_file(&path, &project)?;
        return Ok(
            json!({"success":true,"command":path_string(definition),"project":path,"result":{"created":true,"content_revision":preview::revision(&project)?}}),
        );
    }
    if definition.name == "connect_project" {
        if !path.is_file() {
            return Err("project does not exist".into());
        }
        project::load_project_snapshot(&path)?;
        let headless = leaf.get_flag("headless");
        if dry_run {
            return Ok(
                json!({"success":true,"project":path,"dry_run":true,"planned":{"open":if headless {"headless"}else{"native_editor"}}}),
            );
        }
        let reused = project::project_lock_owner(&path)?.is_some();
        let bridge = if reused {
            Bridge::connect(&path).map_err(|error| error.to_string())?
        } else if headless {
            Bridge::launch(&path).map_err(|error| error.to_string())?
        } else {
            launch_editor(&path)?
        };
        let mut result = bridge
            .request(BridgeCommand::Handshake)
            .map_err(|error| error.to_string())?;
        result.as_object_mut().unwrap().remove("connected");
        result["ready"] = json!(true);
        result["opened"] = json!(true);
        return Ok(json!({"success":true,"project":path,"result":result}));
    }
    let mut owner = project::project_lock_owner(&path)?;
    if definition.name == "project_close" && !dry_run && owner.is_none() {
        return Ok(
            json!({"success":true,"project":path,"result":{"closed":true,"already_closed":true}}),
        );
    }
    let call = ApiCall {
        name: definition.name.into(),
        arguments,
        dry_run,
        if_revision,
    };
    let mut result = if owner.is_some() {
        let bridge = Bridge::connect(&path).map_err(|error| error.to_string())?;
        owner = Some(bridge.owner.clone());
        bridge
            .request(BridgeCommand::Execute(call))
            .map_err(|error| error.to_string())?
    } else if dry_run || preview::is_snapshot_query(definition.name) {
        let snapshot = preview::saved_snapshot(
            project::load_project_snapshot(&path)?,
            path.to_string_lossy().into_owned(),
        );
        let result = if dry_run {
            preview::dry_run(&call, &snapshot)?
        } else {
            preview::query_call(definition.name, call.arguments, &snapshot)?
        };
        preview::with_state(result, &snapshot.project, "saved")?
    } else if definition.read_only {
        shrimply_editing_runtime::query_saved(&path, call)?
    } else {
        let bridge = Bridge::launch(&path).map_err(|error| error.to_string())?;
        bridge
            .request(BridgeCommand::Execute(call))
            .map_err(|error| error.to_string())?
    };
    if definition.name == "project_close" && !dry_run {
        let deadline = Instant::now() + SESSION_WAIT_TIMEOUT;
        while project::project_lock_owner(&path)? == owner {
            if Instant::now() >= deadline {
                return Err("headless session accepted close but has not released the project within 30 seconds; inspect the owner before retrying".into());
            }
            std::thread::sleep(SESSION_POLL_INTERVAL);
        }
        result.as_object_mut().unwrap().remove("closing");
        result["closed"] = json!(true);
    }
    if definition.name == "view_frame"
        && !dry_run
        && let Some(output) = leaf.get_one::<String>("output")
    {
        use base64::Engine as _;
        let png = result
            .get("png")
            .and_then(Value::as_str)
            .ok_or("frame renderer did not return PNG data")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .map_err(|error| error.to_string())?;
        write_image(&path, Path::new(output), &bytes)?;
        result
            .as_object_mut()
            .expect("frame response is an object")
            .remove("png");
        result["output"] = json!(output);
    }

    Ok(json!({"success":true,"command":path_string(definition),"project":path,"result":result}))
}

fn path_string(definition: &commands::Command) -> String {
    format!("shrimply {}", definition.path)
}

fn arguments(matches: &ArgMatches, definition: &commands::Command) -> Result<Value, String> {
    let schema = (definition.schema)();
    let empty = serde_json::Map::new();
    let properties = schema
        .as_value()
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let body = matches.get_one::<String>("body");
    let mut result = serde_json::Map::new();
    for (name, property) in properties {
        if name == "project_path" {
            continue;
        }
        if let Some(value) = matches.get_one::<String>(name) {
            if body.is_some() {
                return Err(
                    "invalid arguments: --body conflicts with individual operation flags".into(),
                );
            }
            result.insert(
                name.clone(),
                parse_argument(
                    name,
                    value,
                    catalog::property_type(property, schema.as_value()),
                )?,
            );
        }
    }
    if let Some(body) = body {
        let value = read_json(body)?;
        if !value.is_object() {
            return Err("invalid arguments: --body must be a JSON object".into());
        }
        Ok(value)
    } else {
        Ok(Value::Object(result))
    }
}

fn parse_argument(name: &str, value: &str, property: &Value) -> Result<Value, String> {
    if name == "fps" && !value.starts_with(['@', '{']) {
        let rate: shrimply_math_core::Fraction = value
            .parse()
            .map_err(|_| "invalid arguments: fps must be an exact ratio, integer, or decimal")?;
        if !rate.is_finite()
            || rate <= shrimply_math_core::Fraction::from(0_u8)
            || rate
                .numer()
                .is_none_or(|value| i64::try_from(*value).is_err())
            || rate
                .denom()
                .is_none_or(|value| i64::try_from(*value).is_err())
        {
            return Err("invalid arguments: fps must be a positive finite ratio with signed 64-bit numerator and denominator".into());
        }
        return Ok(
            json!({"numerator":project::fraction_numerator(rate),"denominator":project::fraction_denominator(rate)}),
        );
    }
    if catalog::is_string(property) {
        return Ok(Value::String(value.into()));
    }
    read_json(value).map_err(|error| {
        format!(
            "invalid arguments for --{}: {error}",
            name.replace('_', "-")
        )
    })
}

fn read_json(value: &str) -> Result<Value, String> {
    let source = if value == "@-" {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| error.to_string())?;
        source
    } else if let Some(path) = value.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|error| error.to_string())?
    } else {
        value.into()
    };
    serde_json::from_str(&source).map_err(|error| error.to_string())
}

fn normalize_media_paths(name: &str, arguments: &mut Value) -> Result<(), String> {
    if name == "insert_files" {
        if let Some(files) = arguments.get_mut("files").and_then(Value::as_array_mut) {
            for file in files {
                if let Some(source) = file.get("source").and_then(Value::as_str) {
                    file["source"] =
                        json!(std::path::absolute(source).map_err(|error| error.to_string())?);
                }
            }
        }
        if let Some(root) = arguments.get("copy_root").and_then(Value::as_str) {
            arguments["copy_root"] =
                json!(std::path::absolute(root).map_err(|error| error.to_string())?);
        }
    } else if name == "run_edit_script" {
        if let Some(operations) = arguments
            .get_mut("operations")
            .and_then(Value::as_array_mut)
        {
            for operation in operations {
                if operation.get("type").and_then(Value::as_str) == Some("insert_files") {
                    normalize_media_paths("insert_files", &mut operation["args"])?;
                }
            }
        }
    } else if name == "generate_tts"
        && let Some(inputs) = arguments.get_mut("inputs").and_then(Value::as_object_mut)
    {
        for value in inputs.values_mut() {
            if value.get("kind").and_then(Value::as_str) == Some("audio")
                && let Some(path) = value.get("path").and_then(Value::as_str)
            {
                value["path"] =
                    json!(std::path::absolute(path).map_err(|error| error.to_string())?);
            }
        }
    }
    Ok(())
}

fn resolve_addresses(
    matches: &ArgMatches,
    definition: &commands::Command,
    path: &Path,
    arguments: &mut Value,
) -> Result<(), String> {
    let schema = (definition.schema)();
    let empty = serde_json::Map::new();
    let properties = schema
        .as_value()
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if !properties.contains_key("address") && !properties.contains_key("addresses") {
        return Ok(());
    }
    let item_id = matches.try_get_one::<String>("item_id").ok().flatten();
    let track_id = matches.try_get_one::<String>("track_id").ok().flatten();
    if item_id.is_none() && properties.contains_key("track_id") {
        return Ok(());
    }
    if matches.get_one::<String>("body").is_some() && (item_id.is_some() || track_id.is_some()) {
        return Err("invalid arguments: --body conflicts with ID selector flags".into());
    }
    if item_id.is_none() && track_id.is_none() {
        return Ok(());
    }
    if arguments.get("address").is_some() || arguments.get("addresses").is_some() {
        return Err("invalid arguments: ID selectors conflict with a complete address".into());
    }
    if properties.contains_key("item_id") && !properties.contains_key("addresses") {
        return Ok(());
    }
    let snapshot = load_snapshot(path)?;
    let address = if let Some(item_id) = item_id {
        let candidates: Vec<_> = shrimply_editing_api::query::all_clips(&snapshot)?
            .clips
            .into_iter()
            .filter(|clip| {
                &clip.address.item_id == item_id
                    && track_id.is_none_or(|id| &clip.address.track_id == id)
            })
            .map(|clip| clip.address)
            .collect();
        if candidates.len() != 1 {
            return Err(format!(
                "item ID resolved to {} presentations; provide a concrete address: {}",
                candidates.len(),
                serde_json::to_string(&candidates).map_err(|error| error.to_string())?
            ));
        }
        serde_json::to_value(&candidates[0]).map_err(|error| error.to_string())?
    } else {
        let id = track_id.expect("one ID selector exists");
        let candidates: Vec<_> = shrimply_editing_api::query::list_scopes(&snapshot)?
            .scopes
            .into_iter()
            .flat_map(|scope| scope.tracks)
            .filter(|track| &track.address.track_id == id)
            .map(|track| track.address)
            .collect();
        if candidates.len() != 1 {
            return Err(format!(
                "track ID resolved to {} presentations; provide a complete address",
                candidates.len()
            ));
        }
        serde_json::to_value(&candidates[0]).map_err(|error| error.to_string())?
    };
    let object = arguments
        .as_object_mut()
        .ok_or("operation arguments must be an object")?;
    if !properties.contains_key("item_id") {
        object.remove("item_id");
    }
    if !properties.contains_key("track_id") {
        object.remove("track_id");
    }
    if properties.contains_key("addresses") {
        object.insert("addresses".into(), json!([address]));
    } else {
        object.insert("address".into(), address);
    }
    Ok(())
}

fn load_snapshot(path: &Path) -> Result<LiveSnapshot, String> {
    if project::project_lock_owner(path)?.is_some() {
        let value = Bridge::connect(path)
            .and_then(|bridge| bridge.request(BridgeCommand::Snapshot))
            .map_err(|error| error.to_string())?;
        serde_json::from_value(value).map_err(|error| error.to_string())
    } else {
        Ok(preview::saved_snapshot(
            project::load_project_snapshot(path)?,
            path.to_string_lossy().into_owned(),
        ))
    }
}

fn launch_editor(path: &Path) -> Result<Bridge, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let editor = current.with_file_name("shrimply-editor");
    #[cfg(target_os = "macos")]
    let editor = current;
    let mut child = std::process::Command::new(editor)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + SESSION_WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("editor exited before becoming ready: {status}"));
        }
        if let Ok(bridge) = Bridge::connect(path) {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(bridge);
        }
        std::thread::sleep(SESSION_POLL_INTERVAL);
    }
    Err("editor has not exposed its editing API within 30 seconds; check the editor window".into())
}

fn write_image(project: &Path, output: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let output = std::path::absolute(output).map_err(|error| error.to_string())?;
    if output == project {
        return Err("image output cannot replace the project".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output)
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(bytes) {
        let _ = std::fs::remove_file(&output);
        return Err(error.to_string());
    }
    Ok(())
}

fn failure(code: &str, message: &str, hint: &str) -> Value {
    json!({"success":false,"error":{"code":code,"message":message,"hint":hint}})
}

fn output(value: &Value) {
    if std::io::stdout().is_terminal() {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("JSON value serializes")
        );
    } else {
        println!("{value}");
    }
}
