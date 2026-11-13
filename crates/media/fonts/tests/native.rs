#![cfg(feature = "native")]

use shrimply_fonts::{Font, native};

#[test]
fn editor_layout_and_headless_shapes_share_the_exact_outline_artifact() {
    use std::{rc::Rc, sync::Arc};
    let font = Font::Asset(Arc::new(
        include_bytes!("../../../../sanim/examples/assets/DejaVuSansCondensed-subset.ttf").to_vec(),
    ));
    let options = native::shaping::Options::new(24.);
    let layout = native::shaping::layout(&font, "Shared AV ffi", &options).unwrap();
    let repeated = native::shaping::layout(&font, "Shared AV ffi", &options).unwrap();
    assert!(Rc::ptr_eq(&layout, &repeated));
    let shape = native::shaping::shape(&font, "Shared AV ffi", &options).unwrap();
    assert!(Arc::ptr_eq(layout.shaped().unwrap(), &shape));
    let worker_font = font.clone();
    let shared = std::thread::spawn(move || {
        native::shaping::shape(
            &worker_font,
            "Shared AV ffi",
            &native::shaping::Options::new(24.),
        )
        .unwrap()
    })
    .join()
    .unwrap();
    assert!(
        Arc::ptr_eq(&shape, &shared),
        "immutable glyphs cross render-thread boundaries without copying Pango objects"
    );
    assert!(shape.advance > 0.);
    assert_eq!(shape.missing_glyphs, 0);
    assert!(!shape.color_glyphs);
    assert_eq!(shape.origin[0], 0.);
    assert!(shape.origin[1] < 0.);
    let mut bold = options.clone();
    bold.weight = 700;
    let styled = native::shaping::shape(&font, "Shared AV ffi", &bold).unwrap();
    assert!(!Arc::ptr_eq(&shape, &styled));
    let mut tracked = options.clone();
    tracked.tracking = 2.;
    let tracked = native::shaping::shape(&font, "Shared AV ffi", &tracked).unwrap();
    assert!(tracked.advance > shape.advance);
}

#[test]
fn native_shaping_validates_resources_and_retains_intrinsic_empty_width() {
    let font = Font::default();
    let options = native::shaping::Options::new(18.);
    let shape = native::shaping::shape(&font, "", &options).unwrap();
    assert_eq!(shape.advance, 0.);
    assert_eq!(shape.bounds[0], shape.bounds[2]);
    assert!(
        shape
            .outline
            .iter()
            .all(|segment| matches!(segment, shrimply_fonts::TextSegment::Move(_))),
        "empty text must not produce drawn contours"
    );
    assert!(native::shaping::shape(&font, "x", &native::shaping::Options::new(0.)).is_err());
    let mut invalid = options.clone();
    invalid.line_spacing = f64::MAX;
    assert!(native::shaping::shape(&font, "x", &invalid).is_err());
    assert!(
        native::shaping::shape(
            &font,
            &"x".repeat(native::shaping::MAX_TEXT_BYTES + 1),
            &options
        )
        .is_err()
    );
}

#[test]
fn cached_asset_shapes_cannot_hide_copy_on_write_resource_changes() {
    use std::sync::Arc;
    let mut font = Font::Asset(Arc::new(
        include_bytes!("../../../../sanim/examples/assets/DejaVuSansCondensed-subset.ttf").to_vec(),
    ));
    let options = native::shaping::Options::new(18.);
    let original = native::shaping::shape(&font, "Asset identity", &options).unwrap();
    let Font::Asset(bytes) = &mut font else {
        unreachable!()
    };
    Arc::make_mut(bytes).clear();
    assert!(
        native::shaping::shape(&font, "Asset identity", &options).is_err(),
        "mutated bytes must not reuse the old native outline by allocation address"
    );
    assert!(!original.outline.is_empty());
}

#[test]
fn shaping_new_text_does_not_invalidate_the_font_environment() {
    const CHILD: &str = "SHRIMPLY_FONT_EPOCH_SHAPING_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shaping_new_text_does_not_invalidate_the_font_environment",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        return;
    }
    let font = Font::default();
    let options = native::shaping::Options::new(24.);
    native::shaping::shape(&font, "Warm the native family", &options).unwrap();
    let epoch = native::system_font_epoch();
    for index in 0..100 {
        native::shaping::shape(&font, &format!("new text {index}"), &options).unwrap();
        assert_eq!(
            native::system_font_epoch(),
            epoch,
            "glyph/layout cache misses must not trigger full execution-state invalidation"
        );
    }
}

#[test]
#[ignore = "measured by make font-shaping-bench"]
fn shared_font_shaping_benchmark() {
    use std::{hint::black_box, time::Instant};
    const CALLS: usize = 500;
    let font = Font::default();
    let text = "Shared shaping: cached outlines, ligatures ffi, and fallback السلام";
    let options = native::shaping::Options::new(24.);
    let expected = native::shaping::shape(&font, text, &options).unwrap();
    let started = Instant::now();
    for _ in 0..CALLS {
        let prepared = native::prepare(&font, 24.).unwrap();
        prepared.layout.set_text(text);
        black_box(prepared.layout.extents());
        let surface = cairo::RecordingSurface::create(cairo::Content::Alpha, None).unwrap();
        let canvas = cairo::Context::new(&surface).unwrap();
        pangocairo::functions::layout_path(&canvas, &prepared.layout);
        black_box(canvas.copy_path().unwrap());
    }
    let uncached = started.elapsed();
    let started = Instant::now();
    for _ in 0..CALLS {
        let shape = native::shaping::shape(&font, text, &options).unwrap();
        assert!(std::sync::Arc::ptr_eq(&shape, &expected));
        black_box(shape);
    }
    let outlines = started.elapsed();
    let first = native::shaping::layout(&font, text, &options).unwrap();
    let started = Instant::now();
    for _ in 0..CALLS {
        let layout = native::shaping::layout(&font, text, &options).unwrap();
        assert!(std::rc::Rc::ptr_eq(&layout, &first));
        black_box(layout);
    }
    eprintln!(
        "Shared font shaping {CALLS} calls: uncached Pango/path={uncached:?}; shared immutable outlines={outlines:?}; cached native layout={:?}",
        started.elapsed()
    );
}

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
