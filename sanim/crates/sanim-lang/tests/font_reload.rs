use sanim_lang::{MemorySources, Reload, Session};

#[test]
fn font_activation_reprepares_unchanged_sources_and_invalidates_state_prefixes() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let typeface=Font()\nlet label=Text(\"Shared fonts\",font=typeface)\nadd(label)\nwait(1s)",
    );
    let mut session = Session::new("main.sanim");
    assert!(matches!(
        session.poll(&sources),
        Reload::Updated { revision: 1, .. }
    ));
    let first = session.current().unwrap().clone();
    assert!(matches!(session.poll(&sources), Reload::Unchanged));
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assets/DejaVuSansCondensed-subset.ttf");
    sanim_data::font::register_application_font(font).unwrap();
    let Reload::Updated {
        revision, prepared, ..
    } = session.poll(&sources)
    else {
        panic!("font activation must publish a new semantic revision");
    };
    assert_eq!(revision, 2);
    assert_ne!(prepared.fingerprint, first.fingerprint);
    assert_eq!(prepared.statistics.restored_statements, 0);
    assert!(matches!(session.poll(&sources), Reload::Unchanged));
}
