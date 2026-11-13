use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};

#[test]
fn batched_curves_render_and_reveal_without_closing_the_path() {
    let mut files = MemorySources::default();
    files.insert("main.sanim",r#"
bg(Color("transparent"))
let p=Polyline(points=[Vec2(-30pt,20pt),Vec2(0pt,-20pt),Vec2(30pt,20pt)],width=3pt,fill=Color("blue3"))
Create(p,duration=1s,ease="linear")
wait(1s)
"#);
    let prepared = Engine::new(Viewport {
        width: 100,
        height: 100,
        dpi: 72,
        ..Viewport::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap();
    assert!(matches!(
        prepared.animation.layers[0].element.geometry.as_ref(),
        Geometry::Polyline { .. }
    ));
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let half = renderer
        .render(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    let full = renderer.render(&scene, 1.into(), &options).unwrap();
    let pixels = |image: &[u8]| image.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
    assert!(pixels(&full.rgba) > pixels(&half.rgba));
    // Closing this triangle would incorrectly paint its horizontal bottom edge.
    assert_eq!(full.rgba[(70 * 100 + 50) * 4 + 3], 0);
    assert!(full.rgba[(30 * 100 + 50) * 4 + 3] > 0);
    assert_eq!(renderer.geometries_prepared, 1);
}
