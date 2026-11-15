use sanim_core::{Fraction, Paint, Value};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}
#[test]
fn directions_lower_to_the_same_native_linear_gradient() {
    for (direction, angle) in [
        ("r", 0),
        ("br", 45),
        ("b", 90),
        ("bl", 135),
        ("l", 180),
        ("tl", 225),
        ("t", 270),
        ("tr", 315),
    ] {
        let source = format!(
            "let utility=<Box bg-linear-to-{direction} from-blue3 to-orange3/>\nlet native=Box(fill=LinearGradient(from=Color(\"blue3\"),to=Color(\"orange3\"),angle={angle}deg))\nAnimation(children=[utility])\nAnimation(children=[native])"
        );
        let scene = prepare(&source).unwrap();
        assert_eq!(
            scene.animation.find("utility").unwrap().fill,
            scene.animation.find("native").unwrap().fill
        );
    }
}
#[test]
fn stop_positions_extend_the_end_colors_and_preserve_native_validation() {
    let scene=prepare("let shape=<Box to-orange3 from-10% bg-linear-to-r via-30% from-blue3 via-purple3 to-90%/>\nAnimation(children=[shape])").unwrap();
    let Paint::LinearGradient { stops, .. } = &scene.animation.find("shape").unwrap().fill else {
        panic!("gradient")
    };
    assert_eq!(
        stops.iter().map(|stop| stop.offset).collect::<Vec<_>>(),
        vec![
            0.into(),
            Fraction::new(1u64, 10u64),
            Fraction::new(3u64, 10u64),
            Fraction::new(9u64, 10u64),
            1.into()
        ]
    );
    assert_eq!(stops[0].color, stops[1].color);
    assert_eq!(stops[3].color, stops[4].color);
}
#[test]
fn missing_end_color_fades_the_last_hue_to_transparent() {
    for extra in ["", "via-purple3"] {
        let scene = prepare(&format!(
            "let shape=<Box bg-linear-to-r from-blue3 {extra}/>\nAnimation(children=[shape])"
        ))
        .unwrap();
        let Paint::LinearGradient { stops, .. } = &scene.animation.find("shape").unwrap().fill
        else {
            panic!("gradient")
        };
        let last = stops.last().unwrap();
        assert_eq!(last.color.0[3], 0.into());
        assert_eq!(&last.color.0[..3], &stops[stops.len() - 2].color.0[..3]);
    }
}
#[test]
fn gradient_conflicts_and_missing_parts_explain_the_fix() {
    for (utilities, reason) in [
        ("from-blue3 to-orange3", "direction"),
        ("bg-linear-to-r", "starting color"),
        ("bg-linear-to-r from-blue3 from-orange3", "duplicate from"),
        (
            "bg-linear-to-r bg-linear-to-b from-blue3",
            "multiple gradient directions",
        ),
        ("bg-linear-to-r from-blue3 via-50%", "via-COLOR"),
        (
            "bg-linear-to-r from-blue3 to-orange3 from-90% to-10%",
            "ordered",
        ),
        ("bg-linear-to-r from-blue3 to-110%", "between 0% and 100%"),
        ("bg-linear-to-r from-blue3 fill=\"red3\"", "explicit fill"),
    ] {
        let error = prepare(&format!("let shape=<Box {utilities}/>")).unwrap_err();
        assert!(error.message.contains(reason), "{utilities}: {error}");
    }
}
#[test]
fn gradient_utilities_share_completion_validation_and_formatter() {
    for prefix in ["bg-linear-to-", "from-", "via-", "to-"] {
        let candidates = sanim_lang::utilities::completions("Box", prefix);
        assert!(!candidates.is_empty());
        for candidate in candidates {
            assert!(
                candidate
                    .property_names()
                    .any(|property| property == "fill")
            );
            assert!(sanim_lang::utilities::describe("Box", &candidate.token).is_ok());
        }
    }
    assert!(sanim_lang::utilities::conflicts("from-blue3", "from-red3"));
    assert!(!sanim_lang::utilities::conflicts("from-blue3", "from-10%"));
    assert!(sanim_lang::utilities::conflicts("from-blue3", "fill"));
    assert!(sanim_lang::utilities::conflicts("fill", "bg-linear-to-r"));
    assert!(!sanim_lang::utilities::conflicts("from-blue3", "width"));
    assert!(sanim_lang::utilities::conflicts(
        "bg-linear-to-r",
        "bg-gradient-to-t"
    ));
    let source = "let shape=<Box bg-linear-to-r from-blue3 via-purple3 to-orange3/>\nAnimation(children=[shape])";
    let formatted =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 1_000_000)
            .unwrap();
    assert_eq!(
        prepare(source).unwrap().animation,
        prepare(&formatted).unwrap().animation
    );
}
#[test]
fn gradient_edits_restore_shared_prefix_state_with_bounded_checkpoint_memory() {
    use std::sync::Arc;
    let prefix = "let earlier=<Box w-100 h-60 bg-linear-to-r from-blue3 to-orange3/>\nAnimation(children=[earlier])\nwait(1s)\n";
    let mut engine = Engine::default();
    let mut sources = MemorySources::default();
    sources.insert("main.sanim",format!("{prefix}let later=<Box bg-linear-to-r from-blue3 from-0% to-orange3/>\nAnimation(children=[later])\nwait(1s)"));
    let original = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let earlier = original.animation.find("earlier").unwrap();
    let Paint::LinearGradient {
        stops: original_stops,
        ..
    } = &earlier.fill
    else {
        panic!("gradient")
    };
    let mut bytes = (usize::MAX, 0);
    for position in 1..80 {
        sources.insert("main.sanim",format!("{prefix}let later=<Box bg-linear-to-r from-blue3 from-{position}% to-orange3/>\nAnimation(children=[later])\nwait(1s)"));
        let changed = engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        assert!(changed.statistics.restored_statements >= 3);
        let restored = changed.animation.find("earlier").unwrap();
        assert!(Arc::ptr_eq(&earlier.geometry, &restored.geometry));
        let Paint::LinearGradient { stops, .. } = &restored.fill else {
            panic!("gradient")
        };
        assert!(Arc::ptr_eq(original_stops, stops));
        bytes.0 = bytes.0.min(changed.statistics.checkpoint_bytes);
        bytes.1 = bytes.1.max(changed.statistics.checkpoint_bytes);
    }
    assert!(
        bytes.1 - bytes.0 < 4096,
        "gradient edits must not accumulate checkpoint state: {bytes:?}"
    );
}
#[test]
#[ignore = "manual gradient utility/native lowering benchmark"]
fn gradient_utility_benchmark() {
    const COUNT: usize = 200;
    let sources:Vec<_>=[false,true].into_iter().map(|markup| {
        (0..COUNT).map(|index| if markup { format!("let item_{index}=<Box w-100 h-60 bg-linear-to-r from-blue3 to-orange3/>\n") }
        else { format!("let item_{index}=Box(width=100pt,height=60pt,fill=LinearGradient(from=Color(\"blue3\"),to=Color(\"orange3\")))\n") }).collect::<String>()
    }).collect();
    let native = prepare(&sources[0]).unwrap();
    let markup = prepare(&sources[1]).unwrap();
    for index in 0..COUNT {
        let Value::Element(native) = &native.bindings[&format!("item_{index}")] else {
            panic!("element")
        };
        let Value::Element(markup) = &markup.bindings[&format!("item_{index}")] else {
            panic!("element")
        };
        assert_eq!(
            native, markup,
            "native and utility draw state must be identical"
        );
    }
    let mut times = [Vec::new(), Vec::new()];
    for pass in 0..6 {
        for index in if pass % 2 == 0 { [0, 1] } else { [1, 0] } {
            let start = std::time::Instant::now();
            let prepared = prepare(&sources[index]).unwrap();
            times[index].push(start.elapsed().as_micros());
            assert_eq!(
                prepared
                    .bindings
                    .values()
                    .filter(|value| matches!(value, Value::Element(_)))
                    .count(),
                COUNT
            );
        }
    }
    times.iter_mut().for_each(|times| times.sort_unstable());
    println!(
        "{COUNT} gradients native_median_us={} markup_median_us={} markup_ratio={:.3}",
        times[0][3],
        times[1][3],
        times[1][3] as f64 / times[0][3] as f64
    );
}
