use sanim_core::{Number, Template, TemplatePart, Value, sql::Sql};
fn template(count: usize) -> Template {
    let mut parts = vec![TemplatePart::Text("SELECT ".into())];
    for index in 0..count {
        if index > 0 {
            parts.push(TemplatePart::Text("+".into()))
        }
        parts.push(TemplatePart::Hole {
            value: Value::Number(Number::scalar(index as i64)),
            format: None,
            conversion: None,
        });
    }
    Template {
        parts: parts.into(),
    }
}
#[test]
fn sql_limits_and_identifier_quoting_are_bounded_without_a_second_sql_parser() {
    let statement = Sql::compile(&template(3), 10_000, 100).unwrap();
    assert!(statement.text.contains(":__sanim_hole_0"));
    assert!(
        Sql::compile(&template(3), 10_000, 2)
            .unwrap_err()
            .contains("parameter count")
    );
    assert!(
        Sql::compile(&template(3), 10, 100)
            .unwrap_err()
            .contains("byte budget")
    );
    let parts = Template {
        parts: vec![
            TemplatePart::Text("SELECT ".into()),
            TemplatePart::Hole {
                value: Value::SqlIdent(
                    sanim_core::sql::identifier("x\"; DROP TABLE t;--").unwrap(),
                ),
                format: None,
                conversion: None,
            },
        ]
        .into(),
    };
    assert_eq!(
        &*Sql::compile(&parts, 1000, 100).unwrap().text,
        "SELECT \"x\"\"; DROP TABLE t;--\""
    );
    assert!(sanim_core::sql::identifier("bad\0name").is_err());
    assert!(template(20).render(10).unwrap_err().contains("byte budget"));
}
#[test]
#[ignore]
fn template_benchmark() {
    use std::{hint::black_box, time::Instant};
    let parts = template(20);
    let iterations = 20_000;
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(Sql::compile(&parts, 1_000_000, 1000).unwrap());
    }
    let compiled = start.elapsed();
    let statement = Sql::compile(&parts, 1_000_000, 1000).unwrap();
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(statement.clone());
    }
    let reused = start.elapsed();
    eprintln!(
        "SQL 20-hole templates {iterations} constructions {compiled:?}, reusable Sql COW clones {reused:?}"
    );
    // Isolate the old balanced copying strategy from parsing/evaluation. Keep
    // output parity and report repeated interleaved medians, not timing asserts.
    fn balanced(parts: &Template) -> String {
        let mut strings = parts
            .parts
            .iter()
            .map(|part| match part {
                TemplatePart::Text(text) => text.to_string(),
                TemplatePart::Hole { value, .. } => {
                    sanim_core::math::format::value(value, "", None).unwrap()
                }
            })
            .collect::<Vec<_>>();
        while strings.len() > 1 {
            let mut next = Vec::with_capacity(strings.len().div_ceil(2));
            let mut values = strings.into_iter();
            while let Some(mut left) = values.next() {
                if let Some(right) = values.next() {
                    left.push_str(&right);
                }
                next.push(left);
            }
            strings = next;
        }
        strings.pop().unwrap()
    }
    const ROUNDS: usize = 5;
    for (holes, iterations) in [(1, 50_000), (20, 10_000), (2000, 100)] {
        let parts = template(holes);
        assert_eq!(parts.render(1_000_000).unwrap(), balanced(&parts));
        let (mut linear, mut old) = (Vec::with_capacity(ROUNDS), Vec::with_capacity(ROUNDS));
        for round in 0..ROUNDS {
            for use_linear in [round % 2 == 0, round % 2 != 0] {
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(if use_linear {
                        parts.render(1_000_000).unwrap()
                    } else {
                        balanced(&parts)
                    });
                }
                if use_linear {
                    linear.push(started.elapsed());
                } else {
                    old.push(started.elapsed());
                }
            }
        }
        linear.sort_unstable();
        old.sort_unstable();
        let (linear, old) = (linear[ROUNDS / 2], old[ROUNDS / 2]);
        eprintln!(
            "String templates holes={holes} iterations={iterations} median_rounds={ROUNDS} linear={linear:?} previous_balanced={old:?} speedup={:.2}x",
            old.as_secs_f64() / linear.as_secs_f64()
        );
    }
}
