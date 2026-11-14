use sanim_core::code::{
    Document, Language, Scope, values,
    view::{Range, RangeOrigin, resolve_ranges, source_revision},
};
use std::sync::Arc;

fn range(start_line: usize, start_column: usize, end_line: usize, end_column: usize) -> Range {
    Range {
        start_line,
        start_column,
        end_line,
        end_column,
        origin: None,
    }
}
fn document() -> Document {
    let mut document = Document::read("sample.rs", "fn task() {}", Language::Rust).unwrap();
    document.scopes = vec![Scope {
        id: "function:task".into(),
        label: "task".into(),
        start_line: 1,
        end_line: 1,
        start_byte: 0,
        end_byte: document.text.len(),
        depth: 0,
    }]
    .into();
    document.syntax_parsed = true;
    document
}
#[test]
fn unicode_columns_crlf_eol_and_eof_sentinels_share_one_byte_mapping() {
    let source = "α😊\r\nrust\n";
    assert_eq!(&source[range(1, 2, 1, 3).byte_range(source).unwrap()], "😊");
    assert_eq!(
        &source[range(1, 1, 1, usize::MAX).byte_range(source).unwrap()],
        "α😊"
    );
    assert_eq!(
        &source[range(1, 1, 2, 1).byte_range(source).unwrap()],
        "α😊\r\n"
    );
    assert_eq!(
        &source[range(2, 1, 3, 1).byte_range(source).unwrap()],
        "rust\n"
    );
    assert_eq!(range(1, 1, 2, 1).byte_range("last").unwrap(), 0..4);
    assert_eq!(range(1, 1, 1, 1).byte_range("").unwrap(), 0..0);
    for invalid in [
        range(0, 1, 1, 1),
        range(1, 0, 1, 1),
        range(1, 1, 1, 4),
        range(1, 1, 5, 1),
    ] {
        assert!(invalid.byte_range(source).is_err(), "{invalid:?}");
    }
}
#[test]
fn canonical_values_keep_provenance_cow_and_unsigned_eol_columns() {
    let document = document();
    let mut selection = range(1, 1, 1, usize::MAX);
    selection.origin = Some(Arc::new(RangeOrigin {
        owner: "code".into(),
        revision: source_revision(&document),
        scope_id: None,
    }));
    let value = values::range_value(selection.clone());
    sanim_core::validate_value(&values::range_type(), &value).unwrap();
    let decoded = values::range(&value).unwrap();
    assert_eq!(decoded, selection);
    assert!(Arc::ptr_eq(
        &decoded.origin.as_ref().unwrap().revision,
        &selection.origin.as_ref().unwrap().revision
    ));
    let args = [
        ("start_line".to_owned(), values::scalar(1)),
        ("start_column".to_owned(), values::scalar(1)),
        ("end_line".to_owned(), values::scalar(1)),
        ("end_column".to_owned(), values::scalar(2)),
    ]
    .into_iter()
    .collect();
    let manual = values::Native::CodeRange.execute(&args).unwrap();
    sanim_core::validate_value(&values::range_type(), &manual).unwrap();
    assert!(values::range(&manual).unwrap().origin.is_none());
}
#[test]
fn owner_revision_and_scope_checks_fail_before_a_range_can_target_another_source() {
    let document = document();
    let mut selection = range(1, 1, 2, 1);
    selection.origin = Some(Arc::new(RangeOrigin {
        owner: "code".into(),
        revision: source_revision(&document),
        scope_id: Some("function:task".into()),
    }));
    selection.validate_for("code", &document).unwrap();
    let wrong = selection.validate_for("other", &document).unwrap_err();
    assert!(
        wrong.contains("belongs to CodeView") && wrong.contains("code") && wrong.contains("other")
    );
    let mut changed = document.clone();
    changed.text = "fn other() {}".into();
    assert!(
        selection
            .validate_for("code", &changed)
            .unwrap_err()
            .contains("stale")
    );
    let mut language = document.clone();
    language.language = Language::Plain;
    assert!(
        selection
            .validate_for("code", &language)
            .unwrap_err()
            .contains("stale")
    );
    let mut moved = document.clone();
    moved.path = "renamed.rs".into();
    selection.validate_for("code", &moved).unwrap();
    let mut missing = document.clone();
    missing.scopes = Arc::new([]);
    assert!(
        selection
            .validate_for("code", &missing)
            .unwrap_err()
            .contains("no longer exists")
    );
    selection.end_line = 1;
    selection.end_column = 2;
    assert!(
        selection
            .validate_for("code", &document)
            .unwrap_err()
            .contains("no longer match scope")
    );
    let manual = range(1, 1, 1, usize::MAX);
    assert!(manual.validate_for("unrelated-owner", &document).is_ok());
}
#[test]
fn batches_resolve_many_unicode_ranges_on_one_long_line() {
    const MATCHES: usize = 4096;
    let source = "α😊".repeat(MATCHES);
    let document = Document::read("sample.txt", &source, Language::Plain).unwrap();
    let ranges = (0..MATCHES)
        .map(|index| range(1, index * 2 + 2, 1, index * 2 + 3))
        .collect::<Vec<_>>();
    let bytes = resolve_ranges("code", &document, &ranges).unwrap();
    assert_eq!(bytes.len(), MATCHES);
    assert!(bytes.into_iter().all(|span| &source[span] == "😊"));
}

#[test]
fn range_origin_payload_is_charged_once_and_guards_do_not_keep_it_alive() {
    use sanim_core::{
        Color, Element, Geometry, Unit, Value, Vec2,
        code::view::{Options, Override, Style, Theme},
        value_budget::{Budget, charge_with_budget},
    };
    let origin = Arc::new(RangeOrigin {
        owner: "view".repeat(4096).into(),
        revision: "revision".into(),
        scope_id: None,
    });
    let weak = Arc::downgrade(&origin);
    let range = Range {
        origin: Some(origin.clone()),
        ..range(1, 1, 1, 2)
    };
    let options = Arc::new(Options {
        font: Default::default(),
        font_size: 12.into(),
        line_height: 18.into(),
        auto_line_height: true,
        tab_size: 4,
        line_numbers: false,
        scope_indicators: false,
        theme: Arc::new(Theme::default()),
        spans: (0..256)
            .map(|_| Override {
                range: range.clone(),
                style: Style::default(),
            })
            .collect(),
    });
    let value = Value::Element(Arc::new(Element {
        id: "code".into(),
        geometry: Arc::new(Geometry::Code {
            document: Arc::new(document()),
            options,
            children: Default::default(),
            pending: false,
            source_changed: false,
            presentation: None,
        }),
        position: Vec2::zero(Unit::Length),
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: Color::parse("light1").unwrap().into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Default::default(),
    }));
    let mut budget = Budget::default();
    let (mut bytes, mut items) = (1024 * 1024, 10_000);
    charge_with_budget(&value, &mut bytes, &mut items, 16, &mut budget).unwrap();
    assert!(
        bytes > 512 * 1024,
        "repeated references must not charge the16KiB owner256times"
    );
    assert!(budget.metadata_bytes() > 0);
    drop(value);
    drop(range);
    drop(origin);
    assert!(
        weak.upgrade().is_none(),
        "persistent budget guards must not retain range objects"
    );
}

#[test]
#[ignore = "release batch range resolution regression: make code-range-bench"]
fn batched_unicode_range_resolution_is_faster_than_rebuilding_source_indices() {
    use std::{hint::black_box, time::Instant};
    const RANGES: usize = 2048;
    const SAMPLES: usize = 9;
    const MIN_SPEEDUP: u32 = 4;
    let source = "α😊".repeat(RANGES);
    let document = Document::read("sample.txt", &source, Language::Plain).unwrap();
    let ranges = (0..RANGES)
        .map(|index| range(1, index * 2 + 1, 1, index * 2 + 2))
        .collect::<Vec<_>>();
    let mut batch = Vec::new();
    let mut independent = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let resolved = resolve_ranges("code", black_box(&document), black_box(&ranges)).unwrap();
        black_box(resolved);
        batch.push(started.elapsed());
        let started = Instant::now();
        let resolved = ranges
            .iter()
            .map(|range| range.byte_range(black_box(&source)).unwrap())
            .collect::<Vec<_>>();
        black_box(resolved);
        independent.push(started.elapsed());
    }
    batch.sort_unstable();
    independent.sort_unstable();
    let (batch, independent) = (batch[SAMPLES / 2], independent[SAMPLES / 2]);
    println!(
        "Unicode range batch: ranges={RANGES}, samples={SAMPLES}, batch median={batch:?}, independent source-index median={independent:?}"
    );
    assert!(
        batch < independent / MIN_SPEEDUP,
        "batch range validation must index shared source once rather than rescan the source/column prefixes for each range"
    );
}

#[test]
fn exact_scope_provenance_distinguishes_same_line_siblings_and_indented_unicode() {
    let source = "fn one() {} fn two() {}\n";
    let mut document = Document::read("sample.rs", source, Language::Rust).unwrap();
    document.scopes = vec![
        Scope {
            id: "function:one".into(),
            label: "one".into(),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 11,
            depth: 0,
        },
        Scope {
            id: "function:two".into(),
            label: "two".into(),
            start_line: 1,
            end_line: 1,
            start_byte: 12,
            end_byte: 23,
            depth: 0,
        },
    ]
    .into();
    let revision = source_revision(&document);
    let mut one = range(1, 1, 1, 12);
    one.origin = Some(Arc::new(RangeOrigin {
        owner: "code".into(),
        revision: revision.clone(),
        scope_id: Some("function:one".into()),
    }));
    let mut two = range(1, 13, 1, 24);
    two.origin = Some(Arc::new(RangeOrigin {
        owner: "code".into(),
        revision,
        scope_id: Some("function:two".into()),
    }));
    let spans = resolve_ranges("code", &document, &[one.clone(), two]).unwrap();
    assert_eq!(&source[spans[0].clone()], "fn one() {}");
    assert_eq!(&source[spans[1].clone()], "fn two() {}");
    one.end_line = 2;
    one.end_column = 1;
    assert!(
        one.validate_for("code", &document)
            .unwrap_err()
            .contains("exact range"),
        "whole-line coordinates must not silently select a sibling function"
    );
    let source = "    fn α() {}\n";
    let mut document = Document::read("sample.rs", source, Language::Rust).unwrap();
    document.scopes = vec![Scope {
        id: "method:α".into(),
        label: "α".into(),
        start_line: 1,
        end_line: 1,
        start_byte: 4,
        end_byte: source.len() - 1,
        depth: 1,
    }]
    .into();
    let mut method = range(1, 5, 1, source.trim_end_matches('\n').chars().count() + 1);
    method.origin = Some(Arc::new(RangeOrigin {
        owner: "code".into(),
        revision: source_revision(&document),
        scope_id: Some("method:α".into()),
    }));
    let span = resolve_ranges("code", &document, &[method])
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(&source[span], "fn α() {}");
}

#[test]
fn source_index_round_trips_lone_cr_crlf_and_unicode_in_both_index_modes() {
    use sanim_core::code::view::SourceIndex;
    let source = "α\rβ\r\n😊\nend\r";
    let mut index = SourceIndex::new(source);
    assert_eq!(
        index.position(source.find('β').unwrap()).unwrap(),
        (1, 3),
        "a lone CR is not a native syntax-tree row break"
    );
    assert_eq!(index.position(source.find('😊').unwrap()).unwrap(), (2, 1));
    for global in [false, true] {
        if global {
            assert_eq!(index.character(6).unwrap(), source.find('😊').unwrap());
            assert_eq!(
                index.advance(source.find('😊').unwrap(), 1).unwrap(),
                source.find('😊').unwrap() + '😊'.len_utf8()
            );
        }
        for at in source
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(source.len()))
        {
            if source.as_bytes().get(at) == Some(&b'\n')
                && at > 0
                && source.as_bytes()[at - 1] == b'\r'
            {
                assert!(index.position(at).unwrap_err().contains("CRLF"));
                continue;
            }
            let (line, column) = index.position(at).unwrap();
            assert_eq!(
                index.at(line, column).unwrap(),
                at,
                "byte={at}, global={global}"
            );
            let range = index.range(at..at, None).unwrap();
            assert_eq!(index.resolve(&range).unwrap(), at..at);
        }
    }
}
