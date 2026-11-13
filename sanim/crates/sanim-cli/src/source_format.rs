//! Syntax-only formatting and explicitly requested atomic source replacement.
use crate::{Args, diagnostic};
use sanim_lang::{Sources, formatter::format_source};
use std::{
    io::{Read, Write},
    path::Path,
};

pub(crate) fn run(args: &Args) -> Result<u8, String> {
    if args.write
        && args
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("sanim")
    {
        return Err("--write only replaces .sanim source files".into());
    }
    let limits = args.limits();
    let mut bytes = Vec::new();
    if args.path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take(limits.max_source_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
    } else {
        std::fs::File::open(&args.path)
            .map_err(|error| format!("cannot open source {}: {error}", args.path.display()))?
            .take(limits.max_source_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
    }
    if bytes.len() > limits.max_source_bytes {
        return Err(format!(
            "source exceeds the {}-byte formatting input limit",
            limits.max_source_bytes
        ));
    }
    let original =
        String::from_utf8(bytes).map_err(|error| format!("source must be UTF-8: {error}"))?;
    let source = args.path.to_str().ok_or("source path must be UTF-8")?;
    let formatted = format_source(
        source,
        &original,
        args.format_options(),
        limits.max_parse_steps,
    )
    .map_err(|error| diagnostic(&error, args.json))?;
    let changed = formatted != original;
    if args.write && changed {
        let target = args
            .path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let metadata = std::fs::metadata(&target).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.permissions().readonly() {
            return Err("source is not a writable regular file".into());
        }
        let directory = target.parent().ok_or("source has no parent directory")?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".sanim-fmt-")
            .tempfile_in(directory)
            .map_err(|error| error.to_string())?;
        temporary
            .write_all(formatted.as_bytes())
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        let mut current = Vec::new();
        std::fs::File::open(&target)
            .map_err(|error| error.to_string())?
            .take(limits.max_source_bytes as u64 + 1)
            .read_to_end(&mut current)
            .map_err(|error| error.to_string())?;
        if current != original.as_bytes() {
            return Err(
                "source changed while formatting; retry after the current edit is saved".into(),
            );
        }
        temporary
            .persist(&target)
            .map_err(|error| format!("cannot publish formatted source: {error}"))?;
    }
    let accepted = !args.format_check || !changed;
    if args.json {
        let mut result = serde_json::json!({"ok":accepted,"source":args.path,"changed":changed,"written":args.write&&changed});
        if !args.format_check && !args.write {
            result["formatted"] = formatted.into();
        }
        println!("{result}");
    } else if args.format_check {
        println!(
            "{}: {}",
            if changed {
                "Needs formatting"
            } else {
                "Formatted"
            },
            args.path.display()
        );
    } else if args.write {
        println!(
            "{}: {}",
            if changed {
                "Formatted"
            } else {
                "Already formatted"
            },
            args.path.display()
        );
    } else {
        print!("{formatted}");
    }
    Ok(u8::from(!accepted))
}

pub(crate) fn check_lint(
    args: &Args,
    entry: &str,
    sources: &dyn Sources,
    lint: &mut sanim_lang::LintReport,
) -> Result<(), String> {
    let bytes = sources.read(entry)?;
    let original =
        std::str::from_utf8(&bytes).map_err(|error| format!("source must be UTF-8: {error}"))?;
    let formatted = format_source(
        entry,
        original,
        args.format_options(),
        args.limits().max_parse_steps,
    )
    .map_err(|error| diagnostic(&error, args.json))?;
    if formatted != original {
        lint.diagnostics.push(sanim_lang::LintDiagnostic {severity:sanim_lang::LintSeverity::Warning,code:"F101".into(),message:"source is not canonically formatted; run shrimply sanim fmt FILE --write (or --check to validate without writing)".into(),source:entry.into(),span:sanim_core::Span {line:1,column:1,..Default::default()}});
    }
    Ok(())
}
