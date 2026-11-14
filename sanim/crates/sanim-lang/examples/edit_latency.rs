//! Stable snapshots isolate compiler/evaluator latency from disk and resource hashing.
use sanim_lang::{Engine, Sources, Statistics};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

#[derive(Default)]
struct Files(BTreeMap<String, (Arc<Vec<u8>>, String)>);
impl Files {
    fn set(&mut self, name: &str, content: String) {
        let bytes = Arc::new(content.into_bytes());
        let hash = sanim_core::digest(&bytes);
        self.0.insert(name.into(), (bytes, hash));
    }
}
impl Sources for Files {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read_snapshot(path)
            .map(|(bytes, _)| bytes.as_ref().clone())
    }
    fn read_snapshot(&self, path: &str) -> Result<(Arc<Vec<u8>>, String), String> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing {path}"))
    }
}
fn report(name: &str, micros: &[u128], statistics: &Statistics) {
    let mut sorted = micros.to_vec();
    sorted.sort_unstable();
    println!(
        "{name}: median_us={} p95_us={} operations={} restored={} parsed={} reused_modules={} checked={} reused_checks={} queries={} decodes={} cache_key_us={} native_us={} parse_us={} check_us={} compiler_cache_us={}",
        sorted[sorted.len() / 2],
        sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)],
        statistics.operations,
        statistics.restored_statements,
        statistics.parsed_modules,
        statistics.reused_modules,
        statistics.checked_modules,
        statistics.reused_checks,
        statistics.executed_queries,
        statistics.decoded_assets,
        statistics.cache_key_micros,
        statistics.native_micros,
        statistics.parse_micros,
        statistics.check_micros,
        statistics.compiler_cache_micros
    );
}
fn run(name: &str, files: &mut Files, variants: &[String]) -> Result<(), String> {
    const SAMPLES: usize = 20;
    let mut engine = Engine::default();
    files.set("main.sanim", variants[0].clone());
    let started = Instant::now();
    let cold = engine
        .prepare("main.sanim", files, &BTreeMap::new())
        .map_err(|e| e.to_string())?;
    report(
        &format!("{name}/cold"),
        &[started.elapsed().as_micros()],
        &cold.statistics,
    );
    let mut times = Vec::with_capacity(SAMPLES);
    let mut statistics = Statistics::default();
    for index in 0..SAMPLES {
        files.set(
            "main.sanim",
            format!("{}\n# edit {index}\n", variants[index % variants.len()]),
        );
        let started = Instant::now();
        let prepared = engine
            .prepare("main.sanim", files, &BTreeMap::new())
            .map_err(|e| e.to_string())?;
        times.push(started.elapsed().as_micros());
        statistics = prepared.statistics;
    }
    report(name, &times, &statistics);
    Ok(())
}
fn main() -> Result<(), String> {
    let mut files = Files::default();
    let helpers = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(40);
    let library = (0..helpers)
        .map(|index| format!("fn helper_{index}(value:Number):Number{{return value+{index}}}\n"))
        .collect::<String>();
    files.set("helpers.sanim", library);
    let program =
        "import \"helpers\" as helpers\nlet marker=Circle(radius=20pt)\nadd(marker)\nwait(1s)\n";
    run(
        "imported-functions/tail",
        &mut files,
        &[format!("{program}wait(3s)"), format!("{program}wait(4s)")],
    )?;

    files.set(
        "data.json",
        format!(
            "[{}]",
            (0..10_000)
                .map(|i| format!("{{\"value\":{i}}}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    );
    let data = "type Row={value:Number}\nlet rows=read<List<Row>>(\"data.json\")\nlet result=query(\"SELECT sum(value) AS total FROM rows\",tables={rows=rows})\n";
    run(
        "data/tail",
        &mut files,
        &[format!("{data}wait(3s)"), format!("{data}wait(4s)")],
    )?;
    run(
        "data/early-color",
        &mut files,
        &[
            format!("let tint=Color(\"blue3\")\n{data}add(Circle(fill=tint))\nwait(3s)"),
            format!("let tint=Color(\"orange3\")\n{data}add(Circle(fill=tint))\nwait(3s)"),
        ],
    )?;

    let scene = (0..200)
        .map(|i| {
            format!("let item_{i}=Circle(radius=2pt,position=Vec2({i}pt,0pt))\nadd(item_{i})\n")
        })
        .collect::<String>();
    run(
        "many-elements/tail",
        &mut files,
        &[format!("{scene}wait(3s)"), format!("{scene}wait(4s)")],
    )?;
    run(
        "many-elements/late-radius",
        &mut files,
        &[
            format!("{scene}let focus=Circle(radius=32pt)\nadd(focus)\nwait(3s)"),
            format!("{scene}let focus=Circle(radius=48pt)\nadd(focus)\nwait(3s)"),
        ],
    )?;
    run(
        "many-elements/leading-comment",
        &mut files,
        &[
            format!("# a\n{scene}wait(3s)"),
            format!("# a longer comment\n{scene}wait(3s)"),
        ],
    )?;
    Ok(())
}
