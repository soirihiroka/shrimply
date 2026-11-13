//! One discoverable command/option catalog shared by both executable entry points.
use serde_json::{Value, json};

pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
}

pub const COMMANDS: &[Command] = &[
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
        name: "ffmpeg",
        kind: "string",
        summary: "FFmpeg executable for video (default ffmpeg)",
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
        summary: "Requested cold/warm profile sample count (default 30, maximum 10000)",
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
        "output" => matches!(command, "render" | "video" | "watch"),
        "ffmpeg" => command == "video",
        "frame" | "time" => matches!(command, "render" | "watch"),
        "cycles" => command == "watch",
        "samples" => command == "profile",
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
        "summary":definition.summary,"project_required":false,"read_only":!matches!(command,"render"|"video"|"watch"|"fmt"),
        "request":{"type":"object","properties":properties},
        "positional":{"file":{"type":"string","required":true,"description":"Sanim entry file; resources resolve relative to its directory"}},
        "output_required":matches!(command,"render"|"video"|"watch"),
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
