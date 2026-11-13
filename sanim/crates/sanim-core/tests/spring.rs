use sanim_core::{Easing, Fraction, math};

#[test]
fn spring_has_exact_rest_endpoints_and_a_restrained_physical_overshoot() {
    assert_eq!(Easing::default(), Easing::Spring);
    assert_eq!(Easing::parse("spring").unwrap(), Easing::Spring);
    assert_eq!(math::spring(0.into()).unwrap(), Fraction::from(0));
    assert_eq!(math::spring(1.into()).unwrap(), Fraction::from(1));
    assert!(math::spring(Fraction::nan()).is_err());
    let mut peak=0.;
    for index in 0..=1000 {
        let value=math::to_f64(math::spring(Fraction::new(index,1000u64)).unwrap()).unwrap();
        assert!(value>=0. && value<1.03,"{index}: {value}");
        if value>peak {peak=value;}
    }
    assert!(peak>1.01,"a spring must retain its actual overshoot, got {peak}");
    assert!(math::spring(Fraction::new(1u64,4u64)).unwrap()>Fraction::new(4u64,5u64));
}

#[test]
fn spring_starts_and_finishes_at_rest_without_a_terminal_snap() {
    let epsilon=Fraction::new(1u64,10000u64);
    assert!(math::spring(epsilon).unwrap()<Fraction::new(1u64,1_000_000u64));
    assert!((Fraction::from(1)-math::spring(Fraction::from(1)-epsilon).unwrap()).abs()<Fraction::new(1u64,1_000_000u64));
    for time in [Fraction::new(2u64,5u64),Fraction::new(9u64,10u64),Fraction::new(1u64,10u64)] {
        let a=math::spring(time).unwrap();
        math::spring(1.into()).unwrap();
        assert_eq!(math::spring(time).unwrap(),a);
    }
}

#[test]
#[ignore="measured by make spring-bench"]
fn spring_sampling_benchmark() {
    const SAMPLES:u64=100_000;
    let start=std::time::Instant::now();
    let mut checksum=0.;
    for index in 0..SAMPLES {
        checksum+=math::to_f64(std::hint::black_box(Easing::Spring.apply(Fraction::new(index%1000,1000u64)).unwrap())).unwrap();
    }
    eprintln!("spring samples={SAMPLES} ns_per_sample={} checksum={checksum}",start.elapsed().as_nanos()/u128::from(SAMPLES));
    assert!(checksum.is_finite());
}
