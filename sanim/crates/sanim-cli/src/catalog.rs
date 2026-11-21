//! One discoverable command/option catalog shared by both executable entry points.
use serde_json::{Value, json};

pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "voxel-watch",
        summary: "Watch voxel source/textures using native events and retain GPU resources between edits (also: voxel watch)",
    },
    Command {
        name: "voxel-render",
        summary: "Render an independent voxel scene to PNG (also: voxel render)",
    },
    Command {
        name: "voxel-inspect",
        summary: "Inspect one sampled voxel state from named camera angles (also: voxel inspect)",
    },
    Command {
        name: "voxel-pick",
        summary: "Resolve a view pixel to its voxel, face, material and distance (also: voxel pick)",
    },
    Command {
        name: "voxel-video",
        summary: "Render a voxel animation to H.264 MP4 (also: voxel video)",
    },
    Command {
        name: "fmt",
        summary: "Format Sanim source with the canonical parser; check or atomically write on request",
    },
    Command {
        name: "lint",
        summary: "Lint Sanim source and imports for type errors and performance warnings without evaluating data",
    },
    Command {
        name: "check",
        summary: "Fully evaluate and validate Sanim, including deferred values and visual assertions",
    },
    Command {
        name: "inspect",
        summary: "Inspect the fully evaluated typed animation, bindings, reflected inputs, and diagnostics",
    },
    Command {
        name: "duration",
        summary: "Resolve exact bounded timeline duration without unnecessary scene work",
    },
    Command {
        name: "profile",
        summary: "Profile editor performance, preparation, and cold/warm frame caches without rasterization",
    },
    Command {
        name: "render",
        summary: "Render a requested Sanim frame to PNG with native Skia",
    },
    Command {
        name: "video",
        summary: "Render a bounded Sanim timeline to H.264 MP4 through FFmpeg",
    },
    Command {
        name: "watch",
        summary: "Watch Sanim source and data changes using native events and render updated PNGs",
    },
];

pub struct OptionDefinition {
    pub name: &'static str,
    pub kind: &'static str,
    pub summary: &'static str,
}

pub const OPTIONS: &[OptionDefinition] = &[
    OptionDefinition {
        name: "reconstruction",
        kind: "string",
        summary: "Voxel image reconstruction: off, spatial (default), or temporal; filters lighting while preserving texture detail",
    },
    OptionDefinition {
        name: "spatial-passes",
        kind: "integer",
        summary: "Edge-aware lighting-filter passes, 0..=3",
    },
    OptionDefinition {
        name: "history-frames",
        kind: "integer",
        summary: "Temporal reconstruction sample window, 1..=32; retains one bounded history",
    },
    OptionDefinition {
        name: "antialias",
        kind: "integer",
        summary: "Voxel primary visibility samples per pixel, 1..=4 (default 2); path tracing uses its path samples",
    },
    OptionDefinition {
        name: "shadows",
        kind: "string",
        summary: "Voxel key shadows: auto, off, hard, or soft (auto: flat off, path traced soft)",
    },
    OptionDefinition {
        name: "shadow-strength",
        kind: "number",
        summary: "Voxel shadow strength, 0..=1",
    },
    OptionDefinition {
        name: "shadow-samples",
        kind: "integer",
        summary: "Voxel key visibility rays per shading sample, 1..=8",
    },
    OptionDefinition {
        name: "ao",
        kind: "string",
        summary: "Voxel ambient occlusion: auto, off, local, or ray-traced (auto: flat local, path traced off)",
    },
    OptionDefinition {
        name: "ao-strength",
        kind: "number",
        summary: "Voxel ambient occlusion strength, 0..=1",
    },
    OptionDefinition {
        name: "ao-distance",
        kind: "number",
        summary: "Ray-traced ambient occlusion radius in voxel cells",
    },
    OptionDefinition {
        name: "ao-samples",
        kind: "integer",
        summary: "Ambient occlusion rays per primary shading sample, 1..=16",
    },
    OptionDefinition {
        name: "exposure",
        kind: "number",
        summary: "Voxel foreground exposure in stops, -16..=16; background unchanged",
    },
    OptionDefinition {
        name: "sun-diameter",
        kind: "string",
        summary: "Voxel sun's angular diameter, e.g. 8deg; larger sources produce softer shadows",
    },
    OptionDefinition {
        name: "sun-strength",
        kind: "string",
        summary: "Voxel key-light strength; finite nonnegative number",
    },
    OptionDefinition {
        name: "environment-strength",
        kind: "string",
        summary: "Voxel sky-fill strength, independent of the visible background",
    },
    OptionDefinition {
        name: "mode",
        kind: "string",
        summary: "Voxel rendering: flat (fast default) or path-traced (bounded Monte Carlo lighting)",
    },
    OptionDefinition {
        name: "bounces",
        kind: "integer",
        summary: "Path-traced voxel light-bounce limit",
    },
    OptionDefinition {
        name: "views",
        kind: "string",
        summary: "voxel inspect: comma-separated front,back,left,right,top,bottom,isometric",
    },
    OptionDefinition {
        name: "view",
        kind: "string",
        summary: "Override the voxel camera with one named, fitted inspection view",
    },
    OptionDefinition {
        name: "pixel",
        kind: "string",
        summary: "voxel pick: output pixel X,Y, with origin at the image top-left",
    },
    OptionDefinition {
        name: "check",
        kind: "boolean",
        summary: "fmt: fail if source needs formatting; never write",
    },
    OptionDefinition {
        name: "write",
        kind: "boolean",
        summary: "fmt: atomically replace the .sanim source after successful formatting",
    },
    OptionDefinition {
        name: "check-format",
        kind: "boolean",
        summary: "lint/check: include canonical source formatting as a warning",
    },
    OptionDefinition {
        name: "indent-width",
        kind: "integer",
        summary: "Formatter indentation width, 1..=16 (default 4)",
    },
    OptionDefinition {
        name: "line-width",
        kind: "integer",
        summary: "Formatter preferred line width, 20..=1000 (default 100)",
    },
    OptionDefinition {
        name: "tabs",
        kind: "boolean",
        summary: "Use tabs for source formatting indentation",
    },
    OptionDefinition {
        name: "json",
        kind: "boolean",
        summary: "Emit structured results and source diagnostics",
    },
    OptionDefinition {
        name: "progress",
        kind: "boolean",
        summary: "Emit observed preparation events on stderr",
    },
    OptionDefinition {
        name: "deny-warnings",
        kind: "boolean",
        summary: "Return a failing status when lint or performance warnings are present",
    },
    OptionDefinition {
        name: "set",
        kind: "string",
        summary: "Reflected override name=JSON (repeatable)",
    },
    OptionDefinition {
        name: "output",
        kind: "string",
        summary: "PNG or MP4 destination (required for render/video/watch)",
    },
    OptionDefinition {
        name: "duration",
        kind: "string",
        summary: "External time viewport, e.g. 10s (required for indefinite video)",
    },
    OptionDefinition {
        name: "ffmpeg",
        kind: "string",
        summary: "Override the project FFmpeg executable for video",
    },
    OptionDefinition {
        name: "frame",
        kind: "integer",
        summary: "Exact zero-based frame index (default 0)",
    },
    OptionDefinition {
        name: "time",
        kind: "string",
        summary: "Exact presentation duration, e.g. 125ms or 4m1s",
    },
    OptionDefinition {
        name: "width",
        kind: "integer",
        summary: "Output width in pixels (default 1920)",
    },
    OptionDefinition {
        name: "height",
        kind: "integer",
        summary: "Output height in pixels (default 1080)",
    },
    OptionDefinition {
        name: "fps",
        kind: "string",
        summary: "Exact frame rate (default 30; accepts 30000/1001)",
    },
    OptionDefinition {
        name: "dpi",
        kind: "integer",
        summary: "Output DPI (default 96, CSS points)",
    },
    OptionDefinition {
        name: "cycles",
        kind: "integer",
        summary: "Stop watching after N completed revisions",
    },
    OptionDefinition {
        name: "samples",
        kind: "integer",
        summary: "Profile sample count, or voxel path-tracing light samples per pixel",
    },
    OptionDefinition {
        name: "max-native-work",
        kind: "integer",
        summary: "Bound cumulative native algorithm work independently of interpreted DSL operations",
    },
    OptionDefinition {
        name: "warn-operations",
        kind: "integer",
        summary: "Warn when one statement reaches this preparation operation count",
    },
    OptionDefinition {
        name: "warn-prepare-ms",
        kind: "integer",
        summary: "Warn when total preparation takes this many milliseconds",
    },
];

pub(crate) fn options_for(command: &str) -> impl Iterator<Item = &'static OptionDefinition> {
    OPTIONS.iter().filter(move |option| match option.name {
        "check" | "write" => command == "fmt",
        "check-format" => matches!(command, "lint" | "check"),
        "indent-width" | "line-width" | "tabs" => matches!(command, "fmt" | "lint" | "check"),
        "views" => command == "voxel-inspect",
        "view" => command.starts_with("voxel-"),
        "pixel" => command == "voxel-pick",
        "output" => matches!(
            command,
            "render"
                | "video"
                | "watch"
                | "voxel-render"
                | "voxel-video"
                | "voxel-inspect"
                | "voxel-watch"
        ),
        "ffmpeg" => matches!(command, "video" | "voxel-video"),
        "duration" => command != "fmt",
        "frame" | "time" => matches!(
            command,
            "render" | "watch" | "voxel-render" | "voxel-inspect" | "voxel-pick" | "voxel-watch"
        ),
        "cycles" => matches!(command, "watch" | "voxel-watch"),
        "mode"
        | "bounces"
        | "sun-diameter"
        | "sun-strength"
        | "environment-strength"
        | "shadows"
        | "shadow-strength"
        | "shadow-samples"
        | "ao"
        | "ao-strength"
        | "ao-distance"
        | "ao-samples"
        | "exposure" => command.starts_with("voxel-") && command != "voxel-pick",
        "samples" => {
            command == "profile" || command.starts_with("voxel-") && command != "voxel-pick"
        }
        "set" => !matches!(command, "fmt" | "lint"),
        "json" => true,
        _ => command != "fmt",
    })
}

pub fn schema(command: &str) -> Result<Value, String> {
    let definition = COMMANDS
        .iter()
        .find(|definition| definition.name == command)
        .ok_or_else(|| format!("unknown Sanim command '{command}'"))?;
    let properties = options_for(command).map(|option| (option.name.replace('-', "_"), json!({
        "type":option.kind,"description":option.summary,"flag":format!("--{}",option.name),
        "repeatable":option.name == "set"
    }))).collect::<serde_json::Map<_,_>>();
    Ok(json!({
        "command":format!("shrimply sanim {}",definition.name),
        "summary":definition.summary,"project_required":false,"read_only":!matches!(command,"render"|"video"|"watch"|"fmt"|"voxel-render"|"voxel-video"|"voxel-inspect"|"voxel-watch"),
        "request":{"type":"object","properties":properties},
        "positional":{"file":{"type":"string","required":true,"description":"Sanim entry file; resources resolve relative to its directory"}},
        "output_required":matches!(command,"render"|"video"|"watch"|"voxel-render"|"voxel-video"|"voxel-watch"),
        "diagnostics":"code, severity, source, span, message; exit 1 on errors or denied warnings, exit 2 on invalid arguments",
        "evaluation":if matches!(command,"check"|"inspect") { "full" } else if matches!(command,"lint"|"fmt") { "static" } else { "demand" }
    }))
}

pub(crate) fn help() -> String {
    let mut text = String::from(
        "Sanim development tools (also available as shrimply sanim)\n\nUsage: sanim <command> <file.sanim> [options]\n       shrimply sanim <command> <file.sanim> [options]\n\nCommands:\n",
    );
    for command in COMMANDS {
        text.push_str(&format!("  {:<10} {}\n", command.name, command.summary));
    }
    text.push_str("\nOptions:\n");
    for option in OPTIONS {
        let flag = format!(
            "--{}{}",
            option.name,
            if option.kind == "boolean" {
                ""
            } else {
                " VALUE"
            }
        );
        text.push_str(&format!("  {flag:<27} {}\n", option.summary));
    }
    text
}
