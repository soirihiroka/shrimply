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
        theme: Arc::new(Theme::default()),
        spans: vec![],
    }
}
fn strings(elements: impl IntoIterator<Item = Element>) -> Vec<String> {
    fn collect(element: &Element, output: &mut Vec<String>) {
        if let Geometry::Text { text, .. } = element.geometry.as_ref() {
            output.push(text.clone());
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
