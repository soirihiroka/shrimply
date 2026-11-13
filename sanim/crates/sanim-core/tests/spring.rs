use sanim_core::{Easing, Fraction, math};

#[test]
fn spring_defaults_to_critical_damping_with_optional_restrained_bounce() {
    assert_eq!(Easing::default(), Easing::Spring);
    assert_eq!(Easing::parse("spring").unwrap(), Easing::Spring);
    assert_eq!(math::spring(0.into()).unwrap(), Fraction::from(0));
    assert_eq!(math::spring(1.into()).unwrap(), Fraction::from(1));
    assert!(math::spring(Fraction::nan()).is_err());
    let mut peak = 0.;
    let mut previous = 0.;
    for index in 0u64..=1000 {
        let p = Fraction::new(index, 1000u64);
        let normal = math::to_f64(math::spring(p).unwrap()).unwrap();
        assert!(normal >= previous && normal <= 1.);
        previous = normal;
        let value = math::to_f64(math::spring_bouncy(p).unwrap()).unwrap();
        assert!((0. ..1.03).contains(&value), "{index}: {value}");
        if value > peak {
            peak = value;
        }
    }
    assert!(
        peak > 1.01,
        "a spring must retain its actual overshoot, got {peak}"
    );
    assert!(math::spring(Fraction::new(1u64, 4u64)).unwrap() > Fraction::new(4u64, 5u64));
}

#[test]
fn spring_starts_and_finishes_at_rest_without_a_terminal_snap() {
    let epsilon = Fraction::new(1u64, 10000u64);
    assert!(math::spring(epsilon).unwrap() <= Fraction::new(1u64, 1_000_000u64));
    assert!(
        (Fraction::from(1) - math::spring(Fraction::from(1) - epsilon).unwrap()).abs()
            < Fraction::new(1u64, 1_000_000u64)
    );
    for time in [
        Fraction::new(2u64, 5u64),
        Fraction::new(9u64, 10u64),
        Fraction::new(1u64, 10u64),
    ] {
        let a = math::spring(time).unwrap();
        math::spring(1.into()).unwrap();
        assert_eq!(math::spring(time).unwrap(), a);
    }
}

#[test]
fn visual_composition_handles_signal_precision_and_tiny_positive_bounds() {
    let signal = Fraction::new(10_644_217_687u64, 10_000_000_000u64);
    let spring = math::spring(Fraction::new(1u64, 60u64)).unwrap();
    let scale = math::visual_lerp(Fraction::new(23u64, 25u64), 1.into(), spring).unwrap();
    assert!(math::visual_product(signal, scale).unwrap() > Fraction::from(0));
    let low = Fraction::new(1u64, 1_000_000_000_000_000_000u64);
    let high = low + low;
    let value =
        math::visual_lerp(low, high, math::spring(Fraction::new(1u64, 5u64)).unwrap()).unwrap();
    assert!(value >= low && value <= high);
    assert_eq!(math::visual_lerp(low, high, 0.into()).unwrap(), low);
    assert_eq!(math::visual_lerp(low, high, 1.into()).unwrap(), high);
    for easing in [
        Easing::CubicIn,
        Easing::CubicOut,
        Easing::Smooth,
        Easing::Smootherstep,
        Easing::Overshoot,
        Easing::Spring,
        Easing::SpringBouncy,
    ] {
        assert!(math::finite(
            &easing
                .apply(Fraction::new(123_456_789u64, 1_000_000_000u64))
                .unwrap()
        ));
    }
}

#[test]
#[ignore = "measured by make spring-bench"]
fn spring_sampling_benchmark() {
    const SAMPLES: u64 = 100_000;
    let start = std::time::Instant::now();
    let mut checksum = 0.;
    for index in 0..SAMPLES {
        checksum += math::to_f64(std::hint::black_box(
            Easing::Spring
                .apply(Fraction::new(index % 1000, 1000u64))
                .unwrap(),
        ))
        .unwrap();
    }
    eprintln!(
        "spring samples={SAMPLES} ns_per_sample={} checksum={checksum}",
        start.elapsed().as_nanos() / u128::from(SAMPLES)
    );
    assert!(checksum.is_finite());
}
