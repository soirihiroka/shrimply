use sanim_core::Fraction;
const FONT: &[u8] = include_bytes!("../../../tests/fixtures/DejaVuSansCondensed-subset.ttf");
#[test]
fn pango_shaping_bounds_cover_outlines_and_missing_glyphs_fail() {
    let (bounds, path) = sanim_data::font::shape(FONT, "AV ffi السلام", Fraction::from(24)).unwrap();
    assert!(!path.is_empty());
    assert!(bounds.max.values[0] > bounds.min.values[0]);
    let error = sanim_data::font::shape(FONT, "中", Fraction::from(24)).unwrap_err();
    assert!(error.contains("missing glyph"), "{error}");
    assert!(sanim_data::font::validate(b"broken font").is_err());
}
