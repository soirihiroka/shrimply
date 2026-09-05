use rust_i18n::t;
use std::borrow::Cow;

rust_i18n::i18n!("locales", fallback = "en");

const DEFAULT_LOCALE: &str = "en";
const SUPPORTED_LOCALES: [&str; 15] = [
    "en", "es", "fr", "de", "ja", "zh-CN", "zh-TW", "ko", "pt", "ru", "tr", "it", "id", "pl", "ar",
];

pub fn init_system_locale() {
    let mut candidates = Vec::new();
    for variable in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable) {
            candidates.extend(value.split(':').map(str::to_string));
        }
    }
    let locale = candidates
        .iter()
        .map(|locale| normalize_locale(locale))
        .find(|locale| SUPPORTED_LOCALES.contains(&locale.as_str()))
        .unwrap_or_else(|| DEFAULT_LOCALE.to_string());
    rust_i18n::set_locale(&locale);
}

pub fn text(key: &str) -> Cow<'_, str> {
    t!(key)
}

pub fn text_args(key: &str, args: &[(&str, String)]) -> String {
    let patterns = args.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let values = args
        .iter()
        .map(|(_, value)| value.clone())
        .collect::<Vec<_>>();

    let translated = text(key);
    let result = rust_i18n::replace_patterns(&translated, &patterns, &values);

    // If a %{...} placeholder survives, the translation's placeholder doesn't
    // match the one passed here (a typo'd %{name}). Never show that raw blob
    // in the UI: fall back to the canonical English string and re-resolve.
    if has_unresolved_placeholder(&result) {
        let english = t!(key, locale = "en");
        if cfg!(debug_assertions) {
            eprintln!(
                "text_args: unresolved placeholder in translation {:?}, fell back to English: {}",
                key, result
            );
        }
        // If English is still unresolved, the argument simply wasn't passed
        // (a different bug). Return it as-is; never attempt a third pass.
        return rust_i18n::replace_patterns(&english, &patterns, &values);
    }

    result
}

/// Returns true if `s` still contains an unresolved `%{name}` placeholder.
fn has_unresolved_placeholder(s: &str) -> bool {
    match s.find("%{") {
        Some(open) => s[open + 2..].contains('}'),
        None => false,
    }
}

fn normalize_locale(locale: &str) -> String {
    let locale = locale
        .split('.')
        .next()
        .unwrap_or(locale)
        .replace('_', "-")
        .to_ascii_lowercase();
    let mut parts = locale.split(['-', '@']);
    let language = parts.next().unwrap_or(DEFAULT_LOCALE);
    if language == "zh" {
        return if parts.any(|part| matches!(part, "hant" | "tw" | "hk" | "mo")) {
            "zh-TW"
        } else {
            "zh-CN"
        }
        .to_string();
    }
    language.to_string()
}
