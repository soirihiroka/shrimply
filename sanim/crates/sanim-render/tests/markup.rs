use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
fn animation(source: &str, width: u32, height: u32) -> sanim_core::Animation {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::new(Viewport {
        width,
        height,
        dpi: 72,
        ..Viewport::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation
}
fn frame(source: &str, width: u32, height: u32) -> std::sync::Arc<sanim_render::Frame> {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(source, width, height)).unwrap();
    renderer
        .render(&scene, 0.into(), &Options::for_animation(scene.animation()))
        .unwrap()
}
fn pixel(frame: &sanim_render::Frame, x: usize, y: usize) -> &[u8] {
    &frame.rgba[(y * frame.width as usize + x) * 4..(y * frame.width as usize + x) * 4 + 4]
}
#[test]
fn markup_gradients_rounding_borders_and_labels_render_natively() {
    let source = r#"bg(Color("transparent"))
let card=<Rect width={100pt} height={50pt} design="pill border-2 border-light1" fill={LinearGradient(from=Color("blue3"),to=Color("orange3"))}><Label font_size={12pt}>native</Label></Rect>
add(card)
wait(1s)"#;
    let image = frame(source, 160, 100);
    assert_eq!(pixel(&image, 31, 26)[3], 0);
    assert!(pixel(&image, 45, 50)[2] > pixel(&image, 45, 50)[0]);
    assert!(pixel(&image, 115, 50)[0] > pixel(&image, 115, 50)[2]);
    assert!(pixel(&image, 80, 25)[3] > 0);
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(source, 160, 100)).unwrap();
    renderer
        .record(&scene, 0.into(), &Options::for_animation(scene.animation()))
        .unwrap();
    assert_eq!(
        renderer.geometries_prepared, 2,
        "one rounded surface and one shaped label"
    );
}
#[test]
fn relative_sizes_resolve_under_external_resizing_and_pills_remain_round() {
    let source = r#"bg(Color("transparent"))
let card=<Rect design="w-50% h-50% pill bg-blue3"/>
add(card)
wait(1s)"#;
    let small = frame(source, 200, 100);
    let large = frame(source, 400, 200);
    assert_eq!(pixel(&small, 51, 26)[3], 0);
    assert_eq!(pixel(&large, 101, 51)[3], 0);
    assert!(pixel(&small, 100, 50)[3] > 0);
    assert!(pixel(&large, 200, 100)[3] > 0);
    let animation = animation(source, 200, 100);
    let laid_out = animation.layout(0.into(), 400.into(), 200.into()).unwrap();
    let Geometry::Rectangle {
        width,
        height,
        radius,
        ..
    } = laid_out[0].geometry.as_ref()
    else {
        panic!("Rect")
    };
    assert_eq!((*width, *height), (200.into(), 100.into()));
    assert_eq!(radius.resolve(*width, *height), 50.into());
}
#[test]
fn native_content_layout_and_container_opacity_are_shared() {
    let source = r#"bg(Color("transparent"))
let card=<Rect opacity={0.5} design="w-160 h-70 rounded-lg row items-center justify-between p-10 bg-blue3"><Circle radius={8pt} fill="orange3"/><Label font_size={16pt}>native</Label></Rect>
add(card)
wait(1s)"#;
    let animation = animation(source, 240, 140);
    let layout = animation
        .layout(Fraction::from(0), 240.into(), 140.into())
        .unwrap();
    let children = layout[0].geometry.children().unwrap();
    assert_eq!(children.len(), 2);
    assert!(children[0].position.values[0] < children[1].position.values[0]);
    let image = frame(source, 240, 140);
    let maximum = image
        .rgba
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .max()
        .unwrap();
    assert!(
        (126..=129).contains(&maximum),
        "container opacity must composite once, got alpha {maximum}"
    );
}
#[test]
fn percentage_signal_sizes_resolve_after_sampling_at_each_viewport() {
    let source = r#"mut width=25%
let shape=<Rect width={width} height={40pt} design="pill bg-blue3"/>
add(shape)
animate(width,to=75%,duration=1s,ease="linear")"#;
    let animation = animation(source, 200, 100);
    for (time, viewport, expected) in [
        (Fraction::from(0), 200, 50),
        (Fraction::new(1u64, 2u64), 200, 100),
        (Fraction::new(1u64, 2u64), 400, 200),
    ] {
        let laid = animation
            .layout(time, Fraction::from(viewport as u32), 100.into())
            .unwrap();
        let Geometry::Rectangle { width, .. } = laid[0].geometry.as_ref() else {
            panic!("Rect")
        };
        assert_eq!(*width, Fraction::from(expected as u32));
    }
}
#[test]
fn borders_are_included_in_scaled_verification_and_flow_extents() {
    let animation = animation(
        "let shape=<Rect width={20pt} height={10pt} scale={2} design=\"border-4\"/>\nadd(shape)\nwait(1s)",
        100,
        100,
    );
    let laid = animation.layout(0.into(), 100.into(), 100.into()).unwrap();
    let bounds = sanim_core::verify::bounds(&laid[0]).unwrap();
    assert_eq!(bounds.min.values, [26., 36.]);
    assert_eq!(bounds.max.values, [74., 64.]);
    assert_eq!(sanim_core::layout::extent(&laid[0]).unwrap(), (48., 28.));
}
