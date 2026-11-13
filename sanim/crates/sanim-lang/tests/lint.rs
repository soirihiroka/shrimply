use sanim_lang::{Engine, MemorySources};

#[test]
fn lint_checks_types_without_opening_runtime_resources() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let unused=read<List<Number>>(\"absent.json\")\nrepeat 3 {let values=read<List<Number>>(\"also-absent.json\");assert(len(values)>0)}\nwait(1s)");
    let mut engine = Engine::default();
    let report = engine.lint("main.sanim", &sources).unwrap();
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.statistics.decoded_assets, 0);
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|warning| warning.code == "P101")
            .count(),
        1
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|warning| warning.code == "L001")
    );
    assert!(
        engine
            .prepare("main.sanim", &sources, &Default::default())
            .is_err()
    );
    sources.insert("main.sanim", "let wrong:Number=\"text\"");
    let error = engine.lint("main.sanim", &sources).unwrap_err();
    assert_eq!(error.source, "main.sanim");
    assert!(error.span.line > 0);
}

#[test]
fn lint_uses_imports_and_visits_nested_functions_once() {
    let mut sources = MemorySources::default();
    sources.insert("lib.sanim", "fn load():List<Number> {repeat 3 {let values=read<List<Number>>(\"missing.json\")} return []}");
    sources.insert("main.sanim", "import \"lib.sanim\" as lib\nwait(1s)");
    let report = Engine::default().lint("main.sanim", &sources).unwrap();
    assert_eq!(report.dependencies.len(), 2);
    let warnings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|warning| warning.code == "P101")
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].source, "lib.sanim");
}

#[test]
fn static_repeat_advice_obeys_the_configured_operation_budget() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "repeat 10 { wait(1ms) }");
    let mut engine = Engine::new(Default::default());
    engine.limits.warn_operations = 5;
    assert!(
        engine
            .lint("main.sanim", &sources)
            .unwrap()
            .diagnostics
            .iter()
            .any(|warning| warning.code == "P102")
    );
    engine.limits.warn_operations = 20;
    assert!(
        engine
            .lint("main.sanim", &sources)
            .unwrap()
            .diagnostics
            .is_empty()
    );
}
