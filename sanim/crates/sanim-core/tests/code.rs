use sanim_core::code::{Document, Language, Patch};
#[test]
fn folded_line_extent_preserves_only_code_outside_the_actual_scope_boundary() {
    for (source, end_byte, expected) in [
        ("a\nb\nc", 4, 2),
        ("a\r\nb\r\nc", 6, 2),
        ("a\nb c", 3, 1),
        ("a\nb  \r\nc", 3, 2),
    ] {
        let scope = sanim_core::code::Scope {
            id: "test".into(),
            label: "scope:test".into(),
            start_line: 1,
            end_line: 2,
            start_byte: 0,
            end_byte,
            depth: 1,
        };
        assert_eq!(
            scope.hidden_end_line(source).unwrap(),
            expected,
            "{source:?}"
        );
    }
}
#[test]
fn unified_patch_validates_context_paths_counts_and_terminal_newline() {
    let base = Document::read("sample.py", "x = 1\nkeep = 2\ny = 3\n", Language::Python).unwrap();
    let patch=Document::read("sample.py.patch","--- a/sample.py\n+++ b/sample.py\n@@ -1 +1 @@\n-x = 1\n+x = 4\n@@ -3 +3 @@\n-y = 3\n+y = 5\n",Language::Auto).unwrap();
    assert_eq!(
        base.apply(&patch, None).unwrap().text.as_ref(),
        "x = 4\nkeep = 2\ny = 5\n"
    );
    let wrong = Document::read("other.py", &base.text, Language::Python).unwrap();
    assert!(
        wrong
            .apply(&patch, None)
            .unwrap_err()
            .contains("does not select exactly one")
    );
    let wrong = Document::read("sample.py", "x = 9\nkeep = 2\ny = 3\n", Language::Python).unwrap();
    assert!(wrong.apply(&patch, None).unwrap_err().contains("line 1"));
    assert!(Patch::parse("--- a/a\n+++ b/a\n@@ -1,2 +1 @@\n-old\n+new\n").is_err());
    let patch =
        Patch::parse("--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n")
            .unwrap();
    assert_eq!(patch.files[0].apply("old").unwrap(), "new\n");
    assert!(
        patch.files[0]
            .apply("old\n")
            .unwrap_err()
            .contains("final newline")
    );
    assert!(
        Patch::parse(
            "--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n first\n\\ No newline at end of file\n last\n"
        )
        .is_err()
    );
    assert!(Patch::parse("--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n+extra\n").is_err());
}
#[test]
fn multifile_patches_require_exact_selection_and_crlf_is_preserved() {
    let patch=Patch::parse("--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-old\n+new\n--- a/b.py\n+++ b/b.py\n@@ -1 +1 @@\n-one\n+two\n").unwrap();
    assert!(patch.select(None).unwrap_err().contains("2 files"));
    assert_eq!(
        patch
            .select(Some("a.py"))
            .unwrap()
            .apply("old\r\n")
            .unwrap(),
        "new\r\n"
    );
    assert_eq!(
        patch.select(Some("b.py")).unwrap().apply("one\n").unwrap(),
        "two\n"
    );
    assert!(
        Patch::parse("GIT binary patch\nliteral 3\nxxx\n")
            .unwrap_err()
            .contains("binary")
    );
}
#[test]
fn nested_source_highlights_preserve_interpolations_and_explicit_overrides() {
    use sanim_core::code::{Highlight, normalize_highlights};
    let spans = vec![
        Highlight {
            start: 0,
            end: 7,
            kind: "string".into(),
        },
        Highlight {
            start: 2,
            end: 4,
            kind: "variable".into(),
        },
        Highlight {
            start: 2,
            end: 4,
            kind: "type".into(),
        },
    ];
    let result = normalize_highlights(&spans, "\"{ab}!\"");
    assert_eq!(
        result
            .iter()
            .map(|span| (span.start, span.end, span.kind.as_ref()))
            .collect::<Vec<_>>(),
        vec![(0, 2, "string"), (2, 4, "type"), (4, 7, "string")]
    );
}

fn presentation_state() -> sanim_core::code::view::State {
    use sanim_core::code::view::{Options, State, Theme};
    State {
        document: std::sync::Arc::new(
            Document::read("source.txt", "initial", Language::Plain).unwrap(),
        ),
        options: std::sync::Arc::new(Options {
            font: sanim_core::Font::family("monospace", "monospace").unwrap(),
            font_size: 18.into(),
            line_height: 24.into(),
            auto_line_height: false,
            tab_size: 4,
            line_numbers: false,
            scope_indicators: false,
            theme: std::sync::Arc::new(Theme::default()),
            spans: vec![],
        }),
        folded: Default::default(),
        selection: std::sync::Arc::from([]),
        end: 0.into(),
        events: Default::default(),
    }
}

#[test]
fn code_presentation_seeks_handle_boundaries_instant_changes_and_cow_history() {
    use sanim_core::{Easing, Fraction, code::view::PresentationEvent};
    use std::sync::Arc;
    let mut state = presentation_state();
    let initial = state.snapshot();
    state.document = Arc::new(Document::read("source.txt", "updated", Language::Plain).unwrap());
    let updated = state.snapshot();
    state.events.insert(
        1.into(),
        PresentationEvent {
            start: 1.into(),
            end: 2.into(),
            easing: Easing::Linear,
            animation: Default::default(),
            from: initial.clone(),
            to: updated.clone(),
        },
    );
    let earlier = state.clone();
    state.document = Arc::new(Document::read("source.txt", "instant", Language::Plain).unwrap());
    let instant = state.snapshot();
    state.events.insert(
        3.into(),
        PresentationEvent {
            start: 3.into(),
            end: 3.into(),
            easing: Easing::Linear,
            animation: Default::default(),
            from: updated.clone(),
            to: instant.clone(),
        },
    );
    assert_eq!(earlier.events.len(), 1);
    for (time, expected, progress) in [
        (4.into(), &instant, 1.into()),
        (
            Fraction::new(3u64, 2u64),
            &updated,
            Fraction::new(1u64, 2u64),
        ),
        (3.into(), &instant, 1.into()),
        (2.into(), &updated, 1.into()),
        (1.into(), &updated, 0.into()),
    ] {
        let presentation = state.presentation(time).unwrap().unwrap();
        assert!(Arc::ptr_eq(&presentation.to, expected));
        assert_eq!(presentation.progress, progress);
        assert_eq!(presentation.appearance, progress);
    }
    assert!(state.presentation(0.into()).unwrap().is_none());
    assert!(Arc::ptr_eq(
        &state.presentation(1.into()).unwrap().unwrap().from,
        &initial
    ));
    assert_eq!(
        earlier
            .presentation(4.into())
            .unwrap()
            .unwrap()
            .to
            .document
            .text
            .as_ref(),
        "updated"
    );
}

#[test]
fn code_presentation_spring_opacity_is_bounded_while_motion_can_overshoot() {
    use sanim_core::{Easing, Fraction, code::view::PresentationEvent};
    let mut state = presentation_state();
    let snapshot = state.snapshot();
    state.events.insert(
        0.into(),
        PresentationEvent {
            start: 0.into(),
            end: 1.into(),
            easing: Easing::SpringBouncy,
            animation: Default::default(),
            from: snapshot.clone(),
            to: snapshot,
        },
    );
    let mut overshot = false;
    for index in 0..=20u64 {
        let presentation = state
            .presentation(Fraction::new(index, 20u64))
            .unwrap()
            .unwrap();
        assert!(presentation.appearance >= 0.into() && presentation.appearance <= 1.into());
        overshot |= presentation.progress > 1.into();
    }
    assert!(overshot);
    assert_eq!(
        state.presentation(1.into()).unwrap().unwrap().appearance,
        1.into()
    );
}

#[test]
fn unresolved_code_bounds_fail_closed_and_native_evidence_uses_the_new_source() {
    use sanim_core::{
        Animation, Color, Element, Fraction, Geometry, Layout, Number, Property, Unit, Value, Vec2,
        verify::{self, Evidence, Predicate, Relation},
    };
    use std::sync::Arc;
    fn rectangle(width: Fraction) -> Element {
        Element {
            id: "glyph".into(),
            geometry: Arc::new(Geometry::Rectangle {
                width,
                height: 10.into(),
                children: Default::default(),
                radius: Default::default(),
            }),
            position: Vec2::zero(Unit::Length),
            rotation: 0.into(),
            scale: 1.into(),
            opacity: 1.into(),
            fill: Color::parse("blue3").unwrap().into(),
            stroke: None,
            reveal: Default::default(),
            horizontal: Default::default(),
            layout: Layout::default(),
        }
    }
    let mut code = rectangle(2.into());
    code.id = "code".into();
    let options = presentation_state().options;
    code.geometry = Arc::new(Geometry::Code {
        document: Arc::new(Document::read("source.txt", "2", Language::Plain).unwrap()),
        options,
        children: vec![rectangle(2.into())].into_iter().collect(),
        pending: false,
        source_changed: false,
        presentation: None,
    });
    code.layout.width = Some(Number {
        value: 2.into(),
        unit: Unit::Length,
    });
    code.layout.height = Some(Number {
        value: 10.into(),
        unit: Unit::Length,
    });
    code.set(Property::Code, Value::String("200".into()))
        .unwrap();
    assert!(
        verify::bounds(&code)
            .unwrap_err()
            .contains("code layout is unresolved")
    );
    let mut animation = Animation::default();
    animation.width = 100;
    animation.height = 100;
    animation.dpi = 72;
    animation.add(code.clone(), 0.into()).unwrap();
    let predicate = Predicate {
        left: code.clone(),
        right: None,
        relation: Relation::InsideCanvas,
        gap: 0.into(),
        continuous: false,
    };
    let unresolved = verify::evaluate(&animation, &predicate, 0.into());
    assert_eq!(unresolved.evidence, Evidence::Unknown, "{unresolved:?}");
    let resolved = verify::evaluate_with(&animation, &predicate, 0.into(), |element, _| {
        if let Geometry::Code {
            document,
            children,
            pending,
            ..
        } = Arc::make_mut(&mut element.geometry)
        {
            let width = Fraction::from(
                document
                    .text
                    .parse::<u64>()
                    .map_err(|error| error.to_string())?,
            );
            *children = vec![rectangle(width)].into_iter().collect();
            *pending = false;
            element.layout.width = Some(Number {
                value: width,
                unit: Unit::Length,
            });
        }
        Ok(())
    });
    assert_eq!(
        resolved.evidence,
        Evidence::Refuted,
        "native evidence must see the changed 200pt source, not stale 2pt children: {resolved:?}"
    );
    // A fixed clipped viewport has content-independent bounds, even while its
    // contents still require native shaping.
    code.layout
        .configure_overflow(
            sanim_core::Overflow::Hidden,
            sanim_core::Overflow::Hidden,
            Some(Vec2::zero(Unit::Length)),
        )
        .unwrap();
    code.layout.width = Some(Number {
        value: 20.into(),
        unit: Unit::Length,
    });
    let viewport = verify::bounds(&code).unwrap();
    assert_eq!(viewport.max.values[0] - viewport.min.values[0], 20.);
}

#[test]
fn code_presentation_snapshots_share_unchanged_selection_buffers() {
    use std::sync::Arc;
    let mut state = presentation_state();
    state.selection = vec![sanim_core::code::view::Range {
        start_line: 1,
        start_column: 1,
        end_line: 1,
        end_column: 4,
        origin: None,
    }]
    .into();
    let before = state.snapshot();
    state.folded.insert("function:example".into());
    let after = state.snapshot();
    assert!(Arc::ptr_eq(&state.selection, &before.selection));
    assert!(Arc::ptr_eq(&before.selection, &after.selection));
    state.selection = Arc::from([]);
    assert_eq!(
        before.selection.len(),
        1,
        "later edits must not mutate retained snapshots"
    );
}
