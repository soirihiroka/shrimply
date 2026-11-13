use sanim_core::{Fraction, Number};
use sanim_lang::{Engine, FileSources, Prepared};
use sanim_render::{Options, Renderer, Scene};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod video;

mod catalog;
mod profile;
mod source_format;
pub use catalog::{COMMANDS, OPTIONS, schema};

#[derive(Default)]
struct Args {
    command: String,
    path: PathBuf,
    json: bool,
    progress: bool,
    sets: BTreeMap<String, serde_json::Value>,
    output: Option<PathBuf>,
    frame: u64,
    ffmpeg: Option<PathBuf>,
    time: Option<Fraction>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<Fraction>,
    dpi: Option<u32>,
    cycles: Option<u64>,
    deny_warnings: bool,
    samples: Option<u32>,
    warn_operations: Option<u64>,
    warn_prepare_ms: Option<u64>,
    format_check: bool,
    write: bool,
    check_format: bool,
    indent_width: Option<usize>,
    line_width: Option<usize>,
    tabs: bool,
}

/// Run the same headless tooling from `sanim` or `shrimply sanim`.
/// Arguments exclude the executable name and the optional `sanim` prefix.
pub fn run(arguments: Vec<std::ffi::OsString>) -> u8 {
    let argv = match arguments
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "Sanim arguments must be UTF-8")
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(argv) => argv,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    if argv.is_empty()
        || argv
            .iter()
            .any(|value| matches!(value.as_str(), "--help" | "-h"))
    {
        print!("{}", catalog::help());
        return 0;
    }
    let json = argv.iter().any(|value| value == "--json");
    let args = match parse(argv) {
        Ok(args) => args,
        Err(error) => {
            emit_error(&error, json);
            return 2;
        }
    };
    match execute(args) {
        Ok(status) => status,
        Err(error) => {
            emit_error(&error, json);
            1
        }
    }
}

fn emit_error(error: &str, json: bool) {
    if json {
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(error) {
            if let Some(object) = value.as_object_mut() {
                object.insert("severity".into(), "error".into());
            }
            eprintln!("{}", serde_json::json!({"ok":false,"diagnostic":value}));
        } else {
            eprintln!(
                "{}",
                serde_json::json!({"ok":false,"diagnostic":{"code":"CLI001","severity":"error","message":error}})
            );
        }
    } else {
        eprintln!("{error}");
    }
}

fn parse(argv: Vec<String>) -> Result<Args, String> {
    if argv.len() < 2 {
        return Err(catalog::help());
    }
    let mut args = Args {
        command: argv[0].clone(),
        path: PathBuf::from(&argv[1]),
        ..Args::default()
    };
    if !COMMANDS.iter().any(|command| command.name == args.command) {
        return Err(format!(
            "unknown command '{}'\n{}",
            args.command,
            catalog::help()
        ));
    }
    let mut i = 2;
    let mut seen = std::collections::BTreeSet::new();
    while i < argv.len() {
        let name = &argv[i];
        let option = catalog::options_for(&args.command)
            .find(|option| name.strip_prefix("--") == Some(option.name))
            .ok_or_else(|| format!("unknown option '{name}'"))?;
        if option.name != "set" && !seen.insert(option.name) {
            return Err(format!("{name} may only be specified once"));
        }
        if option.kind == "boolean" {
            match name.as_str() {
                "--json" => args.json = true,
                "--progress" => args.progress = true,
                "--deny-warnings" => args.deny_warnings = true,
                "--check" => args.format_check = true,
                "--write" => args.write = true,
                "--check-format" => args.check_format = true,
                "--tabs" => args.tabs = true,
                _ => return Err(format!("unsupported boolean option '{name}'")),
            }
            i += 1;
            continue;
        }
        let value = argv
            .get(i + 1)
            .ok_or_else(|| format!("{name} needs a value"))?;
        let integer = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("{name} requires a nonnegative integer"))
        };
        let dimension = || u32::try_from(integer()?).map_err(|_| format!("{name} exceeds u32"));
        match name.as_str() {
            "--set" => {
                let (key, value) = value.split_once('=').ok_or("--set requires name=JSON")?;
                let value = serde_json::from_str(value).map_err(|e| format!("--set {key}: {e}"))?;
                if args.sets.insert(key.into(), value).is_some() {
                    return Err(format!("--set repeats '{key}'"));
                }
            }
            "--output" => args.output = Some(PathBuf::from(value)),
            "--ffmpeg" => args.ffmpeg = Some(PathBuf::from(value)),
            "--frame" => args.frame = integer()?,
            "--time" => {
                let time = Number::parse(value)?;
                if time.unit != sanim_core::Unit::Duration {
                    return Err("--time requires a duration with s or ms suffix".into());
                }
                args.time = Some(time.value);
            }
            "--width" => args.width = Some(dimension()?),
            "--height" => args.height = Some(dimension()?),
            "--dpi" => args.dpi = Some(dimension()?),
            "--fps" => {
                let rate = Number::parse(value)?;
                if !matches!(rate.unit, sanim_core::Unit::Number | sanim_core::Unit::Rate) {
                    return Err(
                        "--fps requires a numeric rate, for example 30 or 30000/1001".into(),
                    );
                }
                args.fps = Some(rate.value);
            }
            "--cycles" => args.cycles = Some(integer()?),
            "--samples" => args.samples = Some(dimension()?),
            "--warn-operations" => args.warn_operations = Some(integer()?),
            "--warn-prepare-ms" => args.warn_prepare_ms = Some(integer()?),
            "--indent-width" => args.indent_width = Some(dimension()? as usize),
            "--line-width" => args.line_width = Some(dimension()? as usize),
            _ => return Err(format!("unknown option '{name}'")),
        }
        i += 2;
    }
    if matches!(args.command.as_str(), "render" | "video" | "watch") && args.output.is_none() {
        return Err("render/video/watch requires --output PATH".into());
    }
    if args.cycles == Some(0) {
        return Err("--cycles must be positive".into());
    }
    if args
        .samples
        .is_some_and(|samples| samples == 0 || samples > profile::MAX_SAMPLES)
    {
        return Err(format!("--samples requires 1..={}", profile::MAX_SAMPLES));
    }
    if args.time.is_some() && seen.contains("frame") {
        return Err("--time and --frame select the same output; specify one".into());
    }
    if args
        .warn_prepare_ms
        .is_some_and(|value| value.checked_mul(1000).is_none())
    {
        return Err("--warn-prepare-ms exceeds the supported range".into());
    }
    if args.format_check && args.write {
        return Err("--check and --write are mutually exclusive".into());
    }
    if args.write && args.path == Path::new("-") {
        return Err("--write requires an existing .sanim file; stdin cannot be replaced".into());
    }
    if args.command != "fmt"
        && !args.check_format
        && (args.indent_width.is_some() || args.line_width.is_some() || args.tabs)
    {
        return Err("formatting options on lint/check require --check-format".into());
    }
    args.format_options().validate()?;
    args.viewport().validate()?;
    Ok(args)
}

fn execute(args: Args) -> Result<u8, String> {
    if args.command == "fmt" {
        return source_format::run(&args);
    }
    let parent = args
        .path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let sources = FileSources::new(parent)?;
    let entry = args
        .path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("script filename must be UTF-8")?;
    if args.command == "watch" {
        let runtime =
            sanim_runtime::Runtime::with_viewport(&args.path, args.viewport(), args.limits())?;
        let mut lint_engine = Engine::new(args.viewport());
        args.configure(&mut lint_engine);
        if !args.sets.is_empty() {
            runtime.set_inputs(args.sets.clone())?;
        }
        let (wake, notifications) = std::sync::mpsc::sync_channel(1);
        runtime.set_waker(Some(std::sync::Arc::new(move || {
            let _ = wake.try_send(());
        })));
        let mut renderer = Renderer::default();
        let mut cycles = 0;
        let mut revision = 0;
        let mut failed_generation = None;
        let mut progress = None;
        while notifications.recv().is_ok() {
            let state = runtime.snapshot();
            if args.progress && progress.as_ref() != Some(&state.progress) {
                eprintln!(
                    "{}",
                    serde_json::to_string(&state.progress).map_err(|e| e.to_string())?
                );
                progress = Some(state.progress.clone());
            }
            if state.preparing {
                continue;
            }
            if let Some(error) = &state.diagnostic {
                if failed_generation == Some(state.generation) {
                    continue;
                }
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"failed","diagnostic":error,"last_good_revision":state.prepared.as_ref().map(|_|state.revision)})
                );
                failed_generation = Some(state.generation);
            } else if let Some(prepared) = &state.prepared {
                let recovered = failed_generation.take().is_some();
                if state.revision == revision && !recovered {
                    continue;
                }
                let lint = match lint_engine.lint(entry, &sources) {
                    Ok(lint) => lint,
                    Err(error) => {
                        eprintln!(
                            "{}",
                            serde_json::json!({"event":"failed","diagnostic":error,"last_good_revision":revision})
                        );
                        continue;
                    }
                };
                let warnings = lint.warning_count() + prepared.performance.len();
                if args.deny_warnings && warnings != 0 {
                    println!(
                        "{}",
                        serde_json::json!({"ok":false,"event":"warnings_denied","diagnostics":lint.diagnostics,"performance":performance_diagnostics(&prepared.performance)})
                    );
                    return Ok(1);
                }
                let render_micros = render(prepared, &args, &mut renderer)?;
                println!(
                    "{}",
                    serde_json::json!({"event":"updated","revision":state.revision,"changed_elements":state.changed_elements,"render_micros":render_micros,"statistics":prepared.statistics,"performance":performance_diagnostics(&prepared.performance),"diagnostics":lint.diagnostics})
                );
                revision = state.revision;
            } else {
                continue;
            }
            cycles += 1;
            if args.cycles.is_some_and(|limit| cycles >= limit) {
                break;
            }
        }
        return Ok(0);
    }
    let mut engine = Engine::new(args.viewport());
    engine.set_progress_observer(progress_observer(args.progress));
    args.configure(&mut engine);
    let mut lint = engine
        .lint(entry, &sources)
        .map_err(|error| diagnostic(&error, args.json))?;
    if args.check_format {
        source_format::check_lint(&args, entry, &sources, &mut lint)?;
    }
    if args.command == "lint" {
        let accepted = !args.deny_warnings || lint.warning_count() == 0;
        if args.json {
            let mut value = serde_json::to_value(&lint).map_err(|error| error.to_string())?;
            value["ok"] = accepted.into();
            value["warning_count"] = lint.warning_count().into();
            println!("{value}");
        } else {
            println!(
                "Sanim lint: {} warning(s); source and imports type-check",
                lint.warning_count()
            );
            print_diagnostics(
                &serde_json::to_value(&lint.diagnostics).map_err(|error| error.to_string())?,
            );
        }
        return Ok(u8::from(!accepted));
    }
    if args.command == "duration" {
        let report = engine
            .duration_json(entry, &sources, &args.sets)
            .map_err(|e| diagnostic(&e, args.json))?;
        if args.json {
            println!(
                "{}",
                serde_json::json!({"ok":!args.deny_warnings || lint.warning_count()+report.performance.len()==0,"seconds":report.duration.to_string(),"frames":report.frames,"viewport":report.viewport,"dependencies":report.dependencies,"statistics":report.statistics,"diagnostics":lint.diagnostics,"performance":performance_diagnostics(&report.performance)})
            );
        } else {
            println!("{}s ({} frames)", report.duration, report.frames);
            print_diagnostics(
                &serde_json::to_value(&lint.diagnostics).map_err(|error| error.to_string())?,
            );
            print_performance(&report.performance);
        }
        return Ok(u8::from(
            args.deny_warnings && lint.warning_count() + report.performance.len() != 0,
        ));
    }

    let prepared = engine
        .prepare_json(entry, &sources, &args.sets)
        .map_err(|e| diagnostic(&e, args.json))?;
    if args.command == "check" {
        let mut renderer = Renderer::default();
        let scene = renderer
            .prepare(prepared.animation.clone())
            .map_err(|message| {
                diagnostic(
                    &sanim_core::Diagnostic::new(
                        "R001",
                        message,
                        entry,
                        sanim_core::Span::default(),
                    ),
                    args.json,
                )
            })?;
        renderer.validate_geometry(&scene).map_err(|message| {
            diagnostic(
                &sanim_core::Diagnostic::new("R001", message, entry, sanim_core::Span::default()),
                args.json,
            )
        })?;
    }
    let warnings = lint.warning_count() + prepared.performance.len();
    let accepted = !args.deny_warnings || warnings == 0;
    if !args.json {
        print_diagnostics(
            &serde_json::to_value(&lint.diagnostics).map_err(|error| error.to_string())?,
        );
        print_performance(&prepared.performance);
    }
    if !accepted {
        if args.json {
            println!(
                "{}",
                serde_json::json!({"ok":false,"diagnostics":lint.diagnostics,"performance":performance_diagnostics(&prepared.performance),"statistics":prepared.statistics})
            );
        }
        return Ok(1);
    }
    match args.command.as_str() {
        "inspect" => {
            let mut value = serde_json::to_value(&prepared).map_err(|error| error.to_string())?;
            value["diagnostics"] =
                serde_json::to_value(&lint.diagnostics).map_err(|error| error.to_string())?;
            value["performance"] =
                serde_json::json!(performance_diagnostics(&prepared.performance));
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
            );
        }
        "video" => video::render(&prepared, &args, &lint)?,
        "profile" => {
            let result = profile::run(&prepared, &args)?;
            println!(
                "{}",
                serde_json::json!({"ok":true,"profile":result,"statistics":prepared.statistics,"performance":performance_diagnostics(&prepared.performance),"diagnostics":lint.diagnostics})
            );
        }
        "render" => {
            let render_micros = render(&prepared, &args, &mut Renderer::default())?;
            println!(
                "{}",
                serde_json::json!({"ok":true,"output":args.output,"render_micros":render_micros,"statistics":prepared.statistics,"performance":performance_diagnostics(&prepared.performance),"diagnostics":lint.diagnostics})
            );
        }
        _ => {
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"ok":true,"statistics":prepared.statistics,"performance":performance_diagnostics(&prepared.performance),"checks":prepared.checks,"diagnostics":lint.diagnostics})
                );
            } else {
                println!(
                    "Valid Sanim: {} layers, {}s, {}fps; {} visual checks",
                    prepared.animation.layers.len(),
                    prepared.animation.duration,
                    prepared.animation.rate,
                    prepared.checks.len()
                );
            }
        }
    }
    Ok(0)
}

fn print_diagnostics(diagnostics: &serde_json::Value) {
    if let Some(diagnostics) = diagnostics.as_array() {
        for diagnostic in diagnostics {
            eprintln!(
                "{}:{}:{}: {} {}: {}",
                diagnostic["source"].as_str().unwrap_or(""),
                diagnostic["span"]["line"].as_u64().unwrap_or(0),
                diagnostic["span"]["column"].as_u64().unwrap_or(0),
                diagnostic["severity"].as_str().unwrap_or("warning"),
                diagnostic["code"].as_str().unwrap_or(""),
                diagnostic["message"].as_str().unwrap_or("")
            );
        }
    }
}

fn performance_diagnostics(warnings: &[sanim_lang::PerformanceWarning]) -> Vec<serde_json::Value> {
    warnings
        .iter()
        .map(|warning| {
            let mut value = serde_json::to_value(warning).expect("performance warnings serialize");
            value["severity"] = "warning".into();
            value
        })
        .collect()
}

fn print_performance(warnings: &[sanim_lang::PerformanceWarning]) {
    for warning in warnings {
        eprintln!(
            "{}:{}:{}: warning {}: {}",
            warning.source, warning.span.line, warning.span.column, warning.code, warning.message
        );
    }
}

fn diagnostic(error: &sanim_core::Diagnostic, json: bool) -> String {
    if json {
        serde_json::to_string(error).unwrap_or_else(|_| error.to_string())
    } else {
        error.to_string()
    }
}

fn progress_observer(enabled: bool) -> Option<sanim_lang::progress::Observer> {
    enabled.then(|| {
        std::sync::Arc::new(|progress| {
            eprintln!(
                "{}",
                serde_json::json!({"event":"progress","progress":progress})
            );
        }) as sanim_lang::progress::Observer
    })
}
fn render(prepared: &Prepared, args: &Args, renderer: &mut Renderer) -> Result<u64, String> {
    let started = std::time::Instant::now();
    let (scene, options) = render_scene(prepared, args, renderer)?;
    let time = if let Some(time) = args.time {
        time
    } else {
        scene.animation().frame_time(args.frame)?
    };
    if time > scene.animation().duration {
        return Err(format!(
            "requested time {time}s exceeds animation duration {}s",
            scene.animation().duration
        ));
    }
    let frame = renderer.render(&scene, time, &options)?;
    let output = args.output.as_ref().ok_or("missing output path")?;
    let temporary = output.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, &frame.png).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, output).map_err(|e| e.to_string())?;
    Ok(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX))
}

fn render_scene(
    prepared: &Prepared,
    _args: &Args,
    renderer: &mut Renderer,
) -> Result<(Scene, Options), String> {
    let scene = renderer.prepare(prepared.animation.clone())?;
    let options = Options::for_animation(scene.animation());
    Ok((scene, options))
}
impl Args {
    fn format_options(&self) -> sanim_lang::formatter::FormatOptions {
        let default = sanim_lang::formatter::FormatOptions::default();
        sanim_lang::formatter::FormatOptions {
            indent_width: self.indent_width.unwrap_or(default.indent_width),
            line_width: self.line_width.unwrap_or(default.line_width),
            use_tabs: self.tabs,
        }
    }
    fn configure(&self, engine: &mut Engine) {
        engine.limits = self.limits();
        if matches!(self.command.as_str(), "render" | "video" | "profile") {
            engine.preparation = sanim_lang::Preparation::Demand;
        }
    }
    fn limits(&self) -> sanim_lang::Limits {
        let mut limits = sanim_lang::Limits::default();
        if let Some(operations) = self.warn_operations {
            limits.warn_operations = operations;
        }
        if let Some(milliseconds) = self.warn_prepare_ms {
            limits.warn_prepare_micros = milliseconds * 1000;
        }
        limits
    }
    fn viewport(&self) -> sanim_core::Viewport {
        let default = sanim_core::Viewport::default();
        sanim_core::Viewport {
            width: self.width.unwrap_or(default.width),
            height: self.height.unwrap_or(default.height),
            rate: self.fps.unwrap_or(default.rate),
            dpi: self.dpi.unwrap_or(default.dpi),
        }
    }
}
