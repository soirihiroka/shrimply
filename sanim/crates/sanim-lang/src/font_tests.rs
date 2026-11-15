use super::*;

#[test]
fn changing_native_font_epoch_invalidates_retained_text_prefixes() {
    for source in [
        "let typeface=Font()\nlet label=Label(\"Epoch test\",font=typeface)\nAnimation(children=[label])\nwait(1s)",
        "let label=Label(\"Epoch test\")\nAnimation(children=[label])\nwait(1s)",
        "fn caption():Element{return Label(\"Epoch test\")}\nlet label=caption()\nAnimation(children=[label])\nwait(1s)",
        "fn snippet():Element{return CodeView(code=r\"print(1)\",lang=\"python\")}\nlet label=snippet()\nAnimation(children=[label])\nwait(1s)",
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
        engine.font_environment = Some(sanim_data::font::FontEpoch {
            revision: actual.revision.wrapping_add(1),
            ..actual
        });
        let refreshed = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(refreshed.statistics.restored_statements, 0);
        assert_eq!(first.animation, refreshed.animation);
        assert_eq!(engine.font_environment, Some(actual));
    }
}
