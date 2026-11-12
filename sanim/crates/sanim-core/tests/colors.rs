use sanim_core::Fraction;

#[test]
fn shared_adwaita_palette_is_available_without_duplicating_definitions() {
    for (name, native) in shrimply_math_color::Color::ADWAITA {
        let color = sanim_core::Color::parse(name).unwrap();
        assert_eq!(
            color,
            sanim_core::Color::parse(&name.to_ascii_lowercase()).unwrap()
        );
        for (actual, expected) in color.0.iter().zip(native.to_array()) {
            assert!(
                (sanim_core::math::to_f64(*actual).unwrap() - f64::from(expected)).abs() < 1e-7,
                "{name}"
            );
        }
    }
    assert_eq!(
        sanim_core::Color::parse("blue3").unwrap(),
        sanim_core::Color::parse("#3584e4").unwrap()
    );
    assert_eq!(
        sanim_core::Color::parse("card_bg_dark").unwrap().0[3],
        Fraction::new(2u64, 25u64)
    );
}
