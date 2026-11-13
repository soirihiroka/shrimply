use sanim_core::{Fraction, Number, Value};
use sanim_lang::{Engine, MemorySources};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

fn main() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim","let tint: Color = reflect(Color(\"red\"))\nfor i in range(1000) {\nlet marker = Circle(id: str(i), radius: 2pt, position: Vec2(0%, 0%), fill: tint)\nFadeIn(marker, duration=1ms, ease=\"linear\")\n}\n");
    let mut engine = Engine::default();
    let start = Instant::now();
    let prepared = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .expect("benchmark script");
    println!(
        "cold_prepare_us={} stats={:?}",
        start.elapsed().as_micros(),
        prepared.statistics
    );
    let start = Instant::now();
    for _ in 0..10 {
        black_box(
            engine
                .prepare("main.sanim", &sources, &BTreeMap::new())
                .expect("warm prepare"),
        );
    }
    println!("warm_prepare_mean_us={}", start.elapsed().as_micros() / 10);
    let start = Instant::now();
    for i in 0u64..100 {
        black_box(
            prepared
                .animation
                .sample(Fraction::new(i, 100u64))
                .expect("seek"),
        );
    }
    println!("random_seek_mean_us={}", start.elapsed().as_micros() / 100);
    black_box(Value::Number(Number::scalar(1)));
    let mut csv = String::from("group,value\n");
    for i in 0..10_000 {
        csv.push_str(&format!("g{},{}\n", i % 10, i));
    }
    sources.insert("data.csv", csv);
    sources.insert("data.sanim","type Sample = { group: String, value: Number }\nlet samples=read<List<Sample>>(\"data.csv\")\nlet totals=query(\"SELECT SUM(value) AS total FROM samples GROUP BY \\\"group\\\" ORDER BY \\\"group\\\"\", tables: {samples: samples})\n");
    let start = Instant::now();
    let data = engine
        .prepare("data.sanim", &sources, &BTreeMap::new())
        .expect("data benchmark");
    println!(
        "data_10000_cold_us={} stats={:?}",
        start.elapsed().as_micros(),
        data.statistics
    );
    let start = Instant::now();
    let warm = engine
        .prepare("data.sanim", &sources, &BTreeMap::new())
        .expect("warm data benchmark");
    println!(
        "data_10000_warm_us={} stats={:?}",
        start.elapsed().as_micros(),
        warm.statistics
    );
}
