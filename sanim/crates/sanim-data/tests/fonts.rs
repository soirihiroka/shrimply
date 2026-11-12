use sanim_core::Fraction;
const FONT: &[u8] = include_bytes!("../../../tests/fixtures/DejaVuSansCondensed-subset.ttf");
#[test]
fn pango_shaping_bounds_cover_outlines_and_missing_glyphs_fail() {
    let (bounds, path) = sanim_data::font::shape(FONT, "AV ffi السلام", Fraction::from(24)).unwrap();
    assert!(!path.is_empty());
    assert!(bounds.max.values[0] > bounds.min.values[0]);
    let error = sanim_data::font::shape(FONT, "中", Fraction::from(24)).unwrap_err();
    assert!(error.contains("missing glyph"), "{error}");
    // This subset omits Ω, even if the full DejaVu family is installed locally.
    // Selecting an ambient font by the same family name must not make it appear.
    let error = sanim_data::font::shape(FONT, "Ω", Fraction::from(24)).unwrap_err();
    assert!(error.contains("missing glyph"), "{error}");
    assert!(sanim_data::font::validate(b"broken font").is_err());
}

#[test]
fn explicit_font_assets_ignore_ambient_coretext_backend_selection() {
    const CHILD: &str = "SANIM_EXPLICIT_FONT_BACKEND_TEST";
    if std::env::var_os(CHILD).is_some() {
        let (bounds, path) =
            sanim_data::font::shape(FONT, "AV ffi السلام", Fraction::from(24)).unwrap();
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
