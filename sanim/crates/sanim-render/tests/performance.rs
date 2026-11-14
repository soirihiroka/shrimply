//! Deterministic performance contracts: count work and prove shared ownership.
use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::sync::Arc;

#[test]
fn folding_and_line_number_replay_reuses_states_recordings_and_syntax() {
    let text = (0..24)
        .map(|index| format!("def task_{index}():\n    return {index}\n\n"))
        .collect::<String>();
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        format!(
            r#"mut numbers=true
let code=CodeView(code=r"""{text}""",lang="python",line_numbers=numbers,width=220pt,font_size=12pt)
let pane=<Box w-240 h-100 overflow-y-scroll>{{code}}</Box>
add(pane)
wait(1s)
code.range(name="task_0").fold(duration=1s,ease="linear")
wait(1s)
numbers=false
wait(1s)
code.range(name="task_0").unfold(duration=1s,ease="linear")
wait(1s)
numbers=true
wait(1s)
"#
        ),
    );
    let animation = Engine::new(Viewport {
        width: 320,
        height: 160,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation;
    let Geometry::Code { document, .. } = animation.find("code").unwrap().geometry.as_ref() else {
        panic!("CodeView")
    };
    let original = document.clone();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let times = [
        Fraction::from(0),
        Fraction::new(3u64, 2u64),
        2.into(),
        3.into(),
        Fraction::new(9u64, 2u64),
        5.into(),
        6.into(),
    ];
    let mut cached = Vec::new();
    for time in times {
        let state = renderer.frame_state(&scene, time, &options).unwrap();
        let code = state.elements()[0].find("code").unwrap();
        let Geometry::Code {
            document,
            options: code_options,
            ..
        } = code.geometry.as_ref()
        else {
            panic!("CodeView")
        };
        assert!(
            original.syntax.shares_tree(&document.syntax),
            "presentation changes must not parse another syntax tree"
        );
        assert_eq!(
            code_options.line_numbers,
            time < Fraction::from(3) || time >= Fraction::from(6)
        );
        let picture = renderer.record(&scene, time, &options).unwrap();
        cached.push((time, state, picture));
    }
    let computed = renderer.states_computed;
    let paths = renderer.geometries_prepared;
    let identity_lookups = renderer.geometry_key_hits;
    let recording_hits = renderer.cache_hits;
    for _ in 0..3 {
        for (time, state, picture) in cached.iter().rev() {
            assert!(Arc::ptr_eq(
                state,
                &renderer.frame_state(&scene, *time, &options).unwrap()
            ));
            assert!(Arc::ptr_eq(
                picture,
                &renderer.record(&scene, *time, &options).unwrap()
            ));
        }
    }
    assert_eq!(
        renderer.states_computed, computed,
        "cached replay must do zero layout/state evaluation"
    );
    assert_eq!(
        renderer.geometries_prepared, paths,
        "cached replay must build zero paths"
    );
    assert_eq!(
        renderer.geometry_key_hits, identity_lookups,
        "recording hits must skip geometry traversal entirely"
    );
    assert_eq!(
        renderer.cache_hits - recording_hits,
        (cached.len() * 3) as u64
    );
}

#[test]
fn suffix_edits_keep_prefix_recordings_and_only_build_changed_geometry() {
    const ELEMENTS: usize = 256;
    let circles = (0..ELEMENTS)
        .map(|_| "Circle(radius=3pt)")
        .collect::<Vec<_>>()
        .join(",");
    let source = |width, color| {
        format!(
            "let prefix=<Box w-600 h-240 flex flex-row flex-wrap children={{[{circles}]}} />\nadd(prefix)\nwait(1s)\nlet late=Rect(width={width}pt,height=8pt,fill=Color(\"{color}\"))\nadd(late)\nwait(2s)\n"
        )
    };
    let mut files = MemorySources::default();
    let mut engine = Engine::new(Viewport {
        width: 640,
        height: 360,
        dpi: 72,
        ..Default::default()
    });
    files.insert("main.sanim", source(20, "blue3"));
    let first = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let cold_work = first.statistics.operations;
    let mut renderer = Renderer::default();
    let first = renderer.prepare(first.animation).unwrap();
    let options = Options::for_animation(first.animation());
    let prefix = renderer.record(&first, 0.into(), &options).unwrap();
    renderer.record(&first, 2.into(), &options).unwrap();
    let paths = renderer.geometries_prepared;
    assert_eq!(paths, 2, "one shared circle path and one late rectangle");
    for (width, color, expected_paths) in [(21, "blue3", paths + 1), (21, "orange3", paths + 1)] {
        files.insert("main.sanim", source(width, color));
        let next = engine
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        assert!(next.statistics.restored_statements >= 3);
        assert!(
            next.statistics.operations <= 32,
            "only the three-statement suffix may execute: {:?}",
            next.statistics
        );
        assert!(
            cold_work > next.statistics.operations * 10,
            "the fixture must exercise a substantial cached prefix"
        );
        assert_eq!(next.statistics.decoded_assets, 0);
        let before_paths = renderer.geometries_prepared;
        let scene = renderer.prepare(next.animation).unwrap();
        assert!(Arc::ptr_eq(
            &prefix,
            &renderer.record(&scene, 0.into(), &options).unwrap()
        ));
        assert_eq!(
            renderer.geometries_prepared, before_paths,
            "an earlier frame must not prepare changed future geometry"
        );
        renderer.record(&scene, 2.into(), &options).unwrap();
        assert_eq!(
            renderer.geometries_prepared, expected_paths,
            "a style edit must not rebuild geometry; a dimension edit builds exactly one path"
        );
    }
}
