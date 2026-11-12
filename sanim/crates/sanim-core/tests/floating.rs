use sanim_core::{Fraction, math};

#[test]
fn binary_float_conversion_preserves_common_sql_and_geometry_values_exactly() {
    for value in [
        0.0,
        -0.0,
        0.1,
        -0.1,
        std::f64::consts::PI,
        0.9644630372551779,
        1.0 + f64::EPSILON,
        2f64.powi(-63),
        -(2f64.powi(-63)),
        2f64.powi(63),
    ] {
        let fraction = math::from_f64(value).unwrap_or_else(|error| panic!("{value}: {error}"));
        assert_eq!(math::to_f64(fraction).unwrap(), value);
    }
    assert_eq!(math::from_f64(0.5).unwrap(), Fraction::new(1u64, 2u64));
    assert_eq!(
        math::from_f64(0.1).unwrap(),
        Fraction::new(3602879701896397u64, 36028797018963968u64)
    );
}

#[test]
fn unsupported_binary_scalar_range_fails_instead_of_rounding() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        2f64.powi(-64),
        2f64.powi(64),
        f64::MAX,
    ] {
        assert!(math::from_f64(value).is_err(), "{value}");
    }
}
