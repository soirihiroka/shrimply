use sanim_lang::{Engine, MemorySources};
fn lint(source: &str) -> sanim_lang::LintReport {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().lint("main.sanim", &sources).unwrap()
}
fn conflicts(source: &str) -> usize {
    lint(source)
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "V101")
        .count()
}

#[test]
fn duplicate_absolute_recipes_warn_without_reading_or_evaluating_resources() {
    let report = lint(
        r#"
let unused=read<String>("absent.txt")
let grid=VoxelGrid(size=[4,4,4])
let part=<VoxelPart volume={grid} position_mode="absolute" at={[2,0,3]}/>
let world=<VoxelBox size={[16,16,16]}>{part}{part}</VoxelBox>
Animation()
"#,
    );
    assert_eq!(report.statistics.decoded_assets, 0);
    assert_eq!(report.statistics.evaluated_expressions, 0);
    let diagnostics = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "V101")
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0].message.contains("same placement")
            && diagnostics[0].message.contains("assert_disjoint")
    );
    assert_eq!(diagnostics[0].span.line, 5);
    assert_eq!(
        conflicts(
            r#"let grid=VoxelGrid(size=[4,4,4])
let world=<VoxelBox><VoxelPart volume={grid} position_mode="absolute" at={[1,0,1]}/><VoxelPart at={[1,0,1]} position_mode="absolute" volume={grid}/></VoxelBox>
Animation()"#
        ),
        1
    );
}

#[test]
fn flow_layout_dynamic_placements_and_distinct_grids_are_not_guessed() {
    for source in [
        "let grid=VoxelGrid(size=[4,4,4]);let part=<VoxelPart volume={grid}/>;let world=<VoxelBox>{part}{part}</VoxelBox>;Animation()",
        "let grid=VoxelGrid(size=[4,4,4]);let world=<VoxelBox><VoxelPart volume={grid} position_mode=\"absolute\" at={[0,0,0]}/><VoxelPart volume={grid} position_mode=\"absolute\" at={[1,0,0]}/></VoxelBox>;Animation()",
        "let a=VoxelGrid(size=[4,4,4]);let b=VoxelGrid(size=[4,4,4]);let world=<VoxelBox><VoxelPart volume={a} position_mode=\"absolute\"/><VoxelPart volume={b} position_mode=\"absolute\"/></VoxelBox>;Animation()",
        "mut grid=VoxelGrid(size=[4,4,4]);let world=<VoxelBox><VoxelPart volume={grid} position_mode=\"absolute\"/><VoxelPart volume={grid} position_mode=\"absolute\"/></VoxelBox>;Animation()",
    ] {
        assert_eq!(conflicts(source), 0, "{source}");
    }
}

#[test]
fn recipe_keys_preserve_captured_bindings_and_respect_lexical_scope() {
    assert_eq!(
        conflicts(
            r#"
let grid=VoxelGrid(size=[4,4,4])
let part=<VoxelPart volume={grid} position_mode="absolute"/>
fn build(other:VoxelGrid):VoxelLayout {
    return <VoxelBox>{part}<VoxelPart volume={other} position_mode="absolute"/></VoxelBox>
}
Animation()
"#
        ),
        0
    );
    assert_eq!(
        conflicts(
            r#"
fn build(grid:VoxelGrid):VoxelLayout {
    let part=<VoxelPart volume={grid} position_mode="absolute"/>
    return <VoxelBox>{part}{part}</VoxelBox>
}
Animation()
"#
        ),
        1
    );
}

#[test]
#[ignore = "manual static voxel lint scalability measurement"]
fn voxel_lint_benchmark() {
    use std::time::Instant;
    for count in [100, 1000] {
        let children = "{part}".repeat(count);
        let source = format!(
            "let grid=VoxelGrid(size=[4,4,4]);let part=<VoxelPart volume={{grid}} position_mode=\"absolute\"/>;let world=<VoxelBox>{children}</VoxelBox>;Animation()"
        );
        let _warmup = lint(&source);
        let mut samples = Vec::new();
        for _ in 0..9 {
            let started = Instant::now();
            let report = lint(&source);
            samples.push(started.elapsed());
            assert_eq!(
                report
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code == "V101")
                    .count(),
                count - 1
            );
            assert_eq!(report.statistics.evaluated_expressions, 0);
        }
        samples.sort();
        eprintln!(
            "{count} duplicate parts: median warmed full parse+typecheck+lint {:?}",
            samples[samples.len() / 2]
        );
    }
}
