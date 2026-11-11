use clap::{Arg, Command};
use serde_json::Value;
use shrimply_editing_api::commands::{COMMANDS, Command as Definition};

const DISCOVERY: &str =
    "Start with shrimply cli search \"<task>\". Do not explore by chaining --help calls.
Inspect a matched command with --help or shrimply schema <command>.";

pub fn cli() -> Command {
    let mut root = branch("shrimply", &[])
        .before_help(DISCOVERY)
        .arg(
            Arg::new("project")
                .long("project")
                .alias("project-path")
                .global(true)
                .value_name("PATH")
                .help("Explicit project target; relative paths are accepted"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .global(true)
                .action(clap::ArgAction::SetTrue)
                .help(
                    "Validate and preview without changing persistent state or invoking services",
                ),
        )
        .arg(
            Arg::new("if-revision")
                .long("if-revision")
                .global(true)
                .value_name("REVISION")
                .help("Reject if the project content revision differs"),
        );
    root = root.subcommand(
        Command::new("cli").about("Discover operations").subcommand(
            Command::new("search")
                .about("Search command names, descriptions, argument names, and synonyms locally")
                .arg(Arg::new("query").required(true)),
        ),
    );
    root = root.subcommand(
        Command::new("schema")
            .about("Inspect a command's typed request schema")
            .arg(Arg::new("command").num_args(1..).required(true)),
    );
    root = root.version(env!("CARGO_PKG_VERSION"));
    root
}

fn branch(name: &str, prefix: &[&str]) -> Command {
    let mut command = Command::new(name.to_owned()).subcommand_required(true);
    let mut children = std::collections::BTreeSet::new();
    for definition in COMMANDS {
        let parts: Vec<_> = definition.path.split_whitespace().collect();
        if parts.starts_with(prefix) && parts.len() > prefix.len() {
            children.insert(parts[prefix.len()]);
        }
    }
    for child in children {
        let mut path = prefix.to_vec();
        path.push(child);
        let full = path.join(" ");
        if let Some(definition) = COMMANDS.iter().find(|definition| definition.path == full) {
            command = command.subcommand(leaf(child, definition));
        } else {
            command = command.subcommand(branch(child, &path));
        }
    }
    command
}

fn leaf(name: &str, definition: &Definition) -> Command {
    let schema = (definition.schema)();
    let mut command = Command::new(name.to_owned())
        .about(definition.summary)
        .after_help(format!("Schema: shrimply schema {}", definition.path));
    let mut fields = std::collections::BTreeSet::new();
    if let Some(properties) = schema
        .as_value()
        .get("properties")
        .and_then(Value::as_object)
    {
        for (field, property) in properties {
            if field == "project_path" {
                continue;
            }
            fields.insert(field.as_str());
            let flag = field.replace('_', "-");
            let description = property
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("See the request schema for type and constraints");
            let mut arg = Arg::new(field.clone())
                .long(flag)
                .help(description.to_owned())
                .allow_hyphen_values(true);
            if definition.name == "render_project" && field == "output" {
                arg = arg.short('o');
            }
            if is_boolean(property_type(property, schema.as_value())) {
                arg = arg
                    .allow_hyphen_values(false)
                    .require_equals(true)
                    .num_args(0..=1)
                    .default_missing_value("true")
                    .value_parser(["true", "false"]);
            }
            command = command.arg(arg);
        }
    }
    if definition.name == "render_project" {
        command = command.arg(
            Arg::new("render_project_path")
                .value_name("PROJECT")
                .index(1)
                .help("Saved project (or use --project)"),
        );
    }
    command = command.arg(Arg::new("body").long("body").help(
        "Complete JSON operation arguments, @file, or @- for stdin; conflicts with operation flags",
    ));
    if fields.contains("address") || fields.contains("addresses") {
        if !fields.contains("item_id") {
            command =
                command.arg(Arg::new("item_id").long("item-id").help(
                    "Resolve a unique clip presentation; ambiguity returns concrete addresses",
                ));
        }
        if !fields.contains("track_id") {
            command = command.arg(
                Arg::new("track_id")
                    .long("track-id")
                    .help("Resolve a unique track or narrow an item ID"),
            );
        }
    }
    if definition.name == "connect_project" {
        command = command.arg(
            Arg::new("headless")
                .long("headless")
                .action(clap::ArgAction::SetTrue)
                .help(
                    "Start a headless owner if closed; an existing GUI or headless owner is reused",
                ),
        );
    }
    if definition.name == "view_frame" {
        command = command.arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .help("Write PNG bytes here and return the path instead of base64"),
        );
    }
    command
}

pub fn is_boolean(property: &Value) -> bool {
    property.get("type").is_some_and(|kind| {
        kind == "boolean"
            || kind
                .as_array()
                .is_some_and(|types| types.contains(&Value::String("boolean".into())))
    }) || property
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|types| types.iter().any(is_boolean))
}

pub fn is_string(property: &Value) -> bool {
    property.get("type").is_some_and(|kind| {
        kind == "string"
            || kind
                .as_array()
                .is_some_and(|types| types.contains(&Value::String("string".into())))
    }) || property
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|types| types.iter().any(is_string))
}

pub fn property_type<'a>(property: &'a Value, schema: &'a Value) -> &'a Value {
    if let Some(reference) = property
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        && let Some(value) = schema.pointer(reference)
    {
        return property_type(value, schema);
    }
    if let Some(value) = property
        .get("anyOf")
        .and_then(Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .find(|value| value.get("type") != Some(&Value::String("null".into())))
        })
    {
        return property_type(value, schema);
    }
    property
}
