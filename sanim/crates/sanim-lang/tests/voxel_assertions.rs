use sanim_lang::{Engine, MemorySources};
fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}

const PARTS: &str = r#"
let shell_grid=VoxelGrid(size=[8,8,8]).fill([0,0,0],[8,8,8],Color("orange3")).erase([1,1,1],[7,7,7])
let first=<VoxelPart volume={shell_grid} absolute />
let second=<VoxelPart volume={VoxelGrid(size=[2,2,2]).fill([0,0,0],[2,2,2],Color("blue3"))} absolute at={[3,3,3]} />
let world=<VoxelBox size={[16,16,16]}>{first}{second}</VoxelBox>
"#;

#[test]
fn pure_assertions_report_source_paths_cells_and_repairs() {
    prepare(&format!("{PARTS}\nworld.assert_disjoint(first,second)\nworld.assert_within(second,world)\nAnimation()")).unwrap();
    let collision = PARTS.replace("at={[3,3,3]}", "at={[0,0,0]}");
    let error = prepare(&format!(
        "{collision}\nworld.assert_disjoint(first,second)\nAnimation()"
    ))
    .unwrap_err();
    assert_eq!(error.code, "S305");
    assert!(
        error.message.contains("V001")
            && error.message.contains("child[0]")
            && error.message.contains("child[1]")
            && error.message.contains("[0, 0, 0]")
            && error.message.contains("move one part"),
        "{error}"
    );
    assert!(error.span.end > error.span.start);
}

#[test]
fn assertions_are_allowed_inside_pure_functions_and_preserve_memo_semantics() {
    let prepared = prepare(&format!(
        r#"{PARTS}
fn validate(root: VoxelLayout, a: VoxelLayout, b: VoxelLayout): Number {{
    root.assert_disjoint(a,b)
    return 1
}}
let valid=validate(world,first,second)
let first_copy=<VoxelPart volume={{shell_grid}} absolute />
let valid_copy=validate(world,first_copy,second)
Animation()
"#
    ))
    .unwrap();
    assert!(
        prepared.statistics.reused_expressions > 0,
        "memoized validation should reuse the successful semantic result"
    );
    let error = prepare(&format!(
        "{PARTS}\nworld.assert_within(first,second)\nAnimation()"
    ))
    .unwrap_err();
    assert!(
        error.message.contains("V002") && error.message.contains("bounds"),
        "{error}"
    );
}

#[test]
fn camera_snapshot_assertion_checks_external_viewport_and_margin() {
    prepare("let scene=VoxelScene(volume=VoxelGrid(size=[16,16,16]))\nscene.assert_framed(margin=0%)\nAnimation()").unwrap();
    let error=prepare("let scene=VoxelScene(volume=VoxelGrid(size=[16,16,16]),zoom=8)\nscene.assert_framed()\nAnimation()").unwrap_err();
    assert!(
        error.message.contains("V003") && error.message.contains("lower zoom"),
        "{error}"
    );
    let error = prepare("let box=<Box/>\nbox.assert_framed()\nAnimation()").unwrap_err();
    assert!(error.message.contains("VoxelScene"), "{error}");
}

#[test]
fn strict_framing_contract_is_typed_reflectable_and_preserved_by_snapshots() {
    let prepared=prepare("let scene=VoxelScene(volume=VoxelGrid(size=[16,16,16]),check_framing=true,frame_margin=5%)\nscene.fade_in()\n").unwrap();
    let sanim_core::Value::Element(element) = &prepared.bindings["scene"] else {
        panic!("scene");
    };
    let sanim_core::Geometry::Voxel(scene) = element.geometry.as_ref() else {
        panic!("voxel");
    };
    assert!(scene.check_framing);
    assert_eq!(scene.frame_margin, sanim_core::Fraction::new(1u64, 20u64));
    let error = prepare(
        "let scene=VoxelScene(volume=VoxelGrid(size=[16,16,16]),frame_margin=50%)\nAnimation()",
    )
    .unwrap_err();
    assert!(error.message.contains("between 0% and 50%"), "{error}");
    let error = prepare(
        "let scene=VoxelScene(volume=VoxelGrid(size=[16,16,16]),check_framing=1)\nAnimation()",
    )
    .unwrap_err();
    assert!(error.message.contains("Bool"), "{error}");
}
