use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use std::{collections::BTreeMap, sync::Arc};

fn sources() -> MemorySources {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "return Animation(children=[wait(250ms),wait(750ms)],flow=\"sequence\").repeat(forever)",
    );
    sources
}

#[test]
fn canonical_layout_clocks_reach_duration_reports_without_expansion() {
    let report = Engine::default()
        .duration("main.sanim", &sources(), &BTreeMap::new())
        .unwrap();
    assert!(report.indefinite);
    assert_eq!(report.event_index.groups().len(), 1);
    assert_eq!(
        report
            .event_index
            .neighbors(Fraction::new(5_000_001u64, 10u64)),
        [
            Some(500_000.into()),
            Some(Fraction::new(2_000_001u64, 4u64))
        ]
    );
    assert!(report.event_index.estimated_bytes() < 1024);
}

#[test]
fn finite_repeat_three_reports_the_middle_cycle_boundary() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "return wait(1s).repeat(3)");
    let report = Engine::default()
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    for boundary in 0..=3 {
        let at = Fraction::from(boundary);
        assert_eq!(report.event_index.neighbors(at), [Some(at); 2]);
    }
    assert_eq!(
        report.event_index.neighbors(Fraction::new(3u64, 2u64)),
        [Some(1.into()), Some(2.into())]
    );
    assert_eq!(
        report.event_index.neighbors(4.into()),
        [Some(3.into()), None]
    );
}

#[test]
fn explicitly_fixed_parent_clips_its_periodic_child() {
    let mut engine = Engine::default();
    engine.available_duration = Some(Fraction::new(9u64, 4u64));
    let mut files = sources();
    files.insert("main.sanim","return Animation(children=[Animation(children=[wait(250ms),wait(750ms)],flow=\"sequence\").repeat(forever)],duration=2250ms)");
    let report = engine
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(!report.indefinite);
    assert_eq!(
        report.event_index.neighbors(Fraction::new(21u64, 10u64)),
        [Some(2.into()), Some(Fraction::new(9u64, 4u64))]
    );
    assert_eq!(
        report.event_index.neighbors(3.into()),
        [Some(Fraction::new(9u64, 4u64)), None]
    );
}

#[test]
fn late_clock_measurement_obeys_a_small_budget_and_releases_revisions() {
    let mut engine = Engine::default();
    engine.limits.max_time_map_bytes = 16 * 1024;
    engine.available_duration = Some(500_001.into());
    let mut previous = std::sync::Weak::<sanim_core::EventIndex>::new();
    for _ in 0..32 {
        let report = engine
            .duration("main.sanim", &sources(), &BTreeMap::new())
            .unwrap();
        assert!(
            previous.upgrade().is_none(),
            "duration reports cannot retain old clock indexes"
        );
        assert!(report.event_index.estimated_bytes() < 1024);
        previous = Arc::downgrade(&report.event_index);
    }
}
