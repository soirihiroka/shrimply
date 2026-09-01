// Build-time validation for the locale files.
//
// Ensures every translation uses the same set of `%{name}` placeholders as its
// section key (the English canonical text). A mistyped placeholder would
// otherwise render as literal text with no error, so the build fails instead.
//
// All *.toml files under locales/ are checked, matching what the i18n macro
// loads. The `toml` crate handles escaping, comments, and multi-line strings.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use toml::Value;

/// Set of `%{name}` placeholder names in a string, sorted and de-duplicated.
fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while let Some(idx) = s[start..].find("%{") {
        let name_start = start + idx + 2;
        match s[name_start..].find('}') {
            Some(close) => {
                out.push(s[name_start..name_start + close].to_string());
                start = name_start + close + 1;
            }
            None => break,
        }
    }
    out.sort();
    out.dedup();
    out
}

fn validate_file(path: &Path) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("i18n validation: failed to read {}: {}", path.display(), e);
            process::exit(1);
        }
    };

    let parsed: Value = match toml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("i18n validation: failed to parse {}: {}", path.display(), e);
            process::exit(1);
        }
    };

    let Some(root) = parsed.as_table() else {
        return;
    };

    for (section, content) in root {
        if section == "_version" {
            continue;
        }

        let ref_placeholders = placeholders(section);
        if let Some(inner) = content.as_table() {
            for (locale, translation) in inner {
                let Some(translation) = translation.as_str() else {
                    continue;
                };

                let got = placeholders(translation);
                if got != ref_placeholders {
                    eprintln!(
                        "i18n validation: placeholder mismatch in {} ({})\n  section:  {:?}\n  expected: {{ {} }}\n  found:    {{ {} }}",
                        path.display(),
                        locale,
                        section,
                        ref_placeholders.join(", "),
                        got.join(", ")
                    );
                    process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let locales = Path::new(&dir).join("locales");

    // Watch the directory so newly added/removed locale files trigger a re-run.
    println!("cargo:rerun-if-changed={}", locales.display());

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&locales).expect("locales dir").flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("toml") {
            println!("cargo:rerun-if-changed={}", path.display());
            files.push(path);
        }
    }

    // Deterministic validation order regardless of filesystem enumeration.
    files.sort();
    for path in &files {
        validate_file(path);
    }
}
