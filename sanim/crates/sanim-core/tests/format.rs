use sanim_core::{Fraction, Number, Value, math::format};

fn number(text: &str) -> Value {
    Value::Number(Number::parse(text).unwrap())
}

#[test]
fn python_style_numeric_and_string_formats_are_bounded_and_exact_for_integers() {
    for (input, spec, expected) in [
        ("-42", "#010x", "-0x000002a"),
        ("1048576", "#_x", "0x10_0000"),
        ("3735928559", "08_x", "dead_beef"),
        ("3735928559", "#012_x", "0x0_dead_beef"),
        ("42", "08b", "00101010"),
        ("3.1415926", "+010.2f", "+000003.14"),
        ("1250", ".2e", "1.25e+03"),
        ("0.000001", ".3g", "1e-06"),
        ("0.000001", "", "1e-06"),
        ("12.345", ".2", "1.2e+01"),
        ("1.005", ".2", "1.0"),
        ("1.005", ".2f", "1.00"),
        ("12345.6", ",.1f", "12,345.6"),
        ("0.125", ".1%", "12.5%"),
        ("42", "*^9d", "***42****"),
        ("65", "c", "A"),
        ("-42", "08,d", "-000,042"),
        ("12.5", "08,.2f", "0,012.50"),
    ] {
        assert_eq!(
            format::value(&number(input), spec, None).unwrap(),
            expected,
            "{input}:{spec}"
        );
    }
    assert_eq!(
        format::value(&Value::Fraction(Fraction::from(u64::MAX)), "x", None).unwrap(),
        "ffffffffffffffff"
    );
    assert_eq!(
        format::value(&Value::String("hello".into()), "*^10.3s", None).unwrap(),
        "***hel****"
    );
    assert_eq!(
        format::value(&Value::String("é🙂a".into()), "^8.2", None).unwrap(),
        "   é🙂   "
    );
    assert_eq!(format::value(&number("0.5"), "", None).unwrap(), "0.5");
}

#[test]
fn conversions_quote_strings_and_escape_ascii_without_a_python_runtime() {
    let text = Value::String("é🙂".into());
    assert_eq!(format::value(&text, "", Some("r")).unwrap(), "'é🙂'");
    assert_eq!(
        format::value(&text, "", Some("a")).unwrap(),
        "'\\xe9\\U0001f642'"
    );
    assert_eq!(
        format::value(&Value::String("can't".into()), "", Some("r")).unwrap(),
        "\"can't\""
    );
    assert_eq!(
        format::value(&Value::String("a\nb".into()), "", Some("r")).unwrap(),
        "'a\\nb'"
    );
    assert_eq!(
        format::value(&number("0.5"), "*>6", Some("s")).unwrap(),
        "***0.5"
    );
}

#[test]
fn invalid_types_precision_and_width_fail_with_clear_errors() {
    for spec in [".2b", "q", ".", "999999999d", ".9999999f", ",x", "s"] {
        assert!(format::value(&number("42"), spec, None).is_err(), "{spec}");
    }
    assert!(
        format::value(&number("0.5"), "b", None)
            .unwrap_err()
            .contains("integral")
    );
    assert!(format::value(&number("1114112"), "c", None).is_err());
    assert!(
        format::value(&Value::String("hello".into()), "+5", None)
            .unwrap_err()
            .contains("String")
    );
    assert!(format::value(&Value::String("hello".into()), "=5", None).is_err());
    assert!(format::value(&number("42"), "", Some("z")).is_err());
}

#[test]
fn static_validation_uses_the_shared_parser_and_rejects_only_provable_type_errors() {
    use sanim_core::{Type, Unit};
    assert!(format::validate("q", &Type::Unknown, None).is_err());
    assert!(format::validate(".2f", &Type::String, None).is_err());
    assert!(format::validate(".2b", &Type::Numeric(Unit::Number), None).is_err());
    assert!(format::validate("08b", &Type::Numeric(Unit::Number), None).is_ok());
    assert!(format::validate("08b", &Type::Data, None).is_ok());
    assert!(
        format::validate(
            "08b",
            &Type::Union(vec![Type::String, Type::Numeric(Unit::Number)]),
            None
        )
        .is_ok()
    );
    assert!(format::validate("08b", &Type::Union(vec![Type::String, Type::Color]), None).is_err());
    assert!(format::validate("08b", &Type::Numeric(Unit::Number), Some("r")).is_err());
    assert!(format::validate("", &Type::Unknown, Some("invalid")).is_err());
}

#[test]
#[ignore = "measured explicitly by make format-bench"]
fn formatter_microbenchmark() {
    let number = number("12.345");
    let mut bytes = 0usize;
    const SAMPLES: usize = 100_000;
    let started = std::time::Instant::now();
    for index in 0..SAMPLES {
        let spec = [".2f", "+010.2f", ".3g", "08.1%", "_.3e"][index % 5];
        bytes += std::hint::black_box(format::value(&number, spec, None).unwrap()).len();
    }
    eprintln!(
        "formatter samples={SAMPLES} ns_per_sample={} output_bytes={bytes}",
        started.elapsed().as_nanos() / SAMPLES as u128
    );
    assert!(bytes > 0);
}

#[test]
#[ignore = "manual differential reference: requires Python 3, never used by the runtime"]
fn python_differential_reference_formats() {
    let integers = [
        "0",
        "1",
        "-1",
        "42",
        "-42",
        "255",
        "1048576",
        "3735928559",
        "18446744073709551615",
    ];
    let floats = [
        "0.5",
        "0.125",
        "12.345",
        "-12.345",
        "0.000001",
        "1.005",
        "2.675",
        "0.9999999",
        "999.999",
        "1000000.25",
        "-0.00001",
    ];
    let integer_formats = [
        "", "d", "b", "o", "x", "X", "#b", "#o", "#x", "#X", "08b", "#010x", "+08d", " 08d",
        "<08d", ">08d", "=08d", "*^9d", "_d", ",d", "08,d", "08_d", "08_x", "#012_x", "c",
    ];
    let float_formats = [
        "f", "F", ".0f", ".1f", ".2f", "08.2f", "+010.2f", " .2f", "<12.3f", "*>12.3f", "^12.2f",
        "=+12.2f", ",.2f", "_.2f", "012,.2f", "#.0f", "e", "E", ".0e", ".2e", "g", "G", ".0g",
        ".3g", "#.3g", "%", "08.1%", "+010.2%", "z.2f", "z.0f", "z.3g", "z.1%",
    ];
    let mut cases = Vec::new();
    for input in integers {
        for spec in integer_formats.into_iter().chain(float_formats) {
            cases.push(serde_json::json!(["int", input, spec, null]));
        }
    }
    for input in floats {
        for spec in float_formats
            .into_iter()
            .chain(["", ".2", "12.2", "08", "+08", "_", ","])
        {
            cases.push(serde_json::json!(["float", input, spec, null]));
        }
    }
    for input in ["", "hi", "hello", "é🙂a", "can't", "line\n"] {
        for spec in [
            "", "s", "8", "08", "<8", ">8", "^8", "*^9s", ".0", ".2", "*>8.2s", "=8", "d", "+8",
        ] {
            cases.push(serde_json::json!(["str", input, spec, null]));
        }
        for conversion in ["s", "r", "a"] {
            for spec in ["", ">14", "*^14.6"] {
                cases.push(serde_json::json!(["str", input, spec, conversion]));
            }
        }
    }
    let script = r#"import json,sys
out=[]
for kind,n,s,c in json.loads(sys.argv[1]):
    v={'int':int,'float':float,'str':str}[kind](n)
    if c: v={'s':str,'r':repr,'a':ascii}[c](v)
    try: out.append({'ok':format(v,s)})
    except (ValueError,OverflowError) as e: out.append({'error':str(e)})
print(json.dumps(out))"#;
    let output = std::process::Command::new("python3")
        .args(["-c", script, &serde_json::to_string(&cases).unwrap()])
        .output()
        .expect("Python 3 is required for the explicit differential test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let mut mismatches = Vec::new();
    for (case, expected) in cases.iter().zip(expected) {
        let kind = case[0].as_str().unwrap();
        let input = case[1].as_str().unwrap();
        let spec = case[2].as_str().unwrap();
        let conversion = case[3].as_str();
        let value = if kind == "str" {
            Value::String(input.into())
        } else {
            number(input)
        };
        let actual = format::value(&value, spec, conversion);
        let matches = match (
            &actual,
            expected.get("ok").and_then(serde_json::Value::as_str),
        ) {
            (Ok(actual), Some(expected)) => actual == expected,
            (Err(_), None) => true,
            _ => false,
        };
        if !matches {
            mismatches.push(format!("{case}: actual={actual:?}, expected={expected}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!("Python differential cases={}", cases.len());
}
