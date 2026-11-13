use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::sync::Arc;

#[test]
fn native_font_changes_invalidate_future_states_even_with_initially_empty_text() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut content=\"\"\nlet label=Text(content)\nlet code=CodeView(code=content,lang=\"plain\")\nadd(label)\nadd(code)\nwait(1s)\ncontent=\"Lazy native fonts\"\nwait(1s)");
    let animation = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap()
        .animation;
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation.clone()).unwrap();
    let options = Options::for_animation(scene.animation());
    let time = Fraction::new(3u64, 2u64);
    let before = renderer.record(&scene, time, &options).unwrap();
    let computed = renderer.states_computed;
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assets/DejaVuSansCondensed-subset.ttf");
    let epoch = sanim_data::font::system_font_epoch();
    sanim_data::font::register_application_font(font).unwrap();
    assert_ne!(epoch, sanim_data::font::system_font_epoch());
    let after = renderer.record(&scene, time, &options).unwrap();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(renderer.states_computed, computed + 1);
    let refreshed = renderer.prepare(animation).unwrap();
    let replay = renderer.record(&refreshed, time, &options).unwrap();
    assert!(Arc::ptr_eq(&after, &replay));
}

#[test]
fn state_cache_byte_limits_include_folded_source_and_font_assets() {
    use sanim_render::CacheLimits;
    let mut files = MemorySources::default();
    files.insert(
        "source.py",
        format!(
            "def example():\n    # {}\n    return 1\n",
            "x".repeat(64 * 1024)
        ),
    );
    files.insert(
        "font.ttf",
        include_bytes!("../../../examples/assets/DejaVuSansCondensed-subset.ttf").as_slice(),
    );
    for source in [
        "let code=CodeView(src=\"source.py\",width=200pt,height=100pt,line_numbers=false)\nadd(code)\ncode.fold(level=1,duration=0s)\nwait(1s)",
        "let label=Text(\"\",font=read<Font>(\"font.ttf\"))\nadd(label)\nwait(1s)",
    ] {
        files.insert("main.sanim", source);
        let animation = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap()
            .animation;
        let mut renderer = Renderer::with_cache_limits(CacheLimits {
            state_bytes: 4096,
            ..Default::default()
        });
        let scene = renderer.prepare(animation).unwrap();
        let options = Options::for_animation(scene.animation());
        let first = renderer.frame_state(&scene, 0.into(), &options).unwrap();
        let replay = renderer.frame_state(&scene, 0.into(), &options).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &replay),
            "native payloads larger than the byte limit must not enter the state cache"
        );
    }
}

#[test]
#[ignore = "manual native endpoint benchmark"]
fn native_endpoint_benchmark() {
    use std::{hint::black_box, time::Instant};
    let source = (0..300)
        .map(|i| {
            format!("def function_{i}(value):\n    result = value + {i}\n    return result\n\n")
        })
        .collect::<String>();
    let mut files = MemorySources::default();
    files.insert("source.py", source);
    files.insert("main.sanim", "let code=CodeView(src=\"source.py\",width=600pt,height=320pt,line_numbers=false)\nadd(code)\nwait(1s)\ncode.fold(level=1,duration=1s,ease=\"linear\")\nwait(1s)");
    let animation = Engine::new(Viewport {
        width: 1280,
        height: 720,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation;
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.frame_state(&scene, 1.into(), &options).unwrap();
    let pending = scene
        .animation()
        .sample_element(
            scene.animation().find("code").unwrap(),
            Fraction::new(3u64, 2u64),
        )
        .unwrap();
    for reuse_roots in [false, true] {
        let mut runs = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            for _ in 0..60 {
                let mut sampled = pending.clone();
                if !reuse_roots {
                    let sanim_core::Geometry::Code {
                        presentation: Some(presentation),
                        ..
                    } = Arc::make_mut(&mut sampled.geometry)
                    else {
                        panic!("code presentation")
                    };
                    presentation.from = Arc::new(presentation.from.as_ref().clone());
                    presentation.to = Arc::new(presentation.to.as_ref().clone());
                }
                sanim_data::resolve_pending(&mut sampled, Fraction::new(3u64, 2u64)).unwrap();
                black_box(sampled);
            }
            runs.push(started.elapsed());
        }
        runs.sort();
        eprintln!(
            "CodeView 300 functions / 60 native endpoint samples, shared snapshot identities={reuse_roots}: median {:?}",
            runs[1]
        );
    }
    let started = Instant::now();
    for index in 1..=30u64 {
        black_box(
            renderer
                .frame_state(&scene, Fraction::new(30 + index, 30u64), &options)
                .unwrap(),
        );
    }
    let cold = started.elapsed();
    let started = Instant::now();
    for index in (1..=30u64).rev() {
        black_box(
            renderer
                .frame_state(&scene, Fraction::new(30 + index, 30u64), &options)
                .unwrap(),
        );
    }
    eprintln!(
        "CodeView 300 functions / 30 fold states: {cold:?}; cached reverse {:?}",
        started.elapsed()
    );
}
