use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};

fn scene(file: &str, bytes: &[u8]) -> sanim_core::Animation {
    let mut sources = MemorySources::default();
    sources.insert(file, bytes);
    sources.insert("main.sanim", format!("bg(Color(\"transparent\"))\nmut width=120pt\nlet image=<Image src=\"{file}\" width={{width}}/>\nadd(image)\nwait(1s)\nwidth=160pt\nwait(1s)"));
    Engine::new(Viewport {
        width: 320,
        height: 180,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &sources, &Default::default())
    .unwrap()
    .animation
}
#[test]
fn bitmap_and_svg_render_directly_and_share_layout_signals_and_frame_caches() {
    for (name, bytes) in [
        (
            "sample.png",
            include_bytes!("../../../tests/fixtures/media-sample.png").as_slice(),
        ),
        (
            "sample.svg",
            include_bytes!("../../../tests/fixtures/media-sample.svg").as_slice(),
        ),
    ] {
        let mut renderer = Renderer::default();
        let scene = renderer.prepare(scene(name, bytes)).unwrap();
        let options = Options::for_animation(scene.animation());
        let first = renderer.render(&scene, 0.into(), &options).unwrap();
        let pixel = |x: usize, y: usize| first.rgba[(y * 320 + x) * 4 + 3];
        assert_eq!(pixel(160, 90), 255);
        assert_eq!(pixel(90, 90), 0);
        if name.ends_with("png") {
            assert_eq!(pixel(102, 56), 0);
        }
        let later = renderer
            .frame_state(&scene, Fraction::from(1), &options)
            .unwrap();
        let Geometry::Media { width, height, .. } = later.elements()[0].geometry.as_ref() else {
            panic!("media")
        };
        assert_eq!((*width, *height), (160.into(), 96.into()));
        let replay = renderer.render(&scene, 0.into(), &options).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &replay));
    }
}
