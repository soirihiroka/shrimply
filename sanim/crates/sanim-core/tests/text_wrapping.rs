use sanim_core::text::{WhiteSpace, normalize};
#[test]
fn html_whitespace_collapses_only_css_spaces_and_keeps_source_offsets() {
    let source = "  one\t two\r\n  three\u{a0}four  ";
    assert_eq!(
        normalize(source, WhiteSpace::Normal).0,
        "one two three\u{a0}four"
    );
    assert_eq!(
        normalize(source, WhiteSpace::Nowrap).0,
        "one two three\u{a0}four"
    );
    assert_eq!(
        normalize(source, WhiteSpace::PreLine).0,
        "one two\nthree\u{a0}four"
    );
    assert_eq!(
        normalize(source, WhiteSpace::PreWrap).0,
        "  one\t two\n  three\u{a0}four  "
    );
    let (text, mapping) = normalize(source, WhiteSpace::Normal);
    assert_eq!(text.chars().count(), mapping.len());
    for (ch, offset) in text.chars().zip(mapping) {
        let original = source[offset..].chars().next().unwrap();
        assert!(ch == original || ch == ' ' && original.is_ascii_whitespace());
    }
}
