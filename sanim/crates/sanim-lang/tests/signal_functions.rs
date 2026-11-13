use sanim_core::{Fraction, Geometry, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;
fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap()
}
#[test]
fn live_parameters_and_returns_keep_updates_while_ordinary_parameters_snapshot() {
    prepare(
        r#"
fn double(sig value:Number):sig Number {return value*2}
fn saved(value:Number):sig Number {return value*2}
mut count=1
let live=double(count)
let frozen=saved(count)
let snapshot=live
assert(live==2)
count=2
assert(live==4)
assert(frozen==2)
assert(snapshot==2)
assert(double(count)+1==5)
sig derived=double(count)
count=3
assert(derived==6)
let items=[double(count)]
let row={value=double(count)}
mut writable=double(count)
writable=7
assert(unwrap(items[0],0)==6)
assert(row.value==6)
assert(writable==7)
"#,
    );
}
#[test]
fn escaping_local_cells_and_chained_sig_returns_remain_owned() {
    prepare(
        r#"
fn double(sig value:Number):sig Number {return value*2}
fn local(sig input:Number):sig Number {
    let offset=3
    sig result=input+offset
    return double(result)
}
mut source=1
let output=local(source)
assert(output==8)
source=4
assert(output==14)
fn own():sig Number {mut owned=7;return owned*2}
let owned=own()
assert(owned==14)
"#,
    );
}
#[test]
fn constants_defaults_generics_and_local_mut_parameters_have_explicit_modes() {
    prepare(
        r#"
fn double<T:Numeric>(sig value:T):sig T {return value*2}
fn defaulted(sig value:Number=3):sig Number {return value+1}
fn changed(mut value:Number):Number {value=value+2;return value}
assert(double(3)==6)
assert(defaulted()==4)
mut original=2
assert(changed(original)==4)
assert(original==2)
"#,
    );
}
#[test]
fn direct_signal_calls_work_in_scalar_control_flow_and_timeline_inputs() {
    let source = r#"
fn truth():sig Bool {return true}
fn count():sig Number {return 2}
fn duration_value():sig Duration {return 100ms}
mut total=0
if truth() {total=total+1}
repeat count() {total=total+1}
wait(duration_value())
assert(total==3)
let items=[9,10]
fn index_value():sig Number{return 0}
assert(unwrap(items[index_value()],0)==9)
fn iterable():sig List<Number>{return [1,2]}
for item in iterable() {total=total+item}
assert(total==6)
fn tint():sig Color{return Color("blue3")}
let default_tint=reflect(tint())
assert(default_tint==Color("blue3"))
"#;
    let prepared = prepare(source);
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let report = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::new(1u64, 10u64));
    assert_eq!(report.duration, prepared.animation.duration);
}
#[test]
fn signal_shape_helpers_lower_once_and_reverse_seek_without_interpretation() {
    let p = prepare(
        r#"
fn double(sig value:Length):sig Length {return value*2}
fn shape(sig size:Length):Element {return Circle(radius=double(size))}
mut size=10pt
let marker=shape(size)
add(marker)
animate(size,to=20pt,duration=1s,ease="linear")
"#,
    );
    for (time, expected) in [
        (Fraction::new(1u64, 2u64), 30),
        (0.into(), 20),
        (1.into(), 40),
    ] {
        let values = p.animation.layout(time, 1920.into(), 1080.into()).unwrap();
        let Geometry::Circle { radius } = values[0].geometry.as_ref() else {
            panic!("circle")
        };
        assert_eq!(*radius, Fraction::from(expected as u32));
    }
    assert!(p.animation.reactive.node_count() > 0);
}
#[test]
fn warm_value_and_checkpoint_caches_do_not_restore_stale_signal_handles() {
    let mut sources = MemorySources::default();
    let script = |value| {
        format!(
            "fn double(sig value:Number):sig Number {{return value*2}}\nmut input={value}\nlet output=double(input)\nassert(output=={expected})\nwait(1s)",
            expected = value * 2
        )
    };
    let mut engine = Engine::default();
    for value in [1, 3, 2, 4] {
        sources.insert("main.sanim", script(value));
        let output = engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            output.bindings["output"].number().unwrap().value,
            Fraction::from(value as u32 * 2)
        );
    }
}
#[test]
fn signal_modes_are_readonly_and_capture_metadata_is_bounded() {
    for (source, expected) in [
        (
            "fn change(sig input:Number):Number {input=2;return input}",
            "read-only",
        ),
        (
            "fn change(input:Number):Number {input=2;return input}",
            "immutable",
        ),
        ("fn bad():sig Number {return wait(1s)}", "expected"),
        ("sig n:sig Number=2", "already choose"),
        ("mut n:sig Number=2", "already choose"),
        ("type Boxed={value:sig Number}", "containers hold"),
        (
            "fn wrong(value:sig Number):Number{return value}",
            "before the parameter",
        ),
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(expected), "{error}");
    }
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        format!(
            "fn make(sig input:Number):sig Number{{return input{}}}\nmut x=1\nlet result=make(x)",
            "+1".repeat(30)
        ),
    );
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 2048;
    let error = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("signal metadata"), "{error}");
}

#[test]
fn demand_publication_exposes_values_or_deferred_metadata_and_never_runtime_handles() {
    let source = "fn double(sig value:Number):sig Number{return value*2}\nmut input=2\nlet output=double(input)\n";
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let mut engine = Engine::default();
    engine.preparation = sanim_lang::Preparation::Demand;
    let output = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(!output.bindings.contains_key("output"));
    assert!(
        output
            .deferred_bindings
            .iter()
            .any(|binding| binding.name == "output")
    );
    files.insert("main.sanim", format!("{source}assert(output==4)\n"));
    let output = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        output.bindings["output"].number().unwrap().value,
        Fraction::from(4)
    );
    assert!(
        !output
            .bindings
            .values()
            .any(|value| matches!(value, Value::Signal(_)))
    );
}

#[test]
fn impure_deferred_returns_and_arguments_fail_statically_and_after_hot_reload() {
    let source = "fn calculate():Number{return 1}\nfn factory():sig Number{return calculate()}\nlet output=factory()\nwait(1s)\n";
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let mut engine = Engine::default();
    engine.preparation = sanim_lang::Preparation::Demand;
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert(
        "main.sanim",
        source.replace("return 1", "wait(1s);return 1"),
    );
    let error = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S131", "{error}");
    for source in [
        "fn calculate():Number{wait(1s);return 1}\nfn consume(sig x:Number):Number{return x}\nlet output=consume(calculate())",
        "fn calculate():Number{wait(1s);return 1}\nfn consume(sig x:Number=calculate()):Number{return x}",
        "fn factory():sig Number{return reflect(1)}\nlet output=factory()",
    ] {
        files.insert("main.sanim", source);
        let error = Engine::default().lint("main.sanim", &files).unwrap_err();
        assert_eq!(
            error.code,
            if source.contains("reflect(") {
                "S113"
            } else {
                "S131"
            },
            "{error}"
        );
    }
    // Acquisition executes construction once; only the captured return expression
    // must be pure. No timeline effect is deferred into signal sampling.
    let p = prepare(
        "fn factory():sig Number{wait(100ms);return 2}\nlet output=factory()\nassert(output==2)\nassert(output==2)",
    );
    assert_eq!(p.animation.duration, Fraction::new(1u64, 10u64));
}

#[test]
fn ordinary_shape_arguments_and_snapshot_parameters_in_live_returns_stay_frozen() {
    let prepared = prepare(
        r#"
fn shape(size:Length):Element{return Circle(radius=size)}
fn snapshot(size:Length):sig Length{return size}
mut size=10pt
let marker=shape(size)
let other=Circle(radius=snapshot(size))
add(marker)
add(other)
animate(size,to=20pt,duration=1s,ease="linear")
"#,
    );
    for time in [
        Fraction::from(0),
        Fraction::new(1u64, 2u64),
        Fraction::from(1),
    ] {
        let elements = prepared
            .animation
            .layout(time, 1920.into(), 1080.into())
            .unwrap();
        for element in elements {
            let Geometry::Circle { radius } = element.geometry.as_ref() else {
                panic!("circle")
            };
            assert_eq!(*radius, Fraction::from(10));
        }
    }
}

#[test]
fn native_properties_acquire_signal_factories_once_and_preserve_ordered_effects() {
    let source = r#"
mut calls=0
fn delayed(sig size:Length):sig Length {
    calls=calls+1
    wait(100ms)
    let offset=size
    return size+offset
}


mut size=10pt
let first=Circle(radius=delayed(size))
let second=Circle(radius=delayed(size)/2)
let third=Circle(radius=abs(delayed(size)))
assert(calls==3)
add(first)
add(second)
add(third)
animate(size,to=20pt,duration=1s,ease="linear")
assert(calls==3)
"#;
    let prepared = prepare(source);
    assert_eq!(prepared.animation.duration, Fraction::new(13u64, 10u64));
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let duration = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(duration.duration, prepared.animation.duration);
    let elements = prepared
        .animation
        .layout(Fraction::new(13u64, 10u64), 1920.into(), 1080.into())
        .unwrap();
    for (element, expected) in elements.iter().zip([30, 15, 30]) {
        let Geometry::Circle { radius } = element.geometry.as_ref() else {
            panic!("circle")
        };
        assert_eq!(*radius, Fraction::from(expected as u32));
    }
}
#[test]
fn constant_native_wrappers_reuse_their_observed_factory_values() {
    let prepared = prepare(
        r#"
mut calls=0
fn color_name():sig String {calls=calls+1;return "blue3"}
let marker=Circle(fill=Color(color_name()))
assert(calls==1)
add(marker)
wait(1s)
"#,
    );
    assert_eq!(prepared.animation.duration, Fraction::from(1));
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"
mut color_name="blue3"
fn current_color_name():sig String{return color_name}
let marker=Circle(fill=Color(current_color_name()))
add(marker)
wait(1s)
color_name="red3"
wait(1s)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let before = prepared.animation.sample(Fraction::from(0)).unwrap();
    let after = prepared.animation.sample(Fraction::from(1)).unwrap();
    assert_ne!(
        before[0].fill, after[0].fill,
        "live String-to-Color conversion follows the input cell"
    );
}
#[test]
#[ignore = "measured by make signal-function-bench"]
fn signal_function_benchmark() {
    let mut animations = Vec::new();
    for helper in [false, true] {
        let mut source = String::from("mut size=10pt\n");
        if helper {
            source.push_str("fn twice(sig value:Length):sig Length{return value*2}\n");
        }
        for index in 0..100 {
            let value = if helper { "twice(size)" } else { "size*2" };
            source.push_str(&format!(
                "let marker_{index}=Circle(radius={value})\nadd(marker_{index})\n"
            ));
        }
        source.push_str("animate(size,to=20pt,duration=1s,ease=\"linear\")\n");
        let prepared = prepare(&source);
        assert!(matches!(prepared.bindings["marker_0"], Value::Element(_)));
        animations.push(prepared.animation);
    }
    assert_eq!(
        animations[0], animations[1],
        "explicit signal helpers must compile to the same native animation IR as inline arithmetic"
    );
    let mut timings = [Vec::new(), Vec::new()];
    for round in 0..7 {
        for index in [round % 2, (round + 1) % 2] {
            let started = std::time::Instant::now();
            for frame in 0u64..300 {
                let time = Fraction::new(frame % 30, 30u64);
                std::hint::black_box(
                    animations[index]
                        .layout(time, 1920.into(), 1080.into())
                        .unwrap(),
                );
            }
            timings[index].push(started.elapsed().as_micros());
        }
    }
    for (index, timings) in timings.iter_mut().enumerate() {
        timings.sort();
        eprintln!(
            "signal helper={} 100 shapes/300 samples median={}us nodes={} (7 interleaved rounds, identical native IR)",
            index == 1,
            timings[timings.len() / 2],
            animations[index].reactive.node_count()
        );
    }
}
