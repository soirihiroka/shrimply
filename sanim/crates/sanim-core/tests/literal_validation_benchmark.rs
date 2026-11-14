use sanim_core::{List, Type, Value};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

#[test]
#[ignore = "manual discriminator validation throughput: make literal-validation-bench"]
fn discriminated_union_validation_benchmark() {
    const ROWS: usize = 4096;
    const WORDS: usize = 32;
    const SAMPLES: usize = 15;
    let samples = Type::List(Box::new(Type::String), sanim_core::ListLength::Unknown);
    let shape = |kind| {
        Type::Record(BTreeMap::from([
            ("kind".to_owned(), kind),
            ("samples".to_owned(), samples.clone()),
        ]))
    };
    let row = shape(Type::String);
    let expected = Type::List(
        Box::new(
            Type::union([
                shape(Type::StringLiteral("before".into())),
                shape(Type::StringLiteral("after".into())),
            ])
            .unwrap(),
        ),
        sanim_core::ListLength::Unknown,
    );
    let words = Value::List(List::typed(
        Type::String,
        (0..WORDS)
            .map(|index| Value::String(format!("sample {index}").into()))
            .collect(),
    ));
    let value = Value::List(List::typed(
        row,
        (0..ROWS)
            .map(|index| {
                Value::Record(
                    [
                        (
                            "kind".to_owned(),
                            Value::String(if index % 2 == 0 { "before" } else { "after" }.into()),
                        ),
                        ("samples".to_owned(), words.clone()),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect(),
    ));
    let mut timings = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let normalized = black_box(&value).with_type(black_box(&expected)).unwrap();
        black_box(normalized);
        timings.push(started.elapsed());
    }
    timings.sort_unstable();
    println!(
        "literal union normalization: {ROWS} records x {WORDS} shared strings; samples={SAMPLES}; median={:?}; minimum={:?}",
        timings[SAMPLES / 2],
        timings[0]
    );
}
