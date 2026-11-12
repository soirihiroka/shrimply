use sanim_lang::{
    Engine, MemorySources,
    progress::{Observer, Phase, Progress, State},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn observed() -> (Engine, Arc<Mutex<Vec<Progress>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let observer: Observer = Arc::new(move |progress| sink.lock().unwrap().push(progress));
    let mut engine = Engine::default();
    engine.set_progress_observer(Some(observer));
    (engine, events)
}

#[test]
fn actual_work_counts_are_monotonic_and_finish_once() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"values\" as values\nlet rows = read<List<Sample>>(\"rows.csv\")\ntype Sample = {value: Number}\nlet result = query(\"SELECT value FROM rows\", tables = {rows = rows})\nrepeat 400 { wait(1ms) }\nassert(len(result) == values.count)\n");
    files.insert("values.sanim", "let count = 2\n");
    const CSV: &str = "value\n1\n2\n";
    files.insert("rows.csv", CSV);
    let (mut engine, events) = observed();
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let events = events.lock().unwrap();
    assert_eq!(events.last().unwrap().state, State::Ready);
    assert_eq!(
        events.iter().filter(|p| p.state != State::Running).count(),
        1
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].elapsed_micros <= pair[1].elapsed_micros)
    );
    for event in events.iter() {
        if let Some(total) = event.total {
            assert!(
                event.completed.is_some_and(|done| done <= total),
                "{event:?}"
            );
        }
    }
    let evaluating: Vec<_> = events
        .iter()
        .filter(|p| p.phase == Phase::Evaluating)
        .collect();
    assert!(!evaluating.is_empty());
    assert!(
        evaluating
            .windows(2)
            .all(|pair| pair[0].completed <= pair[1].completed)
    );
    let last = evaluating.last().unwrap();
    assert_eq!(last.completed, last.total);
    assert_eq!(last.total, Some(5));
    assert!(events.iter().any(|p| p.phase == Phase::Reading
        && p.resource.as_deref() == Some("rows.csv")
        && p.completed == Some(CSV.len() as u64)
        && p.total == Some(CSV.len() as u64)));
    for phase in [Phase::Decoding, Phase::Querying] {
        assert!(
            events
                .iter()
                .any(|p| p.phase == phase && p.completed.is_none() && p.total.is_none())
        );
    }
}

#[test]
fn compile_and_resource_failures_are_terminal_without_false_success() {
    for source in [
        "let value: Color = 1",
        "let data = read<Dict>(\"missing.json\")",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let (mut engine, events) = observed();
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        let events = events.lock().unwrap();
        assert_eq!(events.last().unwrap().state, State::Failed);
        assert_eq!(
            events.iter().filter(|p| p.state != State::Running).count(),
            1
        );
        assert_eq!(
            events.last().unwrap().resource.as_deref(),
            Some("main.sanim")
        );
    }
}

#[test]
fn duration_fallback_is_a_single_job_without_transient_failure() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let marker = Circle()\nlet motions = [Shift(marker, Vec2(1pt, 0pt))]\nfor motion in motions { wait(1s) }\n",
    );
    let (mut engine, events) = observed();
    let report = engine
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(
        report
            .performance
            .iter()
            .any(|warning| warning.code == "P004")
    );
    let events = events.lock().unwrap();
    assert_eq!(events.last().unwrap().state, State::Ready);
    assert_eq!(
        events.iter().filter(|p| p.state != State::Running).count(),
        1
    );
    assert!(events.iter().any(|p| p.phase == Phase::Duration));
}
