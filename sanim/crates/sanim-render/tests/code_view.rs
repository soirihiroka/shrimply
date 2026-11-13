use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources, Prepared};
use sanim_render::{Options, Renderer};

fn prepare(source: &str) -> Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files.insert("snippet.py", "value = 1\nreturn value\n");
    files.insert("large.txt", "word ".repeat(1200));
    Engine::new(Viewport {
        width: 360,
        height: 220,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
}

#[test]
fn native_code_glyph_picking_matches_rendered_coverage_and_constructor_locations() {
    let prepared = prepare(
        r#"bg(Color("transparent"))
let view=CodeView(src="snippet.py",font_size=22pt,line_numbers=false,rotation=17deg,position=Vec2(-30pt,10pt))
add(view)
wait(1s)"#,
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation.clone()).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    let frame = renderer.render(&scene, 0.into(), &options).unwrap();
    let states = renderer.states_computed;
    let mut glyph_samples = 0;
    for y in (0..frame.height).step_by(3) {
        for x in (0..frame.width).step_by(3) {
            let alpha = frame.rgba[((y * frame.width + x) * 4 + 3) as usize];
            if alpha > 0 && alpha < 250 {
                continue;
            }
            let hit = renderer
                .pick(
                    &scene,
                    0.into(),
                    &options,
                    [x as f32 + 0.5, y as f32 + 0.5],
                    0.,
                )
                .unwrap();
            assert_eq!(hit.is_some(), alpha > 0, "glyph coverage at {x},{y}");
            if let Some(hit) = hit {
                glyph_samples += 1;
                assert_eq!(hit.kind, "Text");
                assert!(hit.ancestors.iter().any(|id| id == "view"));
                let origin = prepared
                    .element_sources
                    .get(&hit.id)
                    .or_else(|| {
                        hit.ancestors
                            .iter()
                            .rev()
                            .find_map(|id| prepared.element_sources.get(id))
                    })
                    .expect("lazy native glyphs retain their CodeView constructor owner");
                assert_eq!(origin.creation.span.line, 2);
            }
        }
    }
    assert!(glyph_samples > 0);
    assert_eq!(
        renderer.states_computed, states,
        "all picks reuse the displayed frame state"
    );
}

#[test]
fn dynamic_code_style_uses_actual_color_interpolation_and_caches_reverse_frames() {
    let prepared = prepare(
        r#"bg(Color("transparent"))
let view=CodeView(code=r"value = 1",lang="python",font_size=22pt,line_numbers=false)
add(view)
wait(1s)
view.style(spans=[CodeSpan(CodeRange(1,1,1,6),CodeStyle(fill=Color("orange3"),background=Color("blue5"),underline=true))],duration=1s,ease="linear")
wait(1s)"#,
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    let initial = renderer.render(&scene, 1.into(), &options).unwrap();
    let middle = renderer
        .render(&scene, Fraction::new(3u64, 2u64), &options)
        .unwrap();
    let styled = renderer.render(&scene, 2.into(), &options).unwrap();
    assert_ne!(
        initial.rgba, middle.rgba,
        "style begins changing before the endpoint"
    );
    assert_ne!(
        middle.rgba, styled.rgba,
        "style does not jump to the endpoint"
    );
    assert_ne!(initial.rgba, styled.rgba);
    let computed = renderer.states_computed;
    let paths = renderer.geometries_prepared;
    assert_eq!(
        renderer.render(&scene, 1.into(), &options).unwrap().rgba,
        initial.rgba
    );
    assert_eq!(renderer.states_computed, computed);
    assert_eq!(renderer.geometries_prepared, paths);
}

#[test]
fn many_growing_code_views_fail_the_frame_budget_without_discarding_a_good_frame() {
    let prepared = prepare(
        r#"mut source=""
for index in range(64) {
    let view=CodeView(code=source,lang="plain")
    add(view)
}
wait(1s)
source=read<String>("large.txt")
wait(1s)"#,
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let good = renderer.record(&scene, 0.into(), &options).unwrap();
    let error = match renderer.record(&scene, 1.into(), &options) {
        Ok(_) => panic!("expanding views must stop at the generated-frame memory budget"),
        Err(error) => error,
    };
    assert!(error.contains("native frame geometry"), "{error}");
    assert!(std::sync::Arc::ptr_eq(
        &good,
        &renderer.record(&scene, 0.into(), &options).unwrap()
    ));
}
