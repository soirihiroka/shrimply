use sanim_core::media::Kind;
use std::sync::{Arc, Mutex};
static MEDIA_CACHE_TESTS: Mutex<()> = Mutex::new(());

const PNG: &[u8] = include_bytes!("../../../tests/fixtures/media-sample.png");
const SVG: &[u8] = include_bytes!("../../../tests/fixtures/media-sample.svg");

#[test]
fn native_media_is_shared_across_hosts_and_preserves_bitmap_alpha_and_svg_vectors() {
    let _guard = MEDIA_CACHE_TESTS.lock().unwrap();
    for (bytes, kind) in [(PNG, Kind::Bitmap), (SVG, Kind::Svg)] {
        let bytes = Arc::new(bytes.to_vec());
        let asset = sanim_data::media::decode(bytes.clone(), Default::default()).unwrap();
        assert_eq!((asset.width, asset.height), (160, 96));
        assert_eq!(asset.kind, kind);
        let native = shrimply_drawing_skia::media::prepare(bytes.clone()).unwrap();
        let same = shrimply_drawing_skia::media::prepare_hashed(asset.bytes(), asset.fingerprint())
            .unwrap();
        assert!(Arc::ptr_eq(&native, &same));
        assert_eq!(native.bitmap().is_some(), kind == Kind::Bitmap);
        assert!(native.contains([0.5, 0.5]));
        if kind == Kind::Bitmap {
            assert!(!native.contains([0.01, 0.01]));
        }
        assert!(
            shrimply_drawing_skia::media::prepare_hashed(&bytes, "forged")
                .err()
                .expect("forged fingerprint must fail")
                .contains("fingerprint")
        );
    }
}

#[test]
fn invalid_media_and_decoded_pixel_limits_fail_before_retaining_a_frame() {
    let _guard = MEDIA_CACHE_TESTS.lock().unwrap();
    assert!(
        sanim_data::media::decode(Arc::new(b"not an image".to_vec()), Default::default()).is_err()
    );
    let bytes = Arc::new(PNG.to_vec());
    let key = sanim_core::digest(&bytes);
    assert!(shrimply_drawing_skia::media::prepare_hashed_with_limit(&bytes, &key, 10).is_err());
    let _ = shrimply_drawing_skia::media::prepare(bytes.clone()).unwrap();
    // A pre-existing global-cache hit cannot bypass a stricter caller budget.
    assert!(shrimply_drawing_skia::media::prepare_hashed_with_limit(&bytes, &key, 10).is_err());
}

#[test]
fn replacing_many_sources_releases_old_native_values_without_a_collector() {
    let _guard = MEDIA_CACHE_TESTS.lock().unwrap();
    let bytes = Arc::new(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><!-- old source --></svg>".to_vec());
    let source = Arc::downgrade(&bytes);
    let prepared = shrimply_drawing_skia::media::prepare(bytes.clone()).unwrap();
    let native = Arc::downgrade(&prepared);
    drop(bytes);
    drop(prepared);
    for index in 0..260 {
        let bytes = Arc::new(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><rect width=\"1\" height=\"1\" fill=\"#{index:06x}\"/></svg>").into_bytes());
        shrimply_drawing_skia::media::prepare(bytes).unwrap();
    }
    assert!(native.upgrade().is_none(), "bounded cache eviction drops obsolete native DOMs");
    assert!(source.upgrade().is_none(), "weak identity hints retain no encoded payload");
}

#[test]
fn bitmap_native_cache_identity_is_independent_of_font_activation() {
    let _guard=MEDIA_CACHE_TESTS.lock().unwrap();
    let bytes=Arc::new(PNG.to_vec());
    let first=shrimply_drawing_skia::media::prepare(bytes.clone()).unwrap();
    let font=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/assets/DejaVuSansCondensed-subset.ttf");
    sanim_data::font::register_application_font(font).unwrap();
    let after=shrimply_drawing_skia::media::prepare(bytes).unwrap();
    assert!(Arc::ptr_eq(&first,&after));
}

#[test]
#[ignore = "manual shared media benchmark"]
fn media_benchmark() {
    let _guard = MEDIA_CACHE_TESTS.lock().unwrap();
    use std::{hint::black_box, time::Instant};
    for (name, source) in [("bitmap", PNG), ("SVG", SVG)] {
        let bytes = Arc::new(source.to_vec());
        let key = sanim_core::digest(&bytes);
        let cold = Instant::now();
        let first = shrimply_drawing_skia::media::prepare_hashed(&bytes, &key).unwrap();
        let cold = cold.elapsed();
        let start = Instant::now();
        for _ in 0..10_000 {
            assert!(Arc::ptr_eq(
                &first,
                &black_box(shrimply_drawing_skia::media::prepare_hashed(&bytes, &key).unwrap())
            ));
        }
        eprintln!(
            "shared {name} cold={cold:?}; 10000 validated native-cache hits={:?}",
            start.elapsed()
        );
    }
}
