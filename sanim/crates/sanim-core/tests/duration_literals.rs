use sanim_core::{Fraction, Number, Unit, math};

#[test]
fn duration_components_are_exact_and_round_trip_through_display() {
    for (literal, seconds) in [
        ("0.5s", Fraction::new(1u64, 2u64)),
        ("2ms", Fraction::new(1u64, 500u64)),
        ("7m", Fraction::from(420)),
        ("4m1s", Fraction::from(241)),
        ("4m1.5s", Fraction::new(483u64, 2u64)),
        ("1s250ms", Fraction::new(5u64, 4u64)),
        ("-4m1s", Fraction::from(-241)),
    ] {
        let number = Number::parse(literal).unwrap();
        assert_eq!(number.unit, Unit::Duration, "{literal}");
        assert_eq!(number.value, seconds, "{literal}");
        let (value, unit) = number.display_parts().unwrap();
        assert_eq!(Number::parse(&format!("{value}{unit}")).unwrap(), number);
    }
}

#[test]
fn compound_duration_diagnostics_reject_mixed_repeated_or_reversed_units() {
    for literal in ["1m2pt", "1pt2pt", "1s2s", "1s2m", "1ms2s", "1m2"] {
        assert!(Number::parse(literal).is_err(), "{literal}");
    }
    assert!(Number::parse("18446744073709551615m").is_err());
}

#[test]
fn numeric_source_extent_preserves_operators_and_compound_duration() {
    for (source, literal) in [
        ("4m1.5s + 2ms", "4m1.5s"),
        ("0.5s)", "0.5s"),
        ("2%3", "2"),
        ("50% + 1", "50%"),
        ("1..4", "1"),
        ("1_000.5pt", "1_000.5pt"),
    ] {
        assert_eq!(&source[..math::number_literal_len(source)], literal);
    }
}
