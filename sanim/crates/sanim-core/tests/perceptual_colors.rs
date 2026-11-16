use sanim_core::{Color, math::color::*};
use shrimply_math_color::Oklab;

fn near(left: f32, right: f32, tolerance: f32) {
    assert!((left - right).abs() < tolerance, "{left} versus {right}");
}

#[test]
fn float_conversions_roundtrip_without_byte_quantization() {
    // The GTK/Android byte picker delegates to these same float conversions.
    let byte = shrimply_math_color::Color::<u8>::from_hsva(-120.0, 1.0, 1.0, 0.5);
    assert_eq!(byte.to_array(), [0, 0, 255, 128]);
    assert_eq!(
        shrimply_math_color::Color::<u8>::from_hsv(360.0, 1.0, 1.0).to_array(),
        [255, 0, 0, 255]
    );
    let gray = shrimply_math_color::Color::<u8>::from_rgba(128, 128, 128, 128).to_hsva();
    assert_eq!([gray[0], gray[1]], [0.0, 0.0]);
    near(gray[2], 128.0 / 255.0, 0.000001);
    for rgb in [
        [0.1, 0.2, 0.3, 0.4],
        [0.0001, 0.005, 0.8, 0.12345],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 0.0],
    ] {
        let lab = rgba_to_oklab(rgb);
        let round = map_oklab(lab, rgb[3], Gamut::Error).unwrap();
        for (left, right) in rgb.into_iter().zip(round) {
            near(left, right, 0.00001);
        }
        let hsv = hsv_to_rgba(rgba_to_hsv(rgb));
        let hsl = hsl_to_rgba(rgba_to_hsl(rgb));
        for round in [hsv, hsl] {
            for (left, right) in rgb.into_iter().zip(round) {
                near(left, right, 0.00001);
            }
        }
    }
    let red = rgba_to_oklab([1.0, 0.0, 0.0, 1.0]);
    near(red.l, 0.627955, 0.00001);
    near(red.a, 0.224863, 0.00001);
    near(red.b, 0.125846, 0.00001);
}

#[test]
fn alpha_correct_mixing_preserves_endpoints_and_does_not_import_hidden_colors() {
    for space in MixSpace::ALL {
        let options = MixOptions {
            space,
            ..Default::default()
        };
        let hidden = [1.0, 0.0, 0.0, 0.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(mix_rgba(hidden, blue, 0.0, options).unwrap(), hidden);
        assert_eq!(mix_rgba(hidden, blue, 1.0, options).unwrap(), blue);
        let middle = mix_rgba(hidden, blue, 0.5, options).unwrap();
        for (left, right) in middle.into_iter().zip([0.0, 0.0, 1.0, 0.5]) {
            near(left, right, 0.0001);
        }
        assert_eq!(mix_rgba(hidden, hidden, 0.5, options).unwrap(), [0.0; 4]);
        assert!(mix_rgba(hidden, blue, f64::NAN, options).is_err());
    }
}

#[test]
fn hue_paths_are_explicit_and_achromatic_hues_are_powerless() {
    let left = map_oklab(oklch_to_oklab([0.7, 0.1, 350.0]), 1.0, Gamut::Error).unwrap();
    let right = map_oklab(oklch_to_oklab([0.7, 0.1, 10.0]), 1.0, Gamut::Error).unwrap();
    let options = MixOptions {
        space: MixSpace::Oklch,
        ..Default::default()
    };
    let shortest = oklab_to_oklch(rgba_to_oklab(mix_rgba(left, right, 0.5, options).unwrap()))[2];
    assert!(!(0.01..=359.99).contains(&shortest), "{shortest}");
    let longer = oklab_to_oklch(rgba_to_oklab(
        mix_rgba(
            left,
            right,
            0.5,
            MixOptions {
                hue: HueInterpolation::Longer,
                ..options
            },
        )
        .unwrap(),
    ))[2];
    near(longer, 180.0, 0.01);
    let gray = [0.5, 0.5, 0.5, 1.0];
    let gray_mix = oklab_to_oklch(rgba_to_oklab(mix_rgba(gray, right, 0.5, options).unwrap()))[2];
    near(gray_mix, 10.0, 0.01);
    let zero_hue = map_oklab(oklch_to_oklab([0.7, 0.1, 0.0]), 1.0, Gamut::Error).unwrap();
    let quarter = oklab_to_oklch(rgba_to_oklab(
        mix_rgba(
            zero_hue,
            zero_hue,
            0.25,
            MixOptions {
                hue: HueInterpolation::Longer,
                ..options
            },
        )
        .unwrap(),
    ))[2];
    near(quarter, 90.0, 0.01);
}

#[test]
fn gamut_policies_are_finite_bounded_and_keep_chroma_mapped_lightness_and_hue() {
    let lab = oklch_to_oklab([0.65, 2.0, 35.0]);
    assert!(map_oklab(lab, 1.0, Gamut::Error).is_err());
    let mapped = map_oklab(lab, 0.4, Gamut::Chroma).unwrap();
    let lch = oklab_to_oklch(rgba_to_oklab(mapped));
    near(lch[0], 0.65, 0.00001);
    near(lch[2], 35.0, 0.001);
    near(mapped[3], 0.4, 0.00001);
    assert!(lch[1] > 0.0 && lch[1] < 2.0);
    assert!(
        map_oklab(
            Oklab {
                l: 0.5,
                a: f32::MAX,
                b: f32::MAX
            },
            1.0,
            Gamut::Chroma
        )
        .unwrap()
        .into_iter()
        .all(f32::is_finite)
    );
    assert_eq!(
        map_oklab(oklch_to_oklab([0.0, 10.0, 30.0]), 1.0, Gamut::Chroma).unwrap(),
        [0.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        map_oklab(oklch_to_oklab([1.0, 10.0, 30.0]), 1.0, Gamut::Chroma).unwrap(),
        [1.0; 4]
    );
}

#[test]
fn prepared_ramps_share_math_and_preflight_storage_without_per_sample_allocation() {
    let colors = vec![
        [0.1, 0.5, 0.8, 0.3],
        [0.9, 0.2, 0.1, 0.7],
        [0.5, 0.1, 0.9, 1.0],
    ];
    for space in MixSpace::ALL {
        let options = MixOptions {
            space,
            ..Default::default()
        };
        let ramp = Ramp::new(colors.clone(), options).unwrap();
        assert_eq!(
            ramp.retained_bytes(),
            Ramp::allocation_bytes(colors.len()).unwrap()
        );
        for index in 0..=100 {
            let at = f64::from(index) / 100.0;
            let pair = usize::from(at >= 0.5);
            let expected = mix_rgba(
                colors[pair],
                colors[pair + 1],
                (at * 2.0 - pair as f64).clamp(0.0, 1.0),
                options,
            )
            .unwrap();
            assert_eq!(ramp.sample(at).unwrap(), expected);
        }
    }
    assert!(Ramp::allocation_bytes(usize::MAX).is_err());
    assert!(Ramp::new(vec![], MixOptions::default()).is_err());
}

#[test]
fn color_names_use_shared_parser_and_transparent_is_preserved() {
    assert_eq!(
        Color::parse("rebeccapurple").unwrap().rgba8().unwrap(),
        [102, 51, 153, 255]
    );
    assert_eq!(
        Color::parse("#abc").unwrap().rgba8().unwrap(),
        [170, 187, 204, 255]
    );
    assert_eq!(
        Color::parse("Transparent").unwrap().rgba8().unwrap(),
        [0; 4]
    );
    assert_eq!(
        Color::parse("rgb(255, 0, 0)").unwrap(),
        Color::parse("red").unwrap()
    );
}

#[test]
fn linear_light_compositing_and_contrast_are_explicit() {
    near(
        relative_luminance([0.0, 0.0, 0.0, 1.0]).unwrap(),
        0.0,
        0.000001,
    );
    near(
        contrast([0.0, 0.0, 0.0, 1.0], [1.0; 4]).unwrap(),
        21.0,
        0.00001,
    );
    assert!(contrast([0.0, 0.0, 0.0, 0.5], [1.0; 4]).is_err());
    let composite = over([1.0, 0.0, 0.0, 0.5], [0.0, 0.0, 1.0, 1.0]).unwrap();
    near(composite[0], 0.7353569, 0.00001);
    near(composite[2], 0.7353569, 0.00001);
    let original = [0.1234, 0.3456, 0.5678, 0.75];
    assert_eq!(over(original, [1.0, 0.0, 0.0, 0.0]).unwrap(), original);
    assert_eq!(over([1.0, 0.0, 0.0, 0.0], original).unwrap(), original);
    assert_eq!(
        over([1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]).unwrap(),
        [0.0; 4]
    );
}

#[test]
fn retained_record_projection_memoizes_native_parents_and_prunes_unrelated_fields() {
    use sanim_core::{
        Number, Unit, Value,
        reactive::{Node, ReactiveGraph},
    };
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, Value::Number(Number::scalar(3))).unwrap();
    graph.set_cell(2, Value::Number(Number::scalar(8))).unwrap();
    graph
        .set_cell(3, Value::Color(Color::parse("blue").unwrap()))
        .unwrap();
    let a = graph.insert_node(Node::Cell(1), 100).unwrap();
    let b = graph.insert_node(Node::Cell(2), 100).unwrap();
    let record = graph
        .insert_node(
            Node::Record {
                fields: [("a".into(), a), ("b".into(), b)].into(),
            },
            100,
        )
        .unwrap();
    let selected = graph
        .insert_node(
            Node::Field {
                record,
                name: "a".into(),
            },
            100,
        )
        .unwrap();
    assert_eq!(
        selected, a,
        "literal field selection should reuse the selected child"
    );
    assert_eq!(
        graph
            .cell_dependencies(selected)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![1]
    );
    let color = graph.insert_node(Node::Cell(3), 100).unwrap();
    let converted = graph
        .insert_node(
            Node::NativeConfig {
                operation: sanim_core::code::values::Native::Perceptual(Op::Convert(Space::Oklch)),
                arguments: [("color".into(), color)].into(),
            },
            100,
        )
        .unwrap();
    let l = graph
        .insert_node(
            Node::Field {
                record: converted,
                name: "lightness".into(),
            },
            100,
        )
        .unwrap();
    let h = graph
        .insert_node(
            Node::Field {
                record: converted,
                name: "hue".into(),
            },
            100,
        )
        .unwrap();
    assert_eq!(
        graph.node_type(h),
        Some(&sanim_core::Type::Numeric(Unit::Angle))
    );
    let error = graph
        .insert_node(
            Node::Field {
                record: converted,
                name: "missing".into(),
            },
            100,
        )
        .unwrap_err();
    assert!(
        error.contains("missing") && error.contains("available fields"),
        "{error}"
    );
    let mut sampler = graph.sampler(0.into()).unwrap();
    sampler.value(l).unwrap();
    sampler.value(h).unwrap();
    sampler.value(l).unwrap();
    assert_eq!(
        sampler.evaluated_nodes(),
        4,
        "one cell, one conversion record, and two projected fields"
    );
    let restored: ReactiveGraph =
        serde_json::from_str(&serde_json::to_string(&graph).unwrap()).unwrap();
    assert_eq!(
        restored.sampler(0.into()).unwrap().value(h).unwrap(),
        sampler.value(h).unwrap()
    );
}

#[test]
#[ignore = "manual prepared color-ramp performance comparison"]
fn prepared_perceptual_ramp_benchmark() {
    use std::{hint::black_box, time::Instant};
    let left = [0.1, 0.5, 0.8, 1.0];
    let right = [0.9, 0.2, 0.1, 0.5];
    let options = MixOptions::default();
    let ramp = Ramp::new(vec![left, right], options).unwrap();
    let start = Instant::now();
    let mut sum = 0.0;
    for index in 0..65_536 {
        sum += black_box(mix_rgba(left, right, f64::from(index) / 65_535.0, options).unwrap())[0];
    }
    let uncached = start.elapsed();
    black_box(sum);
    let start = Instant::now();
    let mut sum = 0.0;
    for index in 0..65_536 {
        sum += black_box(ramp.sample(f64::from(index) / 65_535.0).unwrap())[0];
    }
    let prepared = start.elapsed();
    black_box(sum);
    eprintln!(
        "perceptual-ramp pixels=65536 direct_us={} prepared_us={} retained_bytes={}",
        uncached.as_micros(),
        prepared.as_micros(),
        ramp.retained_bytes()
    );
    assert!(
        prepared < uncached,
        "prepared ramps should remove repeated stop conversions"
    );
}
