use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
const SOURCE: &str = r#"bg(Color("transparent"))
let first=Rect(width=20pt,height=20pt,fill=Color("blue3"),x_align=Align.Start,y_align=Align.Start)
let last=Rect(width=20pt,height=20pt,position=Vec2(0pt,80pt),fill=Color("orange3"),x_align=Align.Start,y_align=Align.Start)
let spacer=Box(width=20pt,height=3*20pt)
let clip=Box(overflow="auto",width=100pt,height=60pt,children=[first,spacer,last])
add(clip)
clip.scroll_to(to=Vec2(0pt,40pt),duration=1s,ease="linear")
wait(1s)
"#;
fn animation(source: &str) -> sanim_core::Animation {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::new(Viewport {
        width: 200,
        height: 120,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation
}
#[test]
fn scrolling_clips_pixels_and_picks_using_the_same_native_transforms() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(SOURCE)).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    let before = renderer.render(&scene, 0.into(), &options).unwrap();
    let after = renderer.render(&scene, 1.into(), &options).unwrap();
    let alpha =
        |frame: &sanim_render::Frame, x, y| frame.rgba[(y * frame.width as usize + x) * 4 + 3];
    assert_eq!(alpha(&before, 10, 10), 255);
    assert_eq!(alpha(&before, 10, 90), 0);
    assert_eq!(alpha(&after, 10, 10), 0);
    assert_eq!(alpha(&after, 10, 50), 255);
    assert_eq!(alpha(&after, 10, 70), 0);
    assert_eq!(
        renderer
            .pick(&scene, 1.into(), &options, [10., 50.], 0.)
            .unwrap()
            .unwrap()
            .id,
        "last"
    );
    assert!(
        renderer
            .pick(&scene, 1.into(), &options, [10., 70.], 0.)
            .unwrap()
            .is_none()
    );
    let count = renderer.states_computed;
    assert_eq!(
        renderer.render(&scene, 0.into(), &options).unwrap().rgba,
        before.rgba
    );
    assert_eq!(renderer.states_computed, count);
}

#[test]
fn offscreen_code_glyphs_do_not_prepare_native_paths() {
    let text: String = (0..100)
        .map(|index| format!("value_{index} = {index}\n"))
        .collect();
    let script = format!(
        "let view=CodeView(code=r\"\"\"{text}\"\"\",lang=\"python\",width=140pt,height=60pt,font_size=12pt,line_height=18pt,line_numbers=false)\nadd(view)\nview.scroll_to(to=Vec2(0%,100%),duration=1s,ease=\"linear\")\nwait(1s)\n"
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(&script)).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    assert!(
        renderer.geometries_prepared < 30,
        "only the visible code rows should build Skia paths; got {}",
        renderer.geometries_prepared
    );
    let first = renderer.geometries_prepared;
    renderer.record(&scene, 1.into(), &options).unwrap();
    assert!(
        renderer.geometries_prepared > first,
        "scrolling reveals previously deferred native glyph paths"
    );
    assert!(renderer.geometries_prepared < 60);
}

#[test]
fn percent_axis_targets_remain_relative_after_external_resize() {
    let source = "let content=Box(width=20pt,height=420pt)\nlet view=Box(overflow=\"auto\",width=100pt,height=50%,children=[content])\nadd(view)\nview.scroll_to(y=100%,duration=1s,ease=\"linear\")\nwait(1s)";
    let mut renderer = Renderer::default();
    let axis = renderer.prepare(animation(source)).unwrap();
    let vector = renderer
        .prepare(animation(&source.replace("y=100%", "to=Vec2(0%,100%)")))
        .unwrap();
    for height in [120, 240, 80] {
        let mut options = Options::for_animation(axis.animation());
        options.logical_height = height.into();
        let axis_state = renderer.frame_state(&axis, 1.into(), &options).unwrap();
        let vector_state = renderer.frame_state(&vector, 1.into(), &options).unwrap();
        let view = &axis_state.elements()[0];
        let offset = view.layout.scroll.unwrap();
        let maximum = sanim_core::scrolling::range(view).unwrap();
        assert_eq!(offset.values[1], maximum[1]);
        assert_eq!(offset, vector_state.elements()[0].layout.scroll.unwrap());
    }
}

#[test]
fn scroll_prefix_states_and_pictures_survive_a_suffix_reload() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", SOURCE);
    let mut engine = Engine::new(Viewport {
        width: 200,
        height: 120,
        dpi: 72,
        ..Default::default()
    });
    let before = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(before.animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let time = Fraction::new(1u64, 2u64);
    let picture = renderer.record(&scene, time, &options).unwrap();
    let computed = renderer.states_computed;
    files.insert("main.sanim", SOURCE.replace("wait(1s)", "wait(2s)"));
    let changed = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert!(changed.statistics.restored_statements > 0);
    let next = renderer.prepare(changed.animation).unwrap();
    let replay = renderer.record(&next, time, &options).unwrap();
    assert_eq!(renderer.states_computed, computed);
    assert!(std::sync::Arc::ptr_eq(&picture, &replay));
}
#[test]
#[ignore = "manual scrolling benchmark"]
fn scroll_benchmark() {
    use std::{hint::black_box, time::Instant};
    let source: String = (0..1000)
        .map(|index| format!("value_{index} = {index}\n"))
        .collect();
    let code = format!(
        "let view=CodeView(code=r\"\"\"{source}\"\"\",lang=\"python\",width=140pt,height=60pt,font_size=12pt,line_height=18pt,line_numbers=false)\nadd(view)\nview.scroll_to(to=Vec2(0%,100%),duration=1s,ease=\"linear\")\nwait(1s)\n"
    );
    for (name, source) in [("shapes", SOURCE), ("1000 code rows", code.as_str())] {
        let mut renderer = Renderer::default();
        let scene = renderer.prepare(animation(source)).unwrap();
        let options = Options::for_animation(scene.animation());
        let started = Instant::now();
        for index in 0..60u64 {
            black_box(
                renderer
                    .record(&scene, Fraction::new(index, 60u64), &options)
                    .unwrap(),
            );
        }
        let cold = started.elapsed();
        let paths = renderer.geometries_prepared;
        let started = Instant::now();
        for index in (0..60u64).rev() {
            black_box(
                renderer
                    .record(&scene, Fraction::new(index, 60u64), &options)
                    .unwrap(),
            );
        }
        let warm = started.elapsed();
        assert_eq!(renderer.geometries_prepared, paths);
        eprintln!(
            "native scroll {name}, 60 frames: cold={cold:?}, cached reverse={warm:?}, paths={paths}"
        );
    }
}

#[test]
fn a_zero_height_hidden_box_clips_every_child_pixel() {
    let mut renderer=Renderer::default();
    let scene=renderer.prepare(animation(r#"bg(Color("transparent"))
let pane=<Box w-100 h-0 overflow-hidden><Rect width={20pt} height={20pt} fill="blue3"/></Box>
add(pane)
wait(1s)"#)).unwrap();
    let mut options=Options::for_animation(scene.animation());
    options.encode_png=false;
    let frame=renderer.render(&scene,0.into(),&options).unwrap();
    assert!(frame.rgba.chunks_exact(4).all(|pixel|pixel[3]==0));
}
