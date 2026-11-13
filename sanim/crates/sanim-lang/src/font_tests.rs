use super::*;

#[test]
fn changing_native_font_epoch_invalidates_retained_text_prefixes() {
    for source in [
        "let typeface=Font()\nlet label=Text(\"Epoch test\",font=typeface)\nadd(label)\nwait(1s)",
        "let label=Text(\"Epoch test\")\nadd(label)\nwait(1s)",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let mut engine = Engine::default();
        let first = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let warm = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert!(warm.statistics.restored_statements > 0);
        let actual = engine.font_environment.expect("native font epoch");
        engine.font_environment = Some(actual.wrapping_add(1));
        let refreshed = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(refreshed.statistics.restored_statements, 0);
        assert_eq!(first.animation, refreshed.animation);
        assert_eq!(engine.font_environment, Some(actual));
    }
}
