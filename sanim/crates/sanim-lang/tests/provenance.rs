use sanim_core::Value;
use sanim_lang::{Engine, MemorySources, Prepared};
use std::{collections::BTreeMap, sync::Arc};

fn prepare(engine: &mut Engine, sources: &MemorySources) -> Prepared {
    engine
        .prepare("main.sanim", sources, &BTreeMap::new())
        .unwrap()
}

#[test]
fn constructor_locations_preserve_children_and_helper_call_chains() {
    let mut files = MemorySources::default();
    files.insert(
        "parts.sanim",
        "fn shape():Element {\n    return <Rect width={40pt}><Label>hello</Label></Rect>\n}\n",
    );
    files.insert("main.sanim", "import \"parts.sanim\" as parts\nfn wrap():Element {\n    return parts.shape()\n}\nlet shape=wrap()\nlet group=Box(children=[shape])\nadd(group)\nwait(1s)\n");
    let prepared = prepare(&mut Engine::default(), &files);
    let Value::Element(shape) = &prepared.bindings["shape"] else {
        panic!("shape")
    };
    let origin = &prepared.element_sources[&shape.id];
    assert_eq!(origin.creation.source.as_ref(), "parts.sanim");
    assert_eq!(origin.creation.span.line, 2);
    assert_eq!(origin.creation.span.column, 12);
    assert_eq!(origin.call_sites.len(), 2);
    assert_eq!(origin.call_sites[0].source.as_ref(), "main.sanim");
    assert_eq!(origin.call_sites[0].span.line, 5);
    assert_eq!(origin.call_sites[1].span.line, 3);
    assert_eq!(
        prepared.source_line(&origin.creation),
        Some("    return <Rect width={40pt}><Label>hello</Label></Rect>")
    );
    let child = &shape.geometry.children().unwrap()[0];
    let child_origin = &prepared.element_sources[&child.id];
    assert_eq!(child_origin.creation.span.column, 31);
    assert_eq!(child_origin.call_sites, origin.call_sites);
    let group_origin = &prepared.element_sources["group"];
    assert_eq!(group_origin.creation.source.as_ref(), "main.sanim");
    assert_eq!(group_origin.creation.span.line, 6);
    assert!(group_origin.call_sites.is_empty());
}

#[test]
fn native_generated_descendants_share_constructor_evidence() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let axis=Axis(scale=LinearScale(domain=Vec2(0,1),range=Vec2(0pt,200pt)),ticks=[0,0.5,1])\nadd(axis)\nwait(1s)\n");
    let prepared = prepare(&mut Engine::default(), &files);
    let axis = prepared.animation.find("axis").unwrap();
    let origin = &prepared.element_sources["axis"];
    let mut children: Vec<_> = axis.geometry.children().unwrap().iter().collect();
    assert!(!children.is_empty());
    while let Some(child) = children.pop() {
        assert!(Arc::ptr_eq(origin, &prepared.element_sources[&child.id]));
        if let Some(nested) = child.geometry.children() {
            children.extend(nested);
        }
    }
}

#[test]
fn checkpoint_reuse_keeps_shared_origins_but_source_lines_are_revision_bound() {
    let mut files = MemorySources::default();
    let prefix = "let dot_shape=Circle(id=\"dot\") # original\nadd(dot_shape)\n";
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    let mut engine = Engine::default();
    let original = prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        "let dot_shape=Circle(id=\"dot\") # updated!\nadd(dot_shape)\nwait(2s)\n",
    );
    let current = prepare(&mut engine, &files);
    assert_eq!(current.statistics.restored_statements, 2);
    let origin = &current.element_sources["dot"];
    assert!(Arc::ptr_eq(origin, &original.element_sources["dot"]));
    assert_eq!(
        current.source_line(&origin.creation),
        Some("let dot_shape=Circle(id=\"dot\") # updated!")
    );
    assert_eq!(
        original.source_line(&origin.creation),
        Some("let dot_shape=Circle(id=\"dot\") # original")
    );
    let fresh = prepare(&mut Engine::default(), &files);
    assert_eq!(current.element_sources, fresh.element_sources);
    assert_eq!(current.animation, fresh.animation);
}

#[test]
fn provenance_does_not_participate_in_visual_identity() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let dot_shape=Circle(id=\"dot\")\nadd(dot_shape)\nwait(1s)\n",
    );
    let before = prepare(&mut Engine::default(), &files);
    files.insert("main.sanim", "# documentation moved the constructor\n\nlet dot_shape=Circle(id=\"dot\")\nadd(dot_shape)\nwait(1s)\n");
    let after = prepare(&mut Engine::default(), &files);
    assert_eq!(before.animation, after.animation);
    assert_eq!(before.element_sources["dot"].creation.span.line, 1);
    assert_eq!(after.element_sources["dot"].creation.span.line, 3);
}

#[test]
fn cached_constructor_values_restore_origins_after_viewport_changes() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let marker=Box(children=[Circle(id=\"child\")])\nadd(marker)\nwait(1s)\n",
    );
    let mut engine = Engine::default();
    let original = prepare(&mut engine, &files);
    engine.viewport.width = 100;
    let resized = prepare(&mut engine, &files);
    assert!(resized.statistics.reused_expressions > 0);
    assert_eq!(resized.statistics.restored_statements, 0);
    assert_eq!(original.element_sources, resized.element_sources);
    assert!(resized.element_sources.contains_key("marker"));
    assert!(resized.element_sources.contains_key("child"));
}

#[test]
fn cached_helper_constructor_rebinds_call_site_after_source_moves() {
    let mut files = MemorySources::default();
    let helper = "fn make_shape():Element { return Circle(id=\"helper_shape\") }\n";
    files.insert(
        "main.sanim",
        format!("{helper}let shape=make_shape()\nadd(shape)\nwait(1s)\n"),
    );
    let mut engine = Engine::default();
    let original = prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        format!("{helper}# caller moved\nlet shape=make_shape()\nadd(shape)\nwait(1s)\n"),
    );
    let moved = prepare(&mut engine, &files);
    assert!(moved.statistics.reused_expressions > 0);
    assert_eq!(
        original.element_sources["helper_shape"].call_sites[0]
            .span
            .line,
        2
    );
    assert_eq!(
        moved.element_sources["helper_shape"].call_sites[0]
            .span
            .line,
        3
    );
}

#[test]
fn delayed_signal_construction_retains_the_factory_call_site() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "fn make_shape():sig Element {\n    return Circle(id=\"live_shape\")\n}\nlet shape=make_shape()\nadd(shape)\nwait(1s)\n");
    let prepared = prepare(&mut Engine::default(), &files);
    let origin = &prepared.element_sources["live_shape"];
    assert_eq!(origin.creation.span.line, 2);
    assert_eq!(origin.call_sites.len(), 1);
    assert_eq!(origin.call_sites[0].span.line, 4);
}

#[test]
fn imported_escaping_signal_updates_lineage_when_its_caller_moves() {
    let mut files = MemorySources::default();
    files.insert(
        "parts.sanim",
        "fn make_shape():sig Element { return Circle(id=\"signal_shape\") }\n",
    );
    files.insert(
        "main.sanim",
        "import \"parts.sanim\" as parts\nlet shape=parts.make_shape()\nadd(shape)\nwait(1s)\n",
    );
    let mut engine = Engine::default();
    let before = prepare(&mut engine, &files);
    files.insert("main.sanim", "import \"parts.sanim\" as parts\n# a comment moves the caller\nlet shape=parts.make_shape()\nadd(shape)\nwait(1s)\n");
    let after = prepare(&mut engine, &files);
    assert_eq!(
        before.element_sources["signal_shape"].call_sites[0]
            .span
            .line,
        2
    );
    assert_eq!(
        after.element_sources["signal_shape"].call_sites[0]
            .span
            .line,
        3
    );
    assert_eq!(before.animation, after.animation);
}

#[test]
fn original_source_lines_support_unicode_and_all_parser_line_endings() {
    for newline in ["\n", "\r\n", "\r"] {
        let mut files = MemorySources::default();
        files.insert(
            "main.sanim",
            format!("# λ{newline}let shape=Circle(){newline}add(shape){newline}wait(1s){newline}"),
        );
        let prepared = prepare(&mut Engine::default(), &files);
        let origin = &prepared.element_sources["shape"];
        assert_eq!(origin.creation.span.line, 2);
        assert_eq!(
            prepared.source_line(&origin.creation),
            Some("let shape=Circle()")
        );
    }
}

#[test]
fn generated_code_glyphs_keep_their_constructor_owner_after_patch_and_fold() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"let view=<CodeView lang="python">
    def example():
        return 1
</CodeView>
add(view)
view.patch(src="example.py.patch",duration=500ms)
view.fold(level=1,duration=200ms)
"#,
    );
    files.insert(
        "example.py.patch",
        "--- a/example.py\n+++ b/example.py\n@@ -2 +2 @@\n-    return 1\n+    return 2\n",
    );
    let prepared = prepare(&mut Engine::default(), &files);
    let frame = prepared
        .animation
        .sample_with(
            sanim_core::Fraction::new(7u64, 10u64),
            sanim_data::resolve_pending,
        )
        .unwrap();
    let view = frame.iter().find(|element| element.id == "view").unwrap();
    assert!(!view.geometry.children().unwrap().is_empty());
    assert_eq!(prepared.element_sources["view"].creation.span.line, 1);
    assert!(
        view.geometry
            .children()
            .unwrap()
            .iter()
            .any(|child| !prepared.element_sources.contains_key(&child.id))
    );
    // Lazily generated children intentionally inherit this owner in preview inspection.
    assert_eq!(
        prepared.source_line(&prepared.element_sources["view"].creation),
        Some("let view=<CodeView lang=\"python\">")
    );
}

#[test]
#[ignore = "manual performance measurement"]
fn provenance_lookup_and_reload_benchmark() {
    use std::{hint::black_box, time::Instant};
    const ELEMENTS: usize = 1000;
    const LOOKUPS: usize = 100_000;
    const REVISIONS: usize = 10;
    let prefix = format!(
        "for index in range({ELEMENTS}) {{\nlet shape=Circle(id=\"shape-{{index}}\")\nadd(shape)\n}}\n"
    );
    let separate: String = (0..ELEMENTS)
        .map(|index| {
            format!("let shape_{index}=Circle(id=\"shape-{index}\")\nadd(shape_{index})\n")
        })
        .collect();
    for (style, prefix) in [
        ("bounded loop", prefix),
        ("independent declarations", separate),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
        let mut engine = Engine::default();
        let started = Instant::now();
        let original = prepare(&mut engine, &files);
        let cold = started.elapsed();
        let keys: Vec<_> = original.element_sources.keys().collect();
        assert_eq!(keys.len(), ELEMENTS);
        let started = Instant::now();
        for index in 0..LOOKUPS {
            black_box(
                original
                    .element_sources
                    .get(black_box(keys[index % keys.len()])),
            )
            .unwrap();
        }
        let lookups = started.elapsed();
        let mut reloads = Vec::new();
        for revision in 0..REVISIONS {
            files.insert("main.sanim", format!("{prefix}wait({}s)\n", revision + 2));
            let started = Instant::now();
            let changed = prepare(&mut engine, &files);
            reloads.push(started.elapsed());
            assert!(changed.statistics.restored_statements > 0);
            assert!(Arc::ptr_eq(
                &original.element_sources["shape-0"],
                &changed.element_sources["shape-0"]
            ));
        }
        reloads.sort();
        println!(
            "provenance ({style}): {ELEMENTS} elements, checkpoints={} bytes={}, cold={cold:?}, {LOOKUPS} lookups={lookups:?} ({:?}/lookup), suffix reload median={:?}; constructor evidence is shared across revisions",
            original.statistics.checkpoint_count,
            original.statistics.checkpoint_bytes,
            lookups / LOOKUPS as u32,
            reloads[REVISIONS / 2]
        );
    }
}
