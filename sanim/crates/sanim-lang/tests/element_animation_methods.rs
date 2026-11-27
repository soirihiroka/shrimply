use sanim_core::{Easing, Fraction, Property};
use sanim_lang::{Engine, MemorySources, Prepared};

const SHAPE: &str = "let shape=Box(width=20pt,height=10pt,fill=Color(\"blue3\"),x_align=Align.Center,y_align=Align.Center)\n";
fn run(source: &str) -> Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{}\nAnimation()", source));
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &sources, &Default::default())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
    prepared
}

#[test]
fn default_and_explicit_fades_have_one_typed_method_contract() {
    for call in [
        "shape.fade_in()",
        "shape.fade_in(300ms,\"smootherstep\")",
        "shape.fade_in(duration=300ms,ease=\"smootherstep\")",
    ] {
        let prepared = run(&format!("{SHAPE}{call}"));
        assert_eq!(prepared.animation.duration, Fraction::new(3u64, 10u64));
        let track = &prepared.animation.tracks["shape"][&Property::Opacity][0];
        assert_eq!(track.easing, Easing::Smootherstep);
        assert_eq!(
            prepared.animation.sample(0.into()).unwrap()[0].opacity,
            0.into()
        );
        assert_eq!(
            prepared
                .animation
                .sample(Fraction::new(3u64, 20u64))
                .unwrap()[0]
                .opacity,
            Fraction::new(1u64, 2u64)
        );
        assert_eq!(
            prepared
                .animation
                .sample(Fraction::new(3u64, 10u64))
                .unwrap()[0]
                .opacity,
            1.into()
        );
    }
}

#[test]
fn every_animation_is_element_owned_with_safe_property_defaults() {
    for (call, property, easing) in [
        ("move_to([20pt, 0pt])", Property::Position, Easing::Spring),
        ("shift([20pt, 0pt])", Property::Position, Easing::Spring),
        ("rotate_to(30deg)", Property::Rotation, Easing::Spring),
        ("scale_to(2)", Property::Scale, Easing::Spring),
        (
            "fill_to(Color(\"orange3\"))",
            Property::Fill,
            Easing::Smootherstep,
        ),
        ("fade_in()", Property::Opacity, Easing::Smootherstep),
        ("fade_out()", Property::Opacity, Easing::Smootherstep),
        ("create()", Property::Reveal, Easing::Smootherstep),
        ("uncreate()", Property::Reveal, Easing::Smootherstep),
        ("write()", Property::Reveal, Easing::Smootherstep),
        ("unwrite()", Property::Reveal, Easing::Smootherstep),
        ("grow_from_center()", Property::Scale, Easing::Spring),
        ("shrink_to_center()", Property::Scale, Easing::Spring),
        ("expand_x()", Property::StretchX, Easing::Spring),
        ("stretch_x_to(2)", Property::StretchX, Easing::Spring),
    ] {
        let prepared = run(&format!("{SHAPE}shape.{call}"));
        assert_eq!(
            prepared.animation.duration,
            Fraction::new(3u64, 10u64),
            "{call}"
        );
        assert_eq!(
            prepared.animation.tracks["shape"][&property][0].easing, easing,
            "{call}"
        );
    }
}

#[test]
fn batch_receivers_share_the_same_parallel_plan() {
    let source = "let a=Box(width=20pt,height=20pt)\nlet b=Box(width=20pt,height=20pt)\n<Animation>{[a,b].fade_in(1s,\"linear\")}{wait(500ms)}</Animation>;\n[a,b].fade_out()";
    let prepared = run(source);
    assert_eq!(prepared.animation.duration, Fraction::new(13u64, 10u64));
    assert_eq!(
        prepared.animation.tracks["a"][&Property::Opacity],
        prepared.animation.tracks["b"][&Property::Opacity]
    );
}

#[test]
fn horizontal_methods_accept_timing_before_named_or_positional_anchor_controls() {
    for call in [
        "shape.expand_x(400ms,\"linear\",origin=25%,offset=2pt)",
        "shape.expand_x(400ms,\"linear\",25%,2pt)",
        "shape.stretch_x_to(2,400ms,\"linear\",origin=25%,offset=2pt)",
    ] {
        let prepared = run(&format!("{SHAPE}{call}"));
        let track = &prepared.animation.tracks["shape"][&Property::StretchX][0];
        assert_eq!(prepared.animation.duration, Fraction::new(2u64, 5u64));
        assert_eq!(track.easing, Easing::Linear);
        assert_eq!(
            track.horizontal.unwrap().origin.value,
            Fraction::new(1u64, 4u64)
        );
        assert_eq!(track.horizontal.unwrap().offset, Fraction::from(2));
    }
}

#[test]
fn methods_and_scoped_factories_are_pure_and_validate_receivers() {
    for (source, code) in [
        ("let shape=Box()\nFadeIn(shape)", "S106"),
        ("let shape=Box()\nMoveTo(shape,[1pt, 0pt])", "S106"),
        ("[1,2].fade_in()", "S123"),
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", format!("{}\nAnimation()", source));
        let error = Engine::default()
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err();
        assert_eq!(error.code, code, "{source}: {error}");
    }
    let prepared = run(
        "fn fade(shape:Element):Animation{return shape.fade_in()}\nlet shape=Box()\nfade(shape)",
    );
    assert!(
        prepared.symbols["fade"].signatures()[0]
            .capabilities
            .is_pure()
    );
    for name in [
        "MoveTo",
        "Shift",
        "FadeIn",
        "FadeOut",
        "Create",
        "Write",
        "Uncreate",
        "Unwrite",
        "GrowFromCenter",
        "ShrinkToCenter",
        "ExpandX",
        "StretchXTo",
        "ScaleTo",
        "RotateTo",
        "FillTo",
    ] {
        let factory = sanim_lang::prelude::builtin(name).expect("scoped animation factory");
        assert_eq!(factory.result, sanim_core::Type::Animation, "{name}");
        assert!(factory.capabilities().is_pure(), "{name}");
    }
}

#[test]
#[ignore = "performance measurement: make element-animation-bench"]
fn method_reload_benchmark() {
    const EDITS: usize = 80;
    let prefix = "let shape=Box(width=20pt,height=20pt)\nlet reveal=shape.fade_in(1s)\n<Animation>{reveal}{wait(500ms)}</Animation>\n";
    let mut engine = Engine::default();
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{prefix}wait(1ms)"));
    let cold = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let mut samples = Vec::new();
    let mut bytes = (usize::MAX, 0usize);
    for edit in 0..EDITS {
        sources.insert("main.sanim", format!("{prefix}wait({}ms)", edit + 2));
        let started = std::time::Instant::now();
        let prepared = engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        samples.push(started.elapsed().as_micros());
        assert!(prepared.statistics.restored_statements >= 3);
        assert!(prepared.statistics.operations < cold.statistics.operations);
        if edit >= EDITS / 2 {
            bytes.0 = bytes.0.min(prepared.statistics.checkpoint_bytes);
            bytes.1 = bytes.1.max(prepared.statistics.checkpoint_bytes);
        }
    }
    samples.sort_unstable();
    assert!(
        bytes.1 - bytes.0 < 4096,
        "suffix edits must not accumulate method state: {bytes:?}"
    );
    println!(
        "element method reload median_us={} p95_us={} checkpoint_bytes={}..{}",
        samples[EDITS / 2],
        samples[EDITS * 95 / 100],
        bytes.0,
        bytes.1
    );
}

#[test]
fn receiver_contract_supports_bounded_generics_and_target_unions() {
    for declaration in [
        "fn show<T:Element>(target:T):Animation{return target.fade_in()}",
        "fn show<T:Element>(target:List<T>):Animation{return target.fade_in()}",
        "fn show(target:Element|List<Element>):Animation{return target.fade_in()}",
    ] {
        let target = if declaration.contains("target:List") {
            "[shape]"
        } else {
            "shape"
        };
        let prepared = run(&format!("{SHAPE}{declaration}\nshow({target})"));
        assert_eq!(prepared.animation.duration, Fraction::new(3u64, 10u64));
    }
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let items:List<Element>=[]\nitems.fade_in()\nAnimation()",
    );
    let error = Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("target list is empty"), "{error}");
}

#[test]
fn generic_literal_inference_preserves_bounds_homogeneity_and_return_contracts() {
    for source in [
        "fn show<T:Element>(target:List<T>):Animation{return target.fade_in()}\nshow([1])",
        "let shape=Box()\nfn show<T:Element>(target:List<T>):Animation{return target.fade_in()}\nshow([shape,1])",
        "fn wrong<T>(value:T):List<T>{return [1]}\nlet result=wrong(\"text\")",
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", format!("{}\nAnimation()", source));
        let error = Engine::default()
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap_err();
        assert!(
            matches!(error.code.as_str(), "S105" | "S106" | "S126"),
            "{source}: {error}"
        );
    }
    run(
        "fn count<T>(values:List<{value:T,kind:\"before\"|\"after\"}>):Number{return len(values)}\nlet shape=Box()\nassert(count([{value=shape,kind=\"before\"}])==1)",
    );
}

#[test]
fn module_functions_named_like_animation_methods_keep_their_own_contract() {
    let mut sources = MemorySources::default();
    sources.insert("lib.sanim", "fn fade_in():Number{return 42}");
    sources.insert(
        "main.sanim",
        "import \"lib\" as lib\nassert(lib.fade_in()==42)\nAnimation()",
    );
    Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let element = sanim_lang::prelude::methods_for(&sanim_core::Type::Element)
        .find(|builtin| builtin.name == "Element.fade_in")
        .unwrap();
    let list = sanim_lang::prelude::methods_for(&sanim_core::Type::List(
        Box::new(sanim_core::Type::Element),
        sanim_core::ListLength::Unknown,
    ))
    .find(|builtin| builtin.name == element.name)
    .unwrap();
    assert!(
        std::ptr::eq(element, list),
        "receiver kinds must share one native declaration"
    );
    assert_eq!(element.timing.default, Fraction::new(3u64, 10u64));
    assert_eq!(element.timing.easing, Easing::Smootherstep);
}
