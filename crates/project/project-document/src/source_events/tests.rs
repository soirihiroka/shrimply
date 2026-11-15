use super::*;
use crate::project::{Asset, GeneratedEventTiming, RepeatStrategy, generated_event_neighbors};
use shrimply_math_core::{Fraction, fraction_from_integer};
static PUBLICATIONS: Mutex<()> = Mutex::new(());
fn finite_index(values: Vec<Fraction>) -> Arc<shrimply_math_core::events::Index> {
    Arc::new(shrimply_math_core::events::Index::new(values, Vec::new(), MAX_BYTES).unwrap())
}

fn timing() -> GeneratedEventTiming {
    GeneratedEventTiming {
        start: Time::from_seconds(10),
        end: Time::from_seconds(30),
        animation_time_offset: Time::ZERO,
        playback_speed: Fraction::from(1),
        repeat_strategy: RepeatStrategy::Hold,
    }
}
fn events() -> Events {
    Events {
        duration: Time::from_seconds(4),
        index: finite_index(vec![0.into(), 1.into(), 3.into(), 4.into()]),
    }
}

#[test]
fn sanim_event_projection_matches_exhaustive_repeat_oracle() {
    let events = events();
    for strategy in [
        RepeatStrategy::Hold,
        RepeatStrategy::Empty,
        RepeatStrategy::Repeat,
        RepeatStrategy::PingPong,
    ] {
        for speed in [-3, -1, 1, 2] {
            for offset in [-4, 0, 2, 7] {
                let mut timing = timing();
                timing.playback_speed = fraction_from_integer(speed);
                timing.animation_time_offset = Time::from_seconds(offset);
                timing.repeat_strategy = strategy;
                let mut oracle = Vec::new();
                let periodic =
                    matches!(strategy, RepeatStrategy::Repeat | RepeatStrategy::PingPong);
                for cycle in if periodic { -30..31 } else { 0..1 } {
                    let period = if strategy == RepeatStrategy::PingPong {
                        8
                    } else {
                        4
                    };
                    let base = fraction_from_integer(cycle * period);
                    for boundary in events.index.finite().iter() {
                        let mut source_times = vec![base + *boundary];
                        if strategy == RepeatStrategy::PingPong {
                            source_times.push(base + Fraction::from(8) - *boundary);
                        }
                        for source in source_times {
                            let at = Time {
                                seconds: timing.start.seconds
                                    + (source - timing.animation_time_offset.seconds)
                                        / timing.playback_speed,
                            };
                            if at >= timing.start && at <= timing.end {
                                oracle.push(at);
                            }
                        }
                    }
                }
                oracle.sort_unstable();
                oracle.dedup();
                for tick in 0..=80 {
                    let at = Time {
                        seconds: Fraction::from(9) + Fraction::new(tick as u64, 3u64),
                    };
                    let clamped = at.max(timing.start).min(timing.end);
                    let expected = [
                        oracle
                            .iter()
                            .copied()
                            .filter(|value| *value <= clamped)
                            .max(),
                        oracle
                            .iter()
                            .copied()
                            .filter(|value| *value >= clamped)
                            .min(),
                    ];
                    assert_eq!(
                        generated_event_neighbors(timing, &events.index, events.duration, at),
                        expected,
                        "strategy={strategy:?} speed={speed} offset={offset} at={at:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn sanim_event_lookup_handles_empty_and_frozen_sources() {
    let mut timing = timing();
    assert_eq!(
        generated_event_neighbors(
            timing,
            &shrimply_math_core::events::Index::default(),
            Time::ZERO,
            timing.start
        ),
        [None, None]
    );
    timing.playback_speed = Fraction::from(0);
    let events = events();
    assert_eq!(
        generated_event_neighbors(timing, &events.index, events.duration, timing.start),
        [None, None]
    );
}

#[test]
fn sanim_event_inner_clocks_remain_visible_in_late_clip_cycles() {
    use shrimply_math_core::events::{Group, Index};
    let duration = Time::from_seconds(500_001);
    let index = Index::new(
        vec![0.into(), duration.seconds],
        vec![Group {
            start: 0.into(),
            end: 1.into(),
            until: Some(duration.seconds),
            boundaries: vec![0.into(), Fraction::new(1u64, 4u64), 1.into()].into(),
        }],
        MAX_BYTES,
    )
    .unwrap();
    let mut timing = timing();
    timing.end = Time::from_seconds(500_011);
    let at = Time {
        seconds: Fraction::new(5_000_101u64, 10u64),
    };
    assert_eq!(
        generated_event_neighbors(timing, &index, duration, at),
        [
            Some(Time::from_seconds(500_010)),
            Some(Time {
                seconds: Fraction::new(2_000_041u64, 4u64)
            })
        ]
    );
    timing.animation_time_offset = duration;
    timing.playback_speed = Fraction::from(-1);
    let exact = Time {
        seconds: Fraction::new(43u64, 4u64),
    };
    assert_eq!(
        generated_event_neighbors(timing, &index, duration, exact),
        [Some(exact); 2]
    );
}

#[test]
fn sanim_event_publication_is_shared_bounded_and_configuration_safe() {
    let _publications = PUBLICATIONS.lock().unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let mut item = VideoItem::shape_item(canvas, Time::ZERO, Time::from_seconds(4));
    item.file = Asset::new(
        std::env::temp_dir().join(format!("sanim-event-{}.sanim", uuid::Uuid::new_v4())),
    );
    item.content = VideoItemContent::Sanim(Box::default());
    let change = Change::subscribe();
    publish_import(item.file.path(), canvas, events());
    assert!(change.pending());
    change.take();
    let imported = for_item(&item, canvas).unwrap();
    assert_eq!(imported.index, events().index);
    publish_import(item.file.path(), canvas, events());
    assert!(
        !change.pending(),
        "unchanged source events cannot trigger redraw flashes"
    );
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings
        .parameters
        .insert("delay".into(), serde_json::json!(2));
    assert!(
        for_item(&item, canvas).is_none(),
        "default import timing cannot masquerade as overridden timing"
    );
    publish_clip(&ClipContext::for_item(&item, canvas).unwrap(), events());
    let compiled = for_item(&item, canvas).unwrap();
    assert!(Arc::ptr_eq(&compiled, &for_item(&item, canvas).unwrap()));
    item.end = Time::from_seconds(5);
    assert!(
        for_item(&item, canvas).is_none(),
        "resized clips must resolve temporal layout before publishing new dividers"
    );
    item.end = Time::from_seconds(4);
    item.file = Asset::new("another.sanim");
    assert!(
        for_item(&item, canvas).is_none(),
        "a reused clip identity cannot expose another source's events"
    );
    assert!(registry().lock().unwrap().sources.usage().0 <= MAX_BYTES);
}

#[test]
fn sanim_event_revisions_release_old_state_and_wake_without_registry_locks() {
    let _publications = PUBLICATIONS.lock().unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let mut item = VideoItem::shape_item(canvas, Time::ZERO, Time::from_seconds(4));
    item.file = Asset::new(format!("event-memory-{}.sanim", uuid::Uuid::new_v4()));
    item.content = VideoItemContent::Sanim(Box::default());
    let clip = ClipContext::for_item(&item, canvas).unwrap();
    let change = Change::subscribe();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wake: Arc<Wake> = Arc::new({
        let item = item.clone();
        let calls = calls.clone();
        move || {
            assert!(for_item(&item, canvas).is_some());
            calls.fetch_add(1, Ordering::Relaxed);
        }
    });
    change.set_wake(Arc::downgrade(&wake));
    let mut previous = Weak::<Events>::new();
    for index in 0..200u64 {
        publish_clip(
            &clip,
            Events {
                duration: Time::from_seconds(4),
                index: finite_index(vec![Fraction::new(index, 100u64)]),
            },
        );
        assert!(
            previous.upgrade().is_none(),
            "old hot-reload revisions cannot accumulate in the publication cache"
        );
        previous = Arc::downgrade(&for_item(&item, canvas).unwrap());
    }
    assert_eq!(calls.load(Ordering::Relaxed), 200);
    drop(wake);
    publish_clip(&clip, events());
    assert_eq!(
        calls.load(Ordering::Relaxed),
        200,
        "a closed host is never retained by its subscription"
    );
}

#[test]
#[ignore = "manual performance regression measurement"]
fn sanim_event_lookup_benchmark() {
    let count = 100_000u64;
    let events = Events {
        duration: Time::from_seconds(100),
        index: finite_index(
            (0..count)
                .map(|index| Fraction::new(index, 1000u64))
                .collect(),
        ),
    };
    let mut timing = timing();
    timing.end = Time::from_seconds(7_200);
    timing.repeat_strategy = RepeatStrategy::Repeat;
    let started = std::time::Instant::now();
    for index in 0..100_000u64 {
        std::hint::black_box(generated_event_neighbors(
            timing,
            &events.index,
            events.duration,
            Time {
                seconds: Fraction::new(index, 17u64),
            },
        ));
    }
    eprintln!(
        "100k indexed queries over 100k source boundaries and a two-hour repeated clip: {:?}; shared boundary bytes={}",
        started.elapsed(),
        events.index.estimated_bytes()
    );
    assert_eq!(events.index.finite().len(), count as usize);
    timing.start = Time::ZERO;
    timing.end = events.duration;
    timing.repeat_strategy = RepeatStrategy::Hold;
    let queries = (0..1000u64)
        .map(|index| Time {
            seconds: Fraction::new(index, 17u64),
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    for at in &queries {
        std::hint::black_box(generated_event_neighbors(
            timing,
            &events.index,
            events.duration,
            *at,
        ));
    }
    let indexed = started.elapsed();
    let started = std::time::Instant::now();
    let boundaries = events.index.finite();
    for at in &queries {
        std::hint::black_box(
            boundaries
                .iter()
                .filter(|value| **value <= at.seconds)
                .max(),
        );
        std::hint::black_box(
            boundaries
                .iter()
                .filter(|value| **value >= at.seconds)
                .min(),
        );
    }
    let linear = started.elapsed();
    eprintln!(
        "same 1000 queries: indexed={indexed:?}, full scan={linear:?}, speedup={:.1}x",
        linear.as_secs_f64() / indexed.as_secs_f64()
    );
    assert!(
        indexed < linear,
        "indexed source events must outperform full-array scans"
    );
}
