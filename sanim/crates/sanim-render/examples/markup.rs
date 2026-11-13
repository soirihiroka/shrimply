//! Compare element markup with its ordinary constructor form, using identical native IR.
use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::{fmt::Write, time::Instant};
fn source(markup: bool) -> String {
    let mut source =
        String::from("let gradient=LinearGradient(from=Color(\"blue3\"),to=Color(\"purple3\"))\n");
    let mut names = Vec::new();
    for i in 0..100 {
        let x = (i % 10) * 66 - 297;
        let y = (i / 10) * 34 - 153;
        names.push(format!("tile_{i}"));
        if markup {
            writeln!(source,r#"let tile_{i}=<Rect id="tile-{i}" width={{56pt}} height={{28pt}} position={{Vec2({x}pt,{y}pt)}} pill fill={{gradient}}><Label id="label-{i}" font_size={{10pt}}>item {i}</Label></Rect>"#).unwrap();
        } else {
            writeln!(source,r#"let tile_{i}=Rect(id="tile-{i}",width=56pt,height=28pt,position=Vec2({x}pt,{y}pt),pill,fill=gradient,children=[Text("item {i}",id="label-{i}",font_size=10pt)])"#).unwrap();
        }
    }
    writeln!(source,"let group=Box(children=[{}])\nMoveTo(group,to=Vec2(10pt,0pt),duration=1s,ease=\"linear\")",names.join(",")).unwrap();
    source
}
fn prepare(markup: bool) -> sanim_lang::Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source(markup));
    Engine::new(Viewport {
        width: 960,
        height: 540,
        dpi: 72,
        ..Viewport::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
}
fn record(animation: sanim_core::Animation) -> u128 {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let start = Instant::now();
    for i in 0u64..30 {
        renderer
            .record(&scene, Fraction::new(i, 30u64), &options)
            .unwrap();
    }
    start.elapsed().as_micros()
}
fn main() {
    let start = Instant::now();
    let native = prepare(false);
    let native_prepare = start.elapsed().as_micros();
    let start = Instant::now();
    let markup = prepare(true);
    let markup_prepare = start.elapsed().as_micros();
    assert_eq!(
        native.animation, markup.animation,
        "markup must lower to the same native render/state IR"
    );
    let mut native_times = Vec::new();
    let mut markup_times = Vec::new();
    for pass in 0..5 {
        if pass % 2 == 0 {
            native_times.push(record(native.animation.clone()));
            markup_times.push(record(markup.animation.clone()));
        } else {
            markup_times.push(record(markup.animation.clone()));
            native_times.push(record(native.animation.clone()));
        }
    }
    native_times.sort_unstable();
    markup_times.sort_unstable();
    println!(
        "elements=100 frames=30 identical_ir=true native_prepare_us={native_prepare} markup_prepare_us={markup_prepare} native_record_median_us={} markup_record_median_us={} frame_ratio={:.3}",
        native_times[2],
        markup_times[2],
        markup_times[2] as f64 / native_times[2] as f64
    );
}
