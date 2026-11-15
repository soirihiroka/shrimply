use sanim_core::{Fraction, Span, TimeMap, TimeMapEntry};

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
        end: Some(Fraction::new(end, 1000u64)),
        statement,
        call_path: Default::default(),
        clock: None,
    }
}
const BUDGET: usize = 1024 * 1024;
#[test]
fn periodic_event_index_retains_one_period_at_late_playback_times() {
    let mut map = TimeMap::default();
    let clock =
        std::sync::Arc::new(sanim_core::math::cycle::Cycle::new(0.into(), 1.into()).unwrap());
    for (start, end) in [(0, 250), (250, 1000)] {
        let mut event = entry(start, end, 0, 10, 0);
        event.clock = Some(clock.clone());
        map.push(event, BUDGET).unwrap();
    }
    map.index(None, Some(1.into()), BUDGET).unwrap();
    let index = map.event_index();
    assert_eq!(index.groups().len(), 1);
    assert_eq!(
        index.groups()[0].boundaries.as_ref(),
        &[
            Fraction::from(0),
            Fraction::new(1u64, 4u64),
            Fraction::from(1)
        ]
    );
    assert_eq!(
        index.neighbors(Fraction::new(5_000_001u64, 10u64)),
        [
            Some(500_000.into()),
            Some(Fraction::new(2_000_001u64, 4u64))
        ]
    );
    assert!(std::sync::Arc::ptr_eq(&index, &map.clone().event_index()));
}
#[test]
fn frame_table_handles_exact_fractional_boundaries_and_zero_time_statements() {
    let mut map = TimeMap::default();
    map.push(entry(0, 100, 0, 10, 0), BUDGET).unwrap();
    map.push(entry(100, 100, 10, 20, 1), BUDGET).unwrap();
    map.push(entry(100, 200, 20, 30, 2), BUDGET).unwrap();
    map.index(
        Some(Fraction::from(30)),
        Some(Fraction::new(1u64, 5u64)),
        BUDGET,
    )
    .unwrap();
    assert_eq!(map.indexed_frames(), Some(7));
    assert_eq!(map.at_frame(2).unwrap().statement, 0);
    assert_eq!(map.at_frame(3).unwrap().statement, 2);
    assert_eq!(map.at_frame(5).unwrap().statement, 2);
    assert!(map.at_frame(6).is_none());
    assert!(map.at_frame(usize::MAX).is_none());
    assert_eq!(
        map.event_index().finite().as_ref(),
        &[
            Fraction::from(0),
            Fraction::new(1u64, 10u64),
            Fraction::new(1u64, 5u64)
        ]
    );
    assert!(std::sync::Arc::ptr_eq(
        &map.event_index(),
        &map.clone().event_index()
    ));
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
    map.index(Some(Fraction::from(1000)), Some(Fraction::from(1)), BUDGET)
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
fn huge_frame_extents_use_compact_winner_runs() {
    let mut map = TimeMap::default();
    map.push(entry(0, 1000, 0, 10, 0), BUDGET).unwrap();
    map.push(entry(7_199_000, 7_200_000, 10, 20, 1), BUDGET)
        .unwrap();
    map.index(
        Some(Fraction::from(1_000_000_000u64)),
        Some(Fraction::from(7200)),
        BUDGET,
    )
    .unwrap();
    assert_eq!(map.indexed_frames(), Some(7_200_000_000_001));
    assert_eq!(map.at_frame(7_199_500_000_000).unwrap().statement, 1);
    assert!(map.at_frame(7_200_000_000_000).is_none());
}

#[test]
fn late_periodic_frames_resolve_the_child_source_without_expanding_cycles() {
    let mut map = TimeMap::default();
    let clock =
        std::sync::Arc::new(sanim_core::math::cycle::Cycle::new(0.into(), 1.into()).unwrap());
    map.push(entry(0, 500_000_000, 0, 100, 0), BUDGET).unwrap();
    for (start, end, source) in [(0, 500, 10), (500, 1000, 20)] {
        let mut child = entry(start, end, source, source + 5, 1);
        child.clock = Some(clock.clone());
        map.push(child, BUDGET).unwrap();
    }
    map.index(Some(30.into()), Some(500_000.into()), 4096)
        .unwrap();
    assert_eq!(map.at_frame(3_000_007).unwrap().span.start, 10);
    assert_eq!(map.at_frame(3_000_023).unwrap().span.start, 20);
    assert_eq!(map.indexed_frames(), Some(15_000_001));
}

#[test]
fn deserialized_metadata_is_still_charged_when_rebuilding_indexes() {
    let mut map = TimeMap::default();
    map.push(entry(0, 0, 0, 20, 0), BUDGET).unwrap();
    let bytes = serde_json::to_vec(&map).unwrap();
    let mut restored: TimeMap = serde_json::from_slice(&bytes).unwrap();
    assert!(
        restored
            .index(Some(Fraction::from(30)), Some(Fraction::from(0)), 64)
            .unwrap_err()
            .contains("metadata")
    );
    assert_eq!(restored.indexed_frames(), Some(0));
}

#[test]
#[ignore = "release source-index performance regression benchmark"]
fn compact_source_lookup_benchmark() {
    use std::time::Instant;
    let clock =
        std::sync::Arc::new(sanim_core::math::cycle::Cycle::new(0.into(), 1.into()).unwrap());
    let mut map = TimeMap::default();
    for index in 0..1000 {
        let mut value = entry(
            index,
            index + 1,
            index as usize,
            index as usize + 1,
            index as usize,
        );
        value.clock = Some(clock.clone());
        map.push(value, BUDGET).unwrap();
    }
    let started = Instant::now();
    map.index(Some(1000.into()), Some(500_000.into()), BUDGET)
        .unwrap();
    let indexed = started.elapsed();
    let started = Instant::now();
    for query in 0..100_000 {
        let phase = query % 1000;
        let value = std::hint::black_box(map.at_frame(300_000_000 + phase).unwrap());
        assert_eq!(value.statement, phase);
    }
    println!(
        "500-million-frame source index: build={}us, 100000 late cyclic queries={}us; metadata budget={} bytes",
        indexed.as_micros(),
        started.elapsed().as_micros(),
        BUDGET
    );
}

#[test]
fn periodic_source_ranges_and_points_remain_exact_between_frame_boundaries() {
    for (period, start, end, inside, outside) in [(50u64, 5, 20, 2, 1), (75, 25, 25, 3, 2)] {
        let mut map = TimeMap::default();
        let clock = std::sync::Arc::new(
            sanim_core::math::cycle::Cycle::new(0.into(), Fraction::new(period, 1000u64)).unwrap(),
        );
        map.push(entry(0, 1000, 0, 10, 0), BUDGET).unwrap();
        let mut child = entry(start, end, 20, 30, 1);
        child.clock = Some(clock);
        map.push(child, BUDGET).unwrap();
        map.index(Some(30.into()), Some(1.into()), 4096).unwrap();
        assert_eq!(map.at_frame(inside).unwrap().span.start, 20);
        assert_eq!(map.at_frame(outside).unwrap().span.start, 0);
    }
}

#[test]
fn optional_source_clock_extents_still_validate_rate_and_duration() {
    for rate in [Fraction::from(0), -Fraction::from(1), Fraction::NaN] {
        let mut map = TimeMap::default();
        assert!(map.index(Some(rate), None, BUDGET).is_err());
        assert_eq!(map.indexed_frames(), Some(0));
    }
    for duration in [-Fraction::from(1), Fraction::NaN] {
        let mut map = TimeMap::default();
        assert!(map.index(None, Some(duration), BUDGET).is_err());
    }
}
