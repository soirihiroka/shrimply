use super::*;
use crate::sanim::tests::Fixture;
use sanim_core::Fraction;
use std::time::{Duration, Instant};

#[test]
fn native_sanim_event_clocks_reload_without_active_frame_polling() {
    let fixture = Fixture::new(
        "return Animation(children=[wait(250ms),wait(750ms)],flow=\"sequence\").repeat(forever)",
    );
    let item = fixture.item();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let changes = source_events::Change::subscribe();
    let (sender, receiver) = std::sync::mpsc::channel();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = sender.send(());
    });
    changes.set_wake(Arc::downgrade(&wake));
    let mut source = Source::new(&item).unwrap();
    source
        .poll(&item, Time::ZERO, canvas, 30.into(), false)
        .unwrap();
    let wait_events = |expected_next| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(events) = source_events::for_item(&item, canvas)
                && events.index.neighbors(Fraction::new(21u64, 10u64))[1] == Some(expected_next)
            {
                break events;
            }
            receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("compiled clock change must notify the timeline without requesting another preview frame");
        }
    };
    let old = wait_events(Fraction::new(9u64, 4u64));
    let weak = Arc::downgrade(&old);
    fixture.write(
        "return Animation(children=[wait(500ms),wait(500ms)],flow=\"sequence\").repeat(forever)",
    );
    let current = wait_events(Fraction::new(5u64, 2u64));
    assert!(current.index.estimated_bytes() < 1024);
    drop(old);
    assert!(
        weak.upgrade().is_none(),
        "old event publications must release when a new source revision is installed"
    );

    fixture.write("return )");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = source.runtime.snapshot();
    while state.diagnostic.is_none() && Instant::now() < deadline {
        state = source.runtime.wait_for_change(
            state.generation,
            deadline.saturating_duration_since(Instant::now()),
        );
    }
    assert!(state.diagnostic.is_some());
    assert!(
        Arc::ptr_eq(&current, &source_events::for_item(&item, canvas).unwrap()),
        "a source syntax error retains the last valid divider state"
    );
}
