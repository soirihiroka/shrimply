use sanim_core::{Fraction, Number};
use sanim_lang::{Engine, FileSources, Prepared};
use sanim_render::{Options, Renderer, Scene};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::ExitCode,
};

mod video;

const HELP: &str = "sanim — headless animation language\n\nUsage: sanim <check|inspect|duration|render|video|watch> <file.sanim> [options]\n\n  --json                 Structured results and diagnostics\n  --progress             Observed preparation events as JSON on stderr\n  --set name=JSON         Override a reflected input (repeatable)\n  --output PATH           PNG or MP4 destination (render/video/watch)\n  --ffmpeg PATH          FFmpeg executable for video (default ffmpeg)\n  --frame N               Exact zero-based frame index (default 0)\n  --time DURATION         Exact presentation time, e.g. 125ms\n  --width N --height N    Output pixels\n  --fps RATE             Exact frame rate (default 30)\n  --dpi N                Output DPI (default 96; CSS points)\n  --cycles N             Stop watching after N completed revisions (default unlimited)\n";

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
}

fn main() -> ExitCode {
    let argv = std::env::args().skip(1).collect::<Vec<_>>();
    if argv.is_empty() || argv.iter().any(|v| v == "--help" || v == "-h") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let json = argv.iter().any(|v| v == "--json");
    match parse(argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&error) {
                    eprintln!("{}", serde_json::json!({"ok":false,"diagnostic":value}));
                } else {
                    eprintln!("{}", serde_json::json!({"ok":false,"error":error}));
                }
            } else {
                eprintln!("{error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn parse(argv: Vec<String>) -> Result<Args, String> {
    if argv.len() < 2 {
        return Err(HELP.into());
    }
    let mut args = Args {
        command: argv[0].clone(),
        path: PathBuf::from(&argv[1]),
        ..Args::default()
    };
    if !matches!(
        args.command.as_str(),
        "check" | "inspect" | "duration" | "render" | "video" | "watch"
    ) {
        return Err(format!("unknown command '{}'\n{HELP}", args.command));
    }
    let mut i = 2;
    while i < argv.len() {
        let name = &argv[i];
        if name == "--json" || name == "--progress" {
            if name == "--json" {
                args.json = true;
            } else {
                args.progress = true;
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
    args.viewport().validate()?;
    Ok(args)
}

fn run(args: Args) -> Result<(), String> {
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
            sanim_runtime::Runtime::with_viewport(&args.path, args.viewport(), Default::default())?;
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
                let render_micros = render(prepared, &args, &mut renderer)?;
                println!(
                    "{}",
                    serde_json::json!({"event":"updated","revision":state.revision,"changed_elements":state.changed_elements,"render_micros":render_micros,"statistics":prepared.statistics,"performance":prepared.performance})
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
        return Ok(());
    }
    let mut engine = Engine::new(args.viewport());
    engine.set_progress_observer(progress_observer(args.progress));
    if args.command == "duration" {
        let report = engine
            .duration_json(entry, &sources, &args.sets)
            .map_err(|e| diagnostic(&e, args.json))?;
        if args.json {
            println!(
                "{}",
                serde_json::json!({"ok":true,"seconds":report.duration.to_string(),"frames":report.frames,"viewport":report.viewport,"dependencies":report.dependencies,"statistics":report.statistics})
            );
        } else {
            println!("{}s ({} frames)", report.duration, report.frames);
        }
        return Ok(());
    }

    let prepared = engine
        .prepare_json(entry, &sources, &args.sets)
        .map_err(|e| diagnostic(&e, args.json))?;
    match args.command.as_str() {
        "inspect" => println!(
            "{}",
            serde_json::to_string_pretty(&prepared).map_err(|e| e.to_string())?
        ),
        "video" => video::render(&prepared, &args)?,
        "render" => {
            let render_micros = render(&prepared, &args, &mut Renderer::default())?;
            println!(
                "{}",
                serde_json::json!({"ok":true,"output":args.output,"render_micros":render_micros,"statistics":prepared.statistics,"performance":prepared.performance})
            );
        }
        _ => {
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"ok":true,"statistics":prepared.statistics,"performance":prepared.performance,"checks":prepared.checks})
                );
            } else {
                println!(
                    "Valid Sanim: {} layers, {}s, {}fps; {} visual checks",
                    prepared.animation.layers.len(),
                    prepared.animation.duration,
                    prepared.animation.rate,
                    prepared.checks.len()
                );
                for warning in &prepared.performance {
                    eprintln!(
                        "{}:{}: {}: {}",
                        warning.source, warning.span.line, warning.code, warning.message
                    );
                }
            }
        }
    }
    Ok(())
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
