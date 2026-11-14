use sanim_core::{
    Element, Font, Fraction, Geometry,
    code::{
        Document, Language,
        view::{Options, PatchAnimation, Theme},
    },
};
use sanim_data::code::view::{prepare, transition_tokens};
use std::sync::Arc;

fn tokens(source: &str) -> imbl::Vector<sanim_core::code::view::Token> {
    let document = Document::read("code.txt", source, Language::Plain).unwrap();
    prepare("code", &document, &options(), &Default::default(), &[])
        .unwrap()
        .tokens
}
fn options() -> Options {
    Options {
        font: Font::family("sans-serif", "sans-serif").unwrap(),
        font_size: 20.into(),
        line_height: 30.into(),
        auto_line_height: true,
        tab_size: 4,
        line_numbers: false,
        scope_indicators: false,
        theme: Arc::new(Theme::default()),
        spans: vec![],
    }
}
fn strings(elements: impl IntoIterator<Item = Element>) -> Vec<String> {
    fn collect(element: &Element, output: &mut Vec<String>) {
        if let Geometry::Text { text, .. } = element.geometry.as_ref() {
            output.push(text.to_string());
        }
        if let Some(children) = element.geometry.children() {
            for child in children {
                collect(child, output);
            }
        }
    }
    let mut output = vec![];
    for element in elements {
        collect(&element, &mut output);
    }
    output
}

#[test]
fn fade_crossfades_fixed_endpoints_while_morph_moves_shared_code() {
    let before = tokens("alpha");
    let after = tokens("\nalpha");
    let half = Fraction::new(1u64, 2u64);
    let morph =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    assert_eq!(morph.len(), 1);
    assert!(morph[0].position.values[1] > before[0].position.values[1]);
    assert!(morph[0].position.values[1] < after[0].position.values[1]);
    let fade = transition_tokens(&before, &after, half, half, half, PatchAnimation::Fade).unwrap();
    assert_eq!(fade.len(), 2);
    assert!(
        fade.iter()
            .any(|element| element.position == before[0].position)
    );
    assert!(
        fade.iter()
            .any(|element| element.position == after[0].position)
    );
    assert!(fade.iter().all(|element| element.opacity == half));
    assert_ne!(fade[0].id, fade[1].id);
}

#[test]
fn typewriter_reveals_and_deletes_unicode_cursor_clusters_instead_of_fading_words() {
    let empty = tokens("");
    let complete = tokens("e\u{301}xyz");
    let quarter = Fraction::new(1u64, 4u64);
    let typed = transition_tokens(
        &empty,
        &complete,
        quarter,
        quarter,
        quarter,
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert_eq!(strings(typed.clone()), ["e\u{301}"]);
    assert_eq!(typed[0].opacity, 1.into());
    let erased = transition_tokens(
        &complete,
        &empty,
        quarter,
        quarter,
        quarter,
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert_eq!(strings(erased), ["e\u{301}xy"]);
    let full = transition_tokens(
        &empty,
        &complete,
        1.into(),
        1.into(),
        1.into(),
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert_eq!(strings(full.clone()), ["e\u{301}xyz"]);
    assert!(Arc::ptr_eq(
        &full[0].geometry,
        &complete[0].element.geometry
    ));
}

#[test]
fn typewriter_completes_deletions_before_typing_replacement_and_preserves_native_ligatures() {
    let before = tokens("removed");
    let after = tokens("office");
    // Seven old clusters are removed first; six new clusters follow.
    let midpoint = Fraction::new(7u64, 13u64);
    let deleted = transition_tokens(
        &before,
        &after,
        midpoint,
        midpoint,
        midpoint,
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert!(deleted.is_empty());
    let next = Fraction::new(8u64, 13u64);
    let partial = transition_tokens(
        &before,
        &after,
        next,
        next,
        next,
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert_eq!(strings(partial), ["o"]);
    let final_frame = transition_tokens(
        &before,
        &after,
        1.into(),
        1.into(),
        1.into(),
        PatchAnimation::Typewriter,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &final_frame[0].geometry,
        &after[0].element.geometry
    ));
}

#[test]
#[ignore = "manual performance measurement"]
fn transition_benchmark() {
    use std::{hint::black_box, time::Instant};
    let before = tokens(&(0..100).map(|_| "value = before\n").collect::<String>());
    let after = tokens(&(0..100).map(|_| "value = after\n").collect::<String>());
    for animation in [
        PatchAnimation::Morph,
        PatchAnimation::Fade,
        PatchAnimation::Typewriter,
    ] {
        let half = Fraction::new(1u64, 2u64);
        let started = Instant::now();
        black_box(transition_tokens(&before, &after, half, half, half, animation).unwrap());
        let first = started.elapsed();
        let started = Instant::now();
        for index in 1..60u64 {
            let progress = Fraction::new(index, 60u64);
            black_box(
                transition_tokens(&before, &after, progress, progress, progress, animation)
                    .unwrap(),
            );
        }
        println!(
            "{animation:?}: {} tokens, first={first:?}, 59 cached-plan samples={:?}",
            after.len(),
            started.elapsed()
        );
    }
}

#[test]
fn removed_repeated_tokens_do_not_steal_surviving_later_line_identity() {
    let before = tokens("alpha\nbeta\nalpha");
    let after = tokens("beta\nalpha");
    let half = Fraction::new(1u64, 2u64);
    let middle =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    let surviving_alpha = middle
        .iter()
        .find(|element| element.id == after[1].element.id)
        .unwrap();
    assert_eq!(
        surviving_alpha.position.values[1],
        (before[2].position.values[1] + after[1].position.values[1]) / 2.,
        "an ordered unchanged beta/alpha suffix must keep its own source identity; the removed leading alpha must fade instead of morphing into the surviving suffix"
    );
}

#[test]
fn unique_moved_tokens_keep_morphing_across_the_ordered_edit_path() {
    let before = tokens("alpha\nbeta");
    let after = tokens("beta\nalpha");
    let half = Fraction::new(1u64, 2u64);
    let middle =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    assert_eq!(middle.len(), 2);
    for (old, new) in [(0, 1), (1, 0)] {
        let moved = middle
            .iter()
            .find(|element| element.id == after[new].element.id)
            .unwrap();
        assert_eq!(moved.opacity, 1.into());
        assert_eq!(
            moved.position.values[1],
            (before[old].position.values[1] + after[new].position.values[1]) / 2.
        );
    }
}

#[test]
fn folded_repeated_words_follow_original_source_coordinates() {
    let document = sanim_data::code::parse(Document::source_shared("code.py",
        "def first(value):\n    result = value\n    return result\n\ndef second(value):\n    result = value\n    return result\n".into(), Language::Python).unwrap()).unwrap();
    let folded = document
        .scopes
        .iter()
        .find(|scope| scope.start_line == 1)
        .unwrap();
    let before = prepare("code", &document, &options(), &Default::default(), &[])
        .unwrap()
        .tokens;
    let after = prepare(
        "code",
        &document,
        &options(),
        &imbl::ordset![folded.id.clone()],
        &[],
    )
    .unwrap()
    .tokens;
    let half = Fraction::new(1u64, 2u64);
    let middle =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    let mut surviving = 0;
    for token in after.iter().filter(|token| token.line >= 5) {
        let old = before
            .iter()
            .find(|old| {
                old.line == token.line && old.column == token.column && old.key == token.key
            })
            .unwrap();
        let moved = middle
            .iter()
            .find(|element| element.id == token.element.id)
            .unwrap();
        assert_eq!(
            moved.opacity,
            1.into(),
            "folding keeps surviving source fully opaque"
        );
        assert_eq!(
            moved.position.values[1],
            (old.position.values[1] + token.position.values[1]) / 2.
        );
        surviving += 1;
    }
    assert!(surviving > 5);
}

#[test]
#[ignore = "manual performance measurement"]
fn transition_benchmark_repetitive_functions_patch_and_fold() {
    use std::{hint::black_box, time::Instant};
    let source: String = (0..300)
        .map(|index| {
            format!(
                "def function_{index}(value):\n    result = value + {index}\n    return result\n\n"
            )
        })
        .collect();
    let document = sanim_data::code::parse(
        Document::source_shared("functions.py", source.clone().into(), Language::Python).unwrap(),
    )
    .unwrap();
    let changed = source
        .replace("value + 150", "value * 150")
        .replace("value + 200", "value - 200");
    let patched = sanim_data::code::parse(
        Document::source_shared("functions.py", changed.into(), Language::Python).unwrap(),
    )
    .unwrap();
    let folded = document
        .scopes
        .iter()
        .filter(|scope| scope.start_line == 1)
        .map(|scope| scope.id.clone())
        .collect();
    let options = options();
    let from = prepare("code", &document, &options, &Default::default(), &[])
        .unwrap()
        .tokens;
    let patch = prepare("code", &patched, &options, &Default::default(), &[])
        .unwrap()
        .tokens;
    let fold = prepare("code", &document, &options, &folded, &[])
        .unwrap()
        .tokens;
    for (kind, to) in [("patch", &patch), ("fold", &fold)] {
        let half = Fraction::new(1u64, 2u64);
        let started = Instant::now();
        black_box(transition_tokens(&from, to, half, half, half, PatchAnimation::Morph).unwrap());
        let first = started.elapsed();
        let started = Instant::now();
        for index in 1..60u64 {
            let progress = Fraction::new(index, 60u64);
            black_box(
                transition_tokens(
                    &from,
                    to,
                    progress,
                    progress,
                    progress,
                    PatchAnimation::Morph,
                )
                .unwrap(),
            );
        }
        println!(
            "300 functions {kind}: {} -> {} tokens, first={first:?}, 59 cached-plan samples={:?}",
            from.len(),
            to.len(),
            started.elapsed()
        );
    }
}

#[test]
fn edited_identifiers_keep_shared_grapheme_fragments_opaque_and_move_suffixes() {
    let before = tokens("prefix_old_suffix");
    let after = tokens("prefix_longer_suffix");
    let quarter = Fraction::new(1u64, 4u64);
    let half = Fraction::new(1u64, 2u64);
    let first = transition_tokens(
        &before,
        &after,
        quarter,
        quarter,
        quarter,
        PatchAnimation::Morph,
    )
    .unwrap();
    let middle =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    for frame in [&first, &middle] {
        let stable: Vec<_> = frame.iter().filter(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "prefix_" || text.as_ref() == "_suffix")).collect();
        assert_eq!(stable.len(), 2, "only the replaced middle may fade");
        assert!(stable.iter().all(|element| element.opacity == 1.into()));
    }
    let suffix = |frame: &imbl::Vector<Element>| {
        frame.iter().find(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "_suffix")).unwrap().position.values[0]
    };
    assert!(
        suffix(&middle) > suffix(&first),
        "unchanged suffix follows the expanding replacement width"
    );
    let added = middle.iter().find(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "longer")).unwrap();
    let removed = middle.iter().find(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "old")).unwrap();
    assert_eq!(
        added.opacity,
        0.into(),
        "the new span waits for layout space"
    );
    assert_eq!(
        removed.opacity,
        0.into(),
        "the old span leaves before replacement"
    );
    for (progress, endpoint) in [(0.into(), &before), (1.into(), &after)] {
        let frame = transition_tokens(
            &before,
            &after,
            progress,
            progress,
            progress,
            PatchAnimation::Morph,
        )
        .unwrap();
        assert_eq!(frame.len(), 1);
        assert!(
            Arc::ptr_eq(&frame[0].geometry, &endpoint[0].element.geometry),
            "exact endpoints preserve the complete native ligature run"
        );
    }
    // A cached plan shares its fragment geometry across samples; shaping and
    // text splitting are never repeated in the frame loop.
    let stable_first = first.iter().find(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "_suffix")).unwrap();
    let stable_middle = middle.iter().find(|element| matches!(element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "_suffix")).unwrap();
    assert!(Arc::ptr_eq(&stable_first.geometry, &stable_middle.geometry));
}

#[test]
fn identifier_edits_do_not_split_combining_cursor_clusters() {
    let before = tokens("e\u{301}oldn\u{303}");
    let after = tokens("e\u{301}newn\u{303}");
    let half = Fraction::new(1u64, 2u64);
    let frame =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    let stable: Vec<_> = frame
        .iter()
        .filter(|element| element.opacity == 1.into())
        .cloned()
        .collect();
    assert_eq!(strings(stable), ["e\u{301}", "n\u{303}"]);
}

#[test]
fn stable_source_recolors_without_crossfading_when_highlight_style_changes() {
    use sanim_core::{
        Color, Paint,
        code::view::{Override, Range, Style},
    };
    let document = Document::read("code.txt", "unchanged", Language::Plain).unwrap();
    let before = prepare("code", &document, &options(), &Default::default(), &[])
        .unwrap()
        .tokens;
    let mut changed = options();
    changed.spans.push(Override {
        range: Range {
            origin: None,
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 10,
        },
        style: Style {
            fill: Some(Color::parse("blue3").unwrap()),
            ..Default::default()
        },
    });
    let after = prepare("code", &document, &changed, &Default::default(), &[])
        .unwrap()
        .tokens;
    let half = Fraction::new(1u64, 2u64);
    let frame =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].opacity, 1.into());
    let (Paint::Solid(a), Paint::Solid(b), Paint::Solid(actual)) = (
        &before[0].element.fill,
        &after[0].element.fill,
        &frame[0].fill,
    ) else {
        panic!("native glyph uses solid paint")
    };
    assert_ne!(actual, a);
    assert_ne!(actual, b);
    assert_eq!(
        actual,
        &sanim_core::math::paint::color_lerp(a, b, half).unwrap()
    );
}

#[test]
fn decorated_fragment_edits_preserve_unique_element_ids() {
    use sanim_core::{
        Color,
        code::view::{Override, Range, Style},
    };
    let mut styled = options();
    styled.spans.push(Override {
        range: Range {
            origin: None,
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 18,
        },
        style: Style {
            background: Some(Color::parse("blue5").unwrap()),
            underline: Some(true),
            ..Default::default()
        },
    });
    let prepare_styled = |source| {
        prepare(
            "code",
            &Document::read("code.txt", source, Language::Plain).unwrap(),
            &styled,
            &Default::default(),
            &[],
        )
        .unwrap()
        .tokens
    };
    let before = prepare_styled("prefix_old_suffix");
    let after = prepare_styled("prefix_new_suffix");
    let half = Fraction::new(1u64, 2u64);
    let frame =
        transition_tokens(&before, &after, half, half, half, PatchAnimation::Morph).unwrap();
    fn visit(element: &Element, ids: &mut std::collections::HashSet<String>) {
        assert!(
            ids.insert(element.id.clone()),
            "duplicate fragment element id: {}",
            element.id
        );
        if let Some(children) = element.geometry.children() {
            for child in children {
                visit(child, ids);
            }
        }
    }
    let mut ids = Default::default();
    for element in frame {
        visit(&element, &mut ids);
    }
}

#[test]
fn inserted_words_wait_for_the_suffix_to_open_space_without_fading_stable_code() {
    let before = tokens("Running job");
    let after = tokens("Running queued job");
    for progress in [
        Fraction::new(1u64, 4u64),
        Fraction::new(1u64, 2u64),
        Fraction::new(3u64, 4u64),
    ] {
        let frame = transition_tokens(
            &before,
            &after,
            progress,
            progress,
            progress,
            PatchAnimation::Morph,
        )
        .unwrap();
        for element in &frame {
            if let Geometry::Text { text, .. } = element.geometry.as_ref() {
                if text.as_ref() == "queued" {
                    assert_eq!(
                        element.opacity,
                        if progress <= Fraction::new(1u64, 2u64) {
                            Fraction::from(0)
                        } else {
                            Fraction::new(1u64, 2u64)
                        }
                    );
                } else {
                    assert_eq!(
                        element.opacity,
                        1.into(),
                        "unchanged word {text} must stay readable"
                    );
                }
            }
        }
    }
}
