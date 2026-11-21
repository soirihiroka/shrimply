use sanim_core::Value;
use sanim_lang::{Engine, MemorySources};
use std::{collections::BTreeMap, sync::Arc};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &sources, &BTreeMap::new())
}

#[test]
fn native_random_and_density_are_typed_pure_and_ergonomic() {
    let prepared = prepare(
        r#"
let rng = Random(seed=42)
let original = rng.number(17)
let unrelated = rng.number(99)
assert(original == rng.number(17))
for index in range(30) {
    let chosen = rng.integer(index, min=-3, max=5)
    assert(chosen >= -3 and chosen < 5)
}

assert(rng.chance(0,probability=0) == false)
assert(rng.chance(0,probability=1) == true)
let hill = Noise(seed=73,kind="fbm",scale=12,octaves=3).multiply(2).add(6)
let valley = Distance(center=[8,0,8],axes="xz").divide(8).clamp().pow(2)
let field: Density = hill.add(valley)
fn sample(sample_field: Density): Number { return sample_field.sample([3,0,7]) }
let value = sample(field)
let shifted = Density(axis="x").translate([4,0,0]).scale(2)
assert(shifted.sample([12,0,0]) == 2)
Animation()
"#,
    )
    .unwrap();
    assert!(matches!(prepared.bindings["field"], Value::Density(_)));
    assert!(matches!(prepared.bindings["rng"], Value::Random(42)));
}

#[test]
fn planar_noise_warps_and_smooth_masks_share_the_native_field_api() {
    prepare(
        r#"
let climate = (Noise(seed=71, axes="xz", scale=18, octaves=4)
    .warp(seed=7, axes="xz", scale=35, strength=6))
let wetness = climate.smoothstep(min=-0.3,max=0.3)
assert(wetness.sample([13.17,0,25.61]) == wetness.sample([13.17,500,25.61]))
let vertical = Density(axis="y").warp(seed=7,axes="xz",strength=12)
assert(vertical.sample([13,22,25]) == 22)
let shore = Density(axis="y").smoothstep(10,20)
assert(shore.sample([0,0,0]) == 0)
assert(shore.sample([0,15,0]) == 0.5)
assert(shore.sample([0,30,0]) == 1)
let disk = Distance(center=[10,0,10],axes="xz").warp(axes="xz",strength=4)
assert(disk.sample([3,0,7]) == disk.sample([3,500,7]))
Animation()
"#,
    )
    .unwrap();
    for (source, message) in [
        ("let x=Noise(axes=\"yx\")\nAnimation()", "yx"),
        (
            "let x=Density(axis=\"x\").warp(axes=\"x\")\nAnimation()",
            "x",
        ),
        (
            "let x=Density(axis=\"x\").smoothstep(2,1)\nAnimation()",
            "min < max",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(message), "{error}");
    }
}

#[test]
fn heightmap_density_surface_points_and_repeated_assets_work_without_lists_per_cell() {
    let prepared = prepare(
        r#"
let grass=Color("green3")
let height=Noise(seed=7,kind="ridged",scale=20,octaves=2).multiply(2).add(8)
let terrain=VoxelGrid(size=[32,16,32]).heightmap(height,Color("dark3")).surface(grass,depth=1)
let masked=terrain.surface(Color("orange3"),depth=1,mask=Density(axis="x").subtract(16))
let candidates=terrain.surface_points(step=8)
assert(len(candidates) == 16)
let flooded=terrain.flood(level=10,material=Color("blue3"))
assert(flooded.get([0,9,0]) != None)
let tree=VoxelGrid(size=[2,4,2]).fill([0,0,0],[2,4,2],Color("orange3"))
let world=terrain.place_many(tree,[[0,10,0],[16,10,16]])
let sphere=Density(value=5).subtract(Distance(center=[8,8,8]))
let carved=VoxelGrid(size=[16,16,16]).density(sphere,Color("blue3"))
let scene=VoxelScene(volume=world)
scene.fade_in()
wait(1s)
"#,
    )
    .unwrap();
    let Value::VoxelGrid(world) = &prepared.bindings["world"] else {
        panic!("grid")
    };
    assert!(world.occupied_voxels() > 32 * 7 * 32);
    let Value::VoxelGrid(sphere) = &prepared.bindings["carved"] else {
        panic!("sphere")
    };
    assert_eq!(sphere.get([0; 3]).unwrap(), 0);
    assert_ne!(sphere.get([8; 3]).unwrap(), 0);
}

#[test]
fn native_surface_selection_carving_and_pattern_fields_are_typed_and_pure() {
    prepare(
        r#"
let ground = VoxelGrid(size=[24,16,24]).heightmap(Density(value=8),Color("green3"))
let mask = Density(axis="x").subtract(10)
let positions = ground.surface_points(step=4,seed=71,jitter=0.8,density=1,mask=mask,max_slope=0)
assert(len(positions) > 0)
for point in positions { assert(point[0] >= 10 and point[1] == 8) }
let sphere = Density(value=4).subtract(Distance(center=[12,5,12]))
let hollow = ground.carve(sphere,min=[8,1,8],max=[16,9,16])
assert(hollow.get([12,5,12]) == None)
assert(ground.get([12,5,12]) != None)
let stripes = Density(axis="x").divide(4).floor().mod(2)
assert(stripes.sample([-1,0,0]) == 1)
let fraction = Density(axis="x").fract()
assert(fraction.sample([-1.25,0,0]) == 0.75)
assert(Density(axis="x").sin().sample([0,0,0]) == 0)
assert(Density(axis="x").cos().sample([0,0,0]) == 1)
Animation()
"#,
    )
    .unwrap();
    for (source, message) in [
        (
            "let p=VoxelGrid(size=[8,8,8]).surface_points(jitter=2)\nAnimation()",
            "jitter",
        ),
        (
            "let p=VoxelGrid(size=[8,8,8]).surface_points(max_slope=-1)\nAnimation()",
            "max_slope",
        ),
        (
            "let p=Density(axis=\"x\").mod(0).sample([1,0,0])\nAnimation()",
            "modulo divisor",
        ),
    ] {
        assert!(prepare(source).unwrap_err().message.contains(message));
    }
}

#[test]
fn procedural_errors_name_the_bad_parameter_and_reserve_work_before_sampling() {
    for (source, message) in [
        ("let x=Noise(octaves=99)\nAnimation()", "octaves"),
        ("let x=Noise(scale=0)\nAnimation()", "scale"),
        ("let x=Density(axis=\"q\")\nAnimation()", "q"),
        (
            "let x=Random().integer(0,min=2,max=2)\nAnimation()",
            "min < max",
        ),
        (
            "let x=Random().integer(0,min=1.000000000000000001,max=2)\nAnimation()",
            "bounds must be integers",
        ),
        (
            "let x=Density(value=1).divide(0).sample([1,2,3])\nAnimation()",
            "non-finite",
        ),
        (
            "let x=VoxelGrid(size=[512,512,512]).density(Noise(),Color(\"blue3\"))\nAnimation()",
            "work",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(message), "{error}");
    }
}

#[test]
fn changing_only_timeline_suffix_preserves_field_and_raster_snapshots() {
    let mut engine = Engine::default();
    engine.limits.max_cache_entries = 32;
    engine.limits.max_cache_bytes = 2 * 1024 * 1024;
    let mut sources = MemorySources::default();
    let source = "let field=Noise(seed=42,axes=\"xz\").warp(axes=\"xz\",strength=4).smoothstep(-0.5,0.5).multiply(2).add(8)\nlet terrain=VoxelGrid(size=[16,16,16]).heightmap(field,Color(\"green3\"))\nlet scene=VoxelScene(volume=terrain)\nscene.fade_in()\nwait(1s)";
    sources.insert("main.sanim", source);
    let original = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    sources.insert("main.sanim", source.replace("wait(1s)", "wait(2s)"));
    let changed = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    let (Value::Density(a), Value::Density(b)) =
        (&original.bindings["field"], &changed.bindings["field"])
    else {
        panic!("field")
    };
    assert!(Arc::ptr_eq(a, b));
    let weak_field = Arc::downgrade(a);
    let (Value::VoxelGrid(a), Value::VoxelGrid(b)) =
        (&original.bindings["terrain"], &changed.bindings["terrain"])
    else {
        panic!("volume")
    };
    assert!(Arc::ptr_eq(a, b));
    let weak_volume = Arc::downgrade(a);
    let mut retained = Vec::new();
    for seconds in 3..131 {
        sources.insert(
            "main.sanim",
            source.replace("wait(1s)", &format!("wait({seconds}s)")),
        );
        let next = engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        let Value::Density(field) = &next.bindings["field"] else {
            panic!("field")
        };
        let Value::VoxelGrid(volume) = &next.bindings["terrain"] else {
            panic!("volume")
        };
        assert!(Arc::ptr_eq(&weak_field.upgrade().unwrap(), field));
        assert!(Arc::ptr_eq(a, volume));
        if seconds % 32 == 0 {
            retained.push((Arc::strong_count(field), Arc::strong_count(volume)));
        }
    }
    assert_eq!(
        retained[2], retained[3],
        "edit history must reach a bounded ownership plateau: {retained:?}"
    );
    drop(original);
    drop(changed);
    drop(engine);
    assert!(
        weak_field.upgrade().is_none(),
        "field outlived every owning engine/snapshot"
    );
    assert!(
        weak_volume.upgrade().is_none(),
        "raster outlived every owning engine/snapshot"
    );
}

#[test]
fn reusable_instances_and_native_scatter_keep_typed_asset_identity() {
    let prepared = prepare(
        r#"
let terrain=VoxelGrid(size=[32,20,32]).heightmap(Density(value=8),Color("green3"))
let grass=terrain.scatter(Color("green4"),density=0.2,seed=42)
let tree=VoxelGrid(size=[2,4,2]).fill([0,0,0],[2,4,2],Color("orange3"))
let grove=Instances(asset=tree,positions=[[0,8,0],[16,8,16]],size=[32,20,32])
let empty=Instances(asset=tree,positions=[],size=[32,20,32])
let world=<VoxelBox size={[32,20,32]}><VoxelPart volume={grass} absolute/>{grove}{empty}</VoxelBox>
let scene=VoxelScene(layout=world)
scene.fade_in()
wait(1s)
"#,
    )
    .unwrap();
    let Value::VoxelLayout(grove) = &prepared.bindings["grove"] else {
        panic!("instances")
    };
    let Value::VoxelGrid(tree) = &prepared.bindings["tree"] else {
        panic!("asset")
    };
    assert_eq!(grove.children().unwrap().len(), 2);
    assert!(
        grove
            .children()
            .unwrap()
            .iter()
            .all(|child| Arc::ptr_eq(child.volume().unwrap(), tree))
    );
}

#[test]
fn documented_procedural_world_is_runnable() {
    let docs = include_str!("../../../../docs/source/sanim/voxels.rst");
    let section = docs
        .split("Procedural fields and reusable assets\n")
        .nth(1)
        .unwrap();
    let code = section
        .split(".. code-block:: text\n\n")
        .nth(1)
        .unwrap()
        .split("\nNoise kinds")
        .next()
        .unwrap();
    let source = code
        .lines()
        .map(|line| line.strip_prefix("   ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let prepared = prepare(&source).unwrap();
    assert!(matches!(&prepared.bindings["heights"], Value::Density(_)));
    assert!(matches!(&prepared.bindings["trees"], Value::VoxelLayout(_)));
}

#[test]
fn native_work_is_bounded_cumulatively_and_restored_independently_of_dsl_steps() {
    let prefix = "let base=VoxelGrid(size=[64,32,64])\nlet a=base.heightmap(Density(value=8),Color(\"green3\"))\n";
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{prefix}Animation()"));
    let mut engine = Engine::default();
    engine.limits.max_operations = 200;
    engine.limits.max_native_work = 6000;
    let prepared = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(prepared.statistics.native_work > engine.limits.max_operations);
    sources.insert(
        "main.sanim",
        format!("{prefix}let b=base.heightmap(Density(value=9),Color(\"blue3\"))\nAnimation()"),
    );
    let error = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("native work budget"), "{error}");
}

#[test]
fn animation_lowering_shares_the_remaining_native_budget() {
    let prefix = "let grid=VoxelGrid(size=[8,8,8]).heightmap(Density(value=8),Color(\"green3\"))\n";
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{prefix}Animation()"));
    let mut engine = Engine::default();
    let original = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    engine.limits.max_native_work = original.statistics.native_work;
    sources.insert(
        "main.sanim",
        format!("{prefix}let visual=<Box w-10 h-10/>\nvisual.fade_in()"),
    );
    let error = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("native preparation budget"),
        "{error}"
    );
}

const LANDSCAPE: &str = r#"
let height = Noise(seed=42,scale=30,octaves=4).multiply(10).add(30)
let terrain = VoxelGrid(size=[176,96,176]).heightmap(height,Color("dark3")).surface(Color("green3"))
let wet = terrain.flood(level=25,material=Color("blue3"))
let plants = wet.scatter(Color("green4"),density=0.2,seed=73,step=2)
Animation()
"#;

#[test]
fn realistic_native_landscape_does_not_consume_interpreter_steps_per_cell() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", LANDSCAPE);
    let mut engine = Engine::default();
    engine.limits.max_operations = 1000;
    let prepared = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(prepared.statistics.native_work > 1_000_000);
    assert!(prepared.statistics.operations - prepared.statistics.native_work < 1000);
    assert_eq!(
        prepared.statistics.interpreted_operations,
        prepared.statistics.operations - prepared.statistics.native_work
    );
    assert!(
        !prepared
            .performance
            .iter()
            .any(|warning| warning.code == "P001"),
        "native cell work must not be mislabeled as interpreted operations"
    );
}

#[test]
#[ignore = "manual representative native landscape preparation and suffix reload benchmark"]
fn native_landscape_preparation_benchmark() {
    use std::time::Instant;
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", LANDSCAPE);
    let mut engine = Engine::default();
    let start = Instant::now();
    let prepared = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    let cold = start.elapsed();
    let Value::VoxelGrid(original) = &prepared.bindings["plants"] else {
        panic!("grid")
    };
    let mut times = Vec::new();
    for seconds in 1..8 {
        sources.insert(
            "main.sanim",
            LANDSCAPE.replace("Animation()", &format!("wait({seconds}s)")),
        );
        let start = Instant::now();
        let changed = engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        times.push(start.elapsed());
        let Value::VoxelGrid(grid) = &changed.bindings["plants"] else {
            panic!("grid")
        };
        assert!(Arc::ptr_eq(original, grid));
        assert_eq!(
            changed.statistics.native_work, 0,
            "suffix edits must not rerasterize native geometry"
        );
    }
    times.sort();
    eprintln!(
        "176x96x176 noise/terrain/surface/flood/scatter: cold_us={} warm_suffix_us={} native_work={} voxel_payload_bytes={} metadata_bytes={}",
        cold.as_micros(),
        times[3].as_micros(),
        prepared.statistics.native_work,
        original.resident_bytes(),
        original.storage_metadata_bytes_since(None)
    );
    assert!(
        times[3] < cold,
        "restoring immutable native outputs must earn a real benefit"
    );
}

#[test]
#[ignore = "manual same-result native selection versus repeated DSL field calls"]
fn native_surface_selection_benchmark() {
    use std::time::Instant;
    const PREFIX: &str = r#"
let terrain=VoxelGrid(size=[176,32,176]).heightmap(Density(value=8),Color("green3"))
let probability=Noise(seed=19,axes="xz",scale=24).warp(seed=71,axes="xz",strength=5).add(1).divide(2)
let mask=Density(axis="x").subtract(60)
"#;
    let choices = [
        "let selected=terrain.surface_points(step=4,seed=42,density=probability,mask=mask,max_slope=0)\nAnimation()",
        r#"
let rng=Random(seed=42)
mut selected:List<List<Number,3>>=[]
for point in terrain.surface_points(step=4) {
    let sample=[point[0]+0.5,point[1],point[2]+0.5]
    if mask.sample(sample)>0 and rng.chance(point[0]*4294967296+point[2],probability=probability.sample(sample)) {
        selected=append(selected,point)
    }
}
Animation()
"#,
    ];
    let mut reference = None;
    let mut medians = Vec::new();
    for (variant, suffix) in choices.into_iter().enumerate() {
        let mut times = Vec::new();
        for _ in 0..3 {
            let mut engine = Engine::default();
            let mut sources = MemorySources::default();
            sources.insert("main.sanim", format!("{PREFIX}Animation()"));
            engine
                .prepare("main.sanim", &sources, &BTreeMap::new())
                .unwrap();
            sources.insert("main.sanim", format!("{PREFIX}{suffix}"));
            let start = Instant::now();
            let prepared = engine
                .prepare("main.sanim", &sources, &BTreeMap::new())
                .unwrap();
            times.push(start.elapsed());
            let selected = &prepared.bindings["selected"];
            if let Some(reference) = &reference {
                assert_eq!(
                    reference, selected,
                    "native selection must match equivalent repeated DSL sampling"
                );
            } else {
                reference = Some(selected.clone());
            }
            eprintln!(
                "surface selection variant={variant} points={} interpreted_ops={} native_work={}",
                selected.list().unwrap().len(),
                prepared.statistics.interpreted_operations,
                prepared.statistics.native_work
            );
        }
        times.sort_unstable();
        medians.push(times[1]);
    }
    eprintln!(
        "surface selection native_us={} repeated_dsl_us={} speedup={:.2}",
        medians[0].as_micros(),
        medians[1].as_micros(),
        medians[1].as_secs_f64() / medians[0].as_secs_f64()
    );
    assert!(
        medians[0] < medians[1],
        "native selection must earn its additional API"
    );
}
