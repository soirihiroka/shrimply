use sanim_core::{Fraction, Span, TimeMap, TimeMapEntry, TimeMapKind};

fn entry(
    start: u64,
    end: u64,
    source_start: usize,
    source_end: usize,
    statement: usize,
) -> TimeMapEntry {
    TimeMapEntry {
        source: "scene.sanim".into(),
        span: Span {
            start: source_start,
            end: source_end,
            ..Span::default()
        },
        start: Fraction::new(start, 1000u64),
        end: Fraction::new(end, 1000u64),
        statement,
        kind: TimeMapKind::Statement,
        call_path: Default::default(),
    }
}
const BUDGET: usize = 1024 * 1024;
#[test]
fn frame_table_handles_exact_fractional_boundaries_and_zero_time_statements() {
    let mut map = TimeMap::default();
    map.push(entry(0, 100, 0, 10, 0), BUDGET).unwrap();
    map.push(entry(100, 100, 10, 20, 1), BUDGET).unwrap();
    map.push(entry(100, 200, 20, 30, 2), BUDGET).unwrap();
    map.index(Fraction::from(30), Fraction::new(1u64, 5u64), BUDGET)
        .unwrap();
    assert_eq!(map.indexed_frames(), 7);
    assert_eq!(map.at_frame(2).unwrap().statement, 0);
    assert_eq!(map.at_frame(3).unwrap().statement, 2);
    assert_eq!(map.at_frame(5).unwrap().statement, 2);
    assert!(map.at_frame(6).is_none());
    assert!(map.at_frame(usize::MAX).is_none());
}
#[test]
fn indexed_source_overlap_matches_an_independent_interval_oracle() {
    let mut map = TimeMap::default();
    let mut seed = 31u64;
    let mut intervals = Vec::new();
    for i in 0..150 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = (seed % 400) as usize;
        let end = start + 1 + ((seed >> 16) % 70) as usize;
        map.push(entry(i, i + 1, start, end, i as usize), BUDGET)
            .unwrap();
        intervals.push((start, end, Fraction::new(i, 1000u64)));
    }
    map.index(Fraction::from(1000), Fraction::from(1), BUDGET)
        .unwrap();
    for start in 0..500 {
        let end = start + 13;
        let expected = intervals
            .iter()
            .filter(|&&(a, b, _)| a < end && start < b)
            .map(|&(_, _, t)| t)
            .min();
        assert_eq!(
            map.earliest(
                "scene.sanim",
                Span {
                    start,
                    end,
                    ..Span::default()
                }
            ),
            expected
        );
    }
    assert!(map.earliest("missing.sanim", Span::default()).is_none());
}
#[test]
fn absurd_frame_rates_fail_before_allocating_the_frame_table() {
    let mut map = TimeMap::default();
    map.push(entry(0, 1000, 0, 10, 0), BUDGET).unwrap();
    let error = map
        .index(
            Fraction::from(1_000_000_000u64),
            Fraction::from(7200),
            BUDGET,
        )
        .unwrap_err();
    assert!(error.contains("time map needs"), "{error}");
    assert_eq!(map.indexed_frames(), 0);
}

#[test]
fn deserialized_metadata_is_still_charged_when_rebuilding_indexes() {
    let mut map = TimeMap::default();
    map.push(entry(0, 0, 0, 20, 0), BUDGET).unwrap();
    let bytes = serde_json::to_vec(&map).unwrap();
    let mut restored: TimeMap = serde_json::from_slice(&bytes).unwrap();
    assert!(
        restored
            .index(Fraction::from(30), Fraction::from(0), 64)
            .unwrap_err()
            .contains("metadata")
    );
    assert_eq!(restored.indexed_frames(), 0);
}
