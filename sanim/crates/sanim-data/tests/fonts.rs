use sanim_core::{Font, Fraction};
use std::sync::Arc;
const FONT: &[u8] = include_bytes!("../../../examples/assets/DejaVuSansCondensed-subset.ttf");
#[test]
fn pango_shaping_bounds_cover_outlines_and_missing_glyphs_fail() {
    let (bounds, path) = sanim_data::font::shape(
        &Font::Asset(Arc::new(FONT.to_vec())),
        "AV ffi السلام",
        Fraction::from(24),
    )
    .unwrap();
    assert!(!path.is_empty());
    assert!(bounds.max.values[0] > bounds.min.values[0]);
    let error = sanim_data::font::shape(
        &Font::Asset(Arc::new(FONT.to_vec())),
        "中",
        Fraction::from(24),
    )
    .unwrap_err();
    assert!(error.contains("missing glyph"), "{error}");
    // This subset omits Ω, even if the full DejaVu family is installed locally.
    // Selecting an ambient font by the same family name must not make it appear.
    let error = sanim_data::font::shape(
        &Font::Asset(Arc::new(FONT.to_vec())),
        "Ω",
        Fraction::from(24),
    )
    .unwrap_err();
    assert!(error.contains("missing glyph"), "{error}");
    assert!(sanim_data::font::validate(b"broken font").is_err());
}

#[test]
fn explicit_font_assets_ignore_ambient_coretext_backend_selection() {
    const CHILD: &str = "SANIM_EXPLICIT_FONT_BACKEND_TEST";
    if std::env::var_os(CHILD).is_some() {
        let (bounds, path) = sanim_data::font::shape(
            &Font::Asset(Arc::new(FONT.to_vec())),
            "AV ffi السلام",
            Fraction::from(24),
        )
        .unwrap();
        assert!(!path.is_empty());
        assert!(bounds.max.values[0] > bounds.min.values[0]);
        return;
    }
    // Isolate the environment from other parallel tests. On a macOS build with
    // both backends this reproduces the default that used to reject add_font_file.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "explicit_font_assets_ignore_ambient_coretext_backend_selection",
        ])
        .env(CHILD, "1")
        .env("PANGOCAIRO_BACKEND", "coretext")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_preferred_family_uses_the_explicit_native_fallback() {
    let fallback = Font::family("sans-serif", "sans-serif").unwrap();
    let missing = Font::family("Sanim Missing Family 9f8075b4", "sans-serif").unwrap();
    let direct = sanim_data::font::shape(&fallback, "Stack & Queue 123", 24.into()).unwrap();
    let resolved = sanim_data::font::shape(&missing, "Stack & Queue 123", 24.into()).unwrap();
    assert_eq!(direct, resolved);
    assert!(!resolved.1.is_empty());
    assert!(sanim_data::font::system_font_epoch().is_some());
}

#[test]
fn font_family_descriptors_reject_ambiguous_or_unsafe_names() {
    for (family, fallback) in [
        ("", "sans-serif"),
        ("sans-serif", ""),
        ("Arial,", "sans-serif"),
        ("\"Arial", "sans-serif"),
        ("Arial\0", "serif"),
    ] {
        assert!(
            Font::family(family, fallback).is_err(),
            "{family:?}/{fallback:?}"
        );
    }
    let forged = Font::Family(vec!["broken\0font".into()].into());
    let error = sanim_data::font::shape(&forged, "hello", 24.into()).unwrap_err();
    assert!(error.contains("control characters"), "{error}");
}

#[test]
fn css_family_lists_preserve_order_and_normalize_quotes_and_duplicates() {
    let font = Font::family(" 'Google Sans', Arial, sans-serif ", "SANS-SERIF").unwrap();
    assert_eq!(
        font,
        Font::Family(vec!["Google Sans".into(), "Arial".into(), "sans-serif".into()].into())
    );
}

#[test]
#[ignore = "measured by make font-bench"]
fn native_family_shaping_benchmark() {
    let font = Font::family("Sanim Missing Preferred", "sans-serif").unwrap();
    let start = std::time::Instant::now();
    let mut segments = 0;
    const LABELS: usize = 100;
    for index in 0..LABELS {
        segments += sanim_data::font::shape(&font, &format!("Stack & Queue {index}"), 24.into())
            .unwrap()
            .1
            .len();
    }
    eprintln!(
        "family labels={LABELS} elapsed_us={} segments={segments}",
        start.elapsed().as_micros()
    );
    assert!(segments > 0);
}
