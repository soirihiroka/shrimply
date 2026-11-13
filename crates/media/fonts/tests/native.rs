#![cfg(feature = "native")]

use shrimply_fonts::{Font, native};

#[test]
fn application_fonts_are_atomic_and_reach_already_running_render_threads() {
    let asset = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../sanim/examples/assets/DejaVuSansCondensed-subset.ttf");
    let (ready, wait_ready) = std::sync::mpsc::channel();
    let (resume, wait_resume) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let before = native::system_font_epoch().unwrap();
        let prepared = native::prepare(&Font::default(), 24.).unwrap();
        prepared.layout.set_text("Shared native fonts");
        prepared.layout.extents();
        ready.send(before.revision).unwrap();
        wait_resume.recv().unwrap();
        let map = native::system_map().unwrap();
        let after = native::system_font_epoch().unwrap();
        assert!(after.revision > before.revision);
        let prepared = native::prepare(&Font::default(), 24.).unwrap();
        assert_eq!(prepared.layout.context().font_map().unwrap(), map);
        after.revision
    });
    let revision = wait_ready.recv().unwrap();
    assert!(
        native::register_application_fonts([&asset, &asset.with_file_name("absent-font.ttf")])
            .is_err()
    );
    assert_eq!(native::system_font_epoch().unwrap().revision, revision);
    let invalid = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(invalid.path(), b"not a font").unwrap();
    assert!(native::register_application_fonts([asset.as_path(), invalid.path()]).is_err());
    assert_eq!(native::system_font_epoch().unwrap().revision, revision);
    native::register_application_font(&asset).unwrap();
    let registered = native::system_font_epoch().unwrap();
    native::register_application_font(&asset).unwrap();
    assert_eq!(native::system_font_epoch(), Some(registered));
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), registered.revision);
}

#[test]
#[ignore = "measured by make font-system-bench"]
fn shared_font_epoch_warm_lookup_benchmark() {
    let expected = native::system_font_epoch().unwrap();
    const LOOKUPS: u32 = 100_000;
    let started = std::time::Instant::now();
    for _ in 0..LOOKUPS {
        assert_eq!(
            std::hint::black_box(native::system_font_epoch()),
            Some(expected)
        );
    }
    eprintln!(
        "shared font epoch: {LOOKUPS} lookups, {} ns/lookup",
        started.elapsed().as_nanos() / u128::from(LOOKUPS)
    );
}
