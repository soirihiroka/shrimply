use sanim_core::{Fraction, Geometry, Value};
use sanim_lang::{Engine, MemorySources};
use std::sync::Arc;

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}

#[test]
fn immutable_grid_edits_preserve_previous_snapshots_and_validate_materials() {
    let prepared = prepare(
        r#"
        let empty=VoxelGrid(size=[32,16,32])
        let grass=VoxelMaterial(color=Color("green3"))
        let ground=empty.fill(min=[0,0,0],max=[100%,25%,100%],material=grass)
        let cleared=ground.erase(min=[1,1,1],max=[2,2,2])
        let occupied=ground.get([1,1,1])
        let absent=cleared.get([1,1,1])
        let scene=VoxelScene(volume=ground)
        scene.fade_in(duration=100ms)
    "#,
    )
    .unwrap();
    let grid = |name: &str| {
        let Value::VoxelGrid(grid) = &prepared.bindings[name] else {
            panic!("grid")
        };
        grid
    };
    assert_eq!(grid("empty").occupied_voxels(), 0);
    assert_eq!(grid("ground").occupied_voxels(), 32 * 4 * 32);
    assert_eq!(grid("cleared").occupied_voxels(), 32 * 4 * 32 - 1);
    assert!(matches!(prepared.bindings["occupied"], Value::Some(_)));
    assert_eq!(prepared.bindings["absent"], Value::None);
    assert!(prepare("let grid=VoxelGrid(size=[8,8,8])\nlet bad=grid.set([8,0,0],Color(\"red\"))\nAnimation()").unwrap_err().message.contains("outside dimensions"));
    assert!(prepare("let grid=VoxelGrid(size=[8,8,8])\nlet bad=grid.fill([0,0,0],[8,8,8],Color(\"#ff000080\"))\nAnimation()").unwrap_err().message.contains("opaque"));
}

#[test]
fn camera_signals_use_the_existing_animation_clock_and_keep_volume_shared() {
    let prepared=prepare(r#"
        let grid=VoxelGrid(size=[16,16,16]).fill(min=[0,0,0],max=[16,8,16],material=Color("orange3"))
        mut yaw=0deg
        let scene=VoxelScene(volume=grid,yaw=yaw,zoom=1)
        scene.fade_in(duration=100ms)
        animate(yaw,to=90deg,duration=1s,ease="linear")
    "#).unwrap();
    let samples = [
        Fraction::new(1u64, 10u64),
        Fraction::new(6u64, 10u64),
        Fraction::new(11u64, 10u64),
    ]
    .map(|time| prepared.animation.sample(time).unwrap());
    let views = samples.each_ref().map(|elements| {
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel geometry")
        };
        scene
    });
    assert_eq!(views[0].camera.yaw_degrees, 0.);
    assert!((views[1].camera.yaw_degrees - 45.).abs() < 0.001);
    assert_eq!(views[2].camera.yaw_degrees, 90.);
    assert!(views[0].source.same_snapshot(&views[2].source));
}

#[test]
fn relative_placement_composes_parent_and_child_anchors_without_absolute_coordinates() {
    let prepared=prepare(r#"
        let child=VoxelGrid(size=[10,2,6]).fill(min=[0,0,0],max=[100%,100%,100%],material=Color("blue3"))
        let world=VoxelGrid(size=[40,20,40]).place(child,at=[50%,5,50%],anchor=[50%,0%,50%],offset=[0,1,0])
        let scene=VoxelScene(volume=world)
        scene.fade_in()
    "#).unwrap();
    let Value::VoxelGrid(world) = &prepared.bindings["world"] else {
        panic!("grid")
    };
    assert_ne!(world.get([15, 6, 17]).unwrap(), 0);
    assert_ne!(world.get([24, 7, 22]).unwrap(), 0);
    assert_eq!(world.get([14, 6, 17]).unwrap(), 0);
    assert_eq!(world.get([25, 7, 22]).unwrap(), 0);
    let error=prepare("let child=VoxelGrid(size=[10,2,6])\nlet bad=VoxelGrid(size=[4,4,4]).place(child,at=[0,0,0])\nAnimation()").unwrap_err();
    assert!(
        error.message.contains("placement")
            || error.message.contains("outside")
            || error.message.contains("fit"),
        "{error}"
    );
}

#[test]
fn camera_only_hot_edits_restore_the_same_immutable_voxel_payload() {
    let mut sources = MemorySources::default();
    let source = |yaw| {
        format!(
            "let grid=VoxelGrid(size=[64,32,64]).fill([0,0,0],[64,8,64],Color(\"green3\"))\nlet scene=VoxelScene(volume=grid,yaw={yaw}deg)\nscene.fade_in()\nwait(1s)"
        )
    };
    sources.insert("main.sanim", source(0));
    let mut engine = Engine::default();
    let first = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let Value::VoxelGrid(original) = &first.bindings["grid"] else {
        panic!("grid")
    };
    for yaw in 1..16 {
        sources.insert("main.sanim", source(yaw));
        let updated = engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        let Value::VoxelGrid(grid) = &updated.bindings["grid"] else {
            panic!("grid")
        };
        assert!(Arc::ptr_eq(original, grid));
        assert!(updated.statistics.restored_statements > 0);
    }
}

#[test]
fn scalar_edit_history_stays_bounded_and_releases_voxels_with_the_engine() {
    let mut sources = MemorySources::default();
    let source = |yaw| {
        format!(
            "let grid=VoxelGrid(size=[32,16,32]).fill([0,0,0],[32,8,32],Color(\"green3\"))\nlet scene=VoxelScene(volume=grid,yaw={yaw}deg)\nscene.fade_in()\nwait(1s)"
        )
    };
    let mut engine = Engine::default();
    engine.limits.max_cache_entries = 32;
    engine.limits.max_cache_bytes = 2 * 1024 * 1024;
    sources.insert("main.sanim", source(0));
    let first = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let Value::VoxelGrid(original) = &first.bindings["grid"] else {
        panic!("grid")
    };
    let weak = Arc::downgrade(original);
    let mut counts = Vec::new();
    for yaw in 1..257 {
        sources.insert("main.sanim", source(yaw));
        engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        if yaw % 64 == 0 {
            counts.push(Arc::strong_count(original));
        }
    }
    assert!(
        counts.iter().all(|count| *count < 96),
        "unbounded edit history: {counts:?}"
    );
    assert_eq!(
        counts[2], counts[3],
        "retained voxel ownership did not reach a plateau"
    );
    drop(first);
    drop(engine);
    assert!(
        weak.upgrade().is_none(),
        "voxel source survived its owning engine and snapshots"
    );
}

#[test]
fn shared_voxel_accounting_charges_only_changed_cow_bricks() {
    let volume = Arc::new(
        sanim_voxel::Volume::new([128, 128, 128])
            .unwrap()
            .fill_color([0, 0, 0], [128, 128, 128], 0x33aa55ff)
            .unwrap(),
    );
    let mut bytes = 32 * 1024 * 1024;
    let mut items = 100_000;
    let mut budget = sanim_core::value_budget::Budget::default();
    sanim_core::value_budget::charge_with_budget(
        &Value::VoxelGrid(volume.clone()),
        &mut bytes,
        &mut items,
        64,
        &mut budget,
    )
    .unwrap();
    let before = bytes;
    let next = Arc::new(volume.set([0, 0, 0], 0).unwrap());
    sanim_core::value_budget::charge_with_budget(
        &Value::VoxelGrid(next),
        &mut bytes,
        &mut items,
        64,
        &mut budget,
    )
    .unwrap();
    assert!(
        before - bytes < 32 * 1024,
        "one edit charged {} bytes",
        before - bytes
    );
    let key =
        serde_json::to_vec(&sanim_core::value_key::ValueKey(&Value::VoxelGrid(volume))).unwrap();
    assert!(key.len() < 128, "voxel cache key serialized its payload");
}

#[test]
fn uniform_chunk_aliases_still_charge_their_persistent_map_storage() {
    let volume = Arc::new(
        sanim_voxel::Volume::new([128, 128, 128])
            .unwrap()
            .fill_color([0, 0, 0], [128, 128, 128], 0x33aa55ff)
            .unwrap(),
    );
    let mut bytes = 16 * 1024 * 1024;
    let mut items = 100_000;
    sanim_core::value_budget::charge(
        &Value::VoxelGrid(volume.clone()),
        &mut bytes,
        &mut items,
        64,
    )
    .unwrap();
    let used = 16 * 1024 * 1024 - bytes;
    assert!(
        used >= volume.chunk_count() * 128,
        "uniform brick aliasing hid map metadata: {used} bytes for {} entries",
        volume.chunk_count()
    );
}

#[test]
fn incremental_voxel_equality_keeps_exact_content_semantics() {
    let volume = sanim_voxel::Volume::new([32, 16, 32])
        .unwrap()
        .fill_color([0, 0, 0], [32, 16, 32], 0x33aa55ff)
        .unwrap();
    let (volume, orange) = volume
        .with_material(Arc::new(sanim_voxel::Material::solid(0xff7800ff).unwrap()))
        .unwrap();
    let changed = volume.set([31, 15, 31], orange).unwrap();
    assert_ne!(volume, changed);
    assert_eq!(volume, changed.set([31, 15, 31], 1).unwrap());
    let restored: sanim_voxel::Volume =
        serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(changed, restored);
}

#[test]
#[ignore]
fn voxel_equality_delta_benchmark() {
    use std::hint::black_box;
    let volume = sanim_voxel::Volume::new([128; 3])
        .unwrap()
        .fill_color([0; 3], [128; 3], 0x33aa55ff)
        .unwrap();
    let (volume, orange) = volume
        .with_material(Arc::new(sanim_voxel::Material::solid(0xff7800ff).unwrap()))
        .unwrap();
    let changed = volume.set([127; 3], orange).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..10_000 {
        assert_ne!(black_box(&volume), black_box(&changed));
    }
    let incremental = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        assert!(
            !black_box(&volume)
                .chunks()
                .zip(black_box(&changed).chunks())
                .all(|((key, a), (other, b))| key == other && a == b)
        );
    }
    let scan = start.elapsed();
    println!(
        "last-brick value equality over {} bricks: incremental {}ns/query vs content scan {}ns/query",
        volume.chunk_count(),
        incremental.as_nanos() / 10_000,
        scan.as_nanos() / 100
    );
}

#[test]
fn independently_allocated_equal_voxels_are_not_mistaken_for_shared_memory() {
    let build = || {
        Arc::new(
            sanim_voxel::Volume::new([16, 8, 16])
                .unwrap()
                .fill_color([1, 1, 1], [15, 7, 15], 0x33aa55ff)
                .unwrap(),
        )
    };
    let first = build();
    let second = build();
    assert_eq!(first, second);
    let mut bytes = 8 * 1024 * 1024;
    let mut items = 100_000;
    let mut budget = sanim_core::value_budget::Budget::default();
    sanim_core::value_budget::charge_with_budget(
        &Value::VoxelGrid(first.clone()),
        &mut bytes,
        &mut items,
        64,
        &mut budget,
    )
    .unwrap();
    let before = bytes;
    sanim_core::value_budget::charge_with_budget(
        &Value::VoxelGrid(first),
        &mut bytes,
        &mut items,
        64,
        &mut budget,
    )
    .unwrap();
    let clone_cost = before - bytes;
    let before = bytes;
    sanim_core::value_budget::charge_with_budget(
        &Value::VoxelGrid(second.clone()),
        &mut bytes,
        &mut items,
        64,
        &mut budget,
    )
    .unwrap();
    let independent_cost = before - bytes;
    assert!(clone_cost < 256);
    assert!(
        independent_cost >= second.resident_bytes(),
        "independent chunks cost {independent_cost} bytes, resident {}",
        second.resident_bytes()
    );
}

#[test]
fn generous_host_data_budget_does_not_invalidate_empty_voxel_grids() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let scene=VoxelScene(volume=VoxelGrid(size=[8,8,8]))\nscene.fade_in()",
    );
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 512 * 1024 * 1024;
    engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
}

#[test]
fn voxel_scene_requires_one_typed_spatial_input() {
    let missing = prepare("let scene=VoxelScene()\nscene.fade_in()").unwrap_err();
    assert!(missing.message.contains("exactly one"), "{missing}");
    let both=prepare("let grid=VoxelGrid(size=[8,8,8])\nlet scene=VoxelScene(volume=grid,layout=VoxelPart(volume=grid))\nscene.fade_in()").unwrap_err();
    assert!(both.message.contains("exactly one"), "{both}");
}

#[test]
fn texture_reads_are_typed_watched_and_shared_across_material_faces() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[20, 80, 40, 255, 60, 100, 20, 255])
            .unwrap();
    }
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"let tile=read<Texture>("tiles/grass.png")
        let grass=VoxelMaterial(texture=tile)
        let scene=VoxelScene(volume=VoxelGrid(size=[8,8,8]).fill([0,0,0],[8,8,8],grass))
        scene.fade_in()
    "#,
    );
    sources.insert("tiles/grass.png", png);
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    assert!(prepared.dependencies.contains_key("tiles/grass.png"));
    let Value::VoxelMaterial(material) = &prepared.bindings["grass"] else {
        panic!("material")
    };
    let first = material.faces()[0].as_ref().unwrap();
    assert!(
        material
            .faces()
            .iter()
            .all(|face| Arc::ptr_eq(face.as_ref().unwrap(), first))
    );
}

#[test]
fn xml_layout_uses_three_axis_flow_and_exposes_resolved_placements() {
    let prepared=prepare(r#"
        let block=VoxelGrid(size=[8,4,8]).fill([0,0,0],[8,4,8],Color("blue3"))
        let layout=<VoxelBox size={[32,8,16]} flow="x" padding={2} justify="between" align={["start","center","center"]}>
            <VoxelPart volume={block}/>
            <VoxelPart volume={block}/>
        </VoxelBox>
        let scene=VoxelScene(layout=layout)
        scene.fade_in()
    "#).unwrap();
    let elements = prepared.animation.sample(Fraction::from(1)).unwrap();
    let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
        panic!("voxel scene")
    };
    assert_eq!(scene.dimensions(), [32, 8, 16]);
    let parts = scene
        .placements
        .iter()
        .filter(|placed| placed.leaf)
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].origin, [2, 2, 4]);
    assert_eq!(parts[1].origin, [22, 2, 4]);
}

#[test]
fn signal_layout_reflows_only_when_its_resolved_recipe_changes() {
    let prepared = prepare(
        r#"
        let block=VoxelGrid(size=[4,4,4]).fill([0,0,0],[4,4,4],Color("orange3"))
        mut width=16
        sig layout=<VoxelBox size={[width,8,8]} justify="between">
            <VoxelPart volume={block}/>
            <VoxelPart volume={block}/>
        </VoxelBox>
        let scene=VoxelScene(layout=layout)
        scene.fade_in(duration=100ms)
        animate(width,to=24,duration=1s,ease="linear")
        wait(1s)
    "#,
    )
    .unwrap();
    let samples = [
        Fraction::new(1u64, 10u64),
        Fraction::new(6u64, 10u64),
        Fraction::new(11u64, 10u64),
    ]
    .map(|time| prepared.animation.sample(time).unwrap());
    let scenes = samples.each_ref().map(|elements| {
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        scene
    });
    assert_eq!(scenes[0].dimensions(), [16, 8, 8]);
    assert_eq!(scenes[1].dimensions(), [20, 8, 8]);
    assert_eq!(scenes[2].dimensions(), [24, 8, 8]);
    assert_eq!(
        scenes[1]
            .placements
            .iter()
            .rfind(|part| part.leaf)
            .unwrap()
            .origin,
        [16, 0, 0]
    );
    let held = prepared
        .animation
        .sample(Fraction::new(15u64, 10u64))
        .unwrap();
    let Geometry::Voxel(held) = held[0].geometry.as_ref() else {
        panic!("voxel scene")
    };
    assert_eq!(held.dimensions(), [24, 8, 8]);
    // Other tests deliberately evict this process-wide bounded memo in parallel.
    // Exact Arc reuse is checked in the isolated core regression and benchmark.
    assert_eq!(held.source, scenes[2].source);
    for frame in 0..=60u64 {
        let elements = prepared
            .animation
            .sample(Fraction::new(1u64, 10u64) + Fraction::new(frame, 60u64))
            .unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        assert_eq!(
            scene.dimensions(),
            [16 + (8 * frame / 60) as u32, 8, 8],
            "animation frame {frame}/60"
        );
        if frame == 1 {
            assert!(
                scene.source.same_snapshot(&scenes[0].source),
                "sub-cell size changes should reuse their snapped layout"
            );
        }
    }
}

#[test]
fn fractional_animated_gap_and_padding_snap_without_rejecting_subframes() {
    let prepared = prepare(
        r#"
        let block=VoxelGrid(size=[4,2,2]).fill([0,0,0],[4,2,2],Color("blue3"))
        mut spacing=0
        sig layout=<VoxelBox size={[32,8,8]} gap={spacing} padding={spacing}>
            <VoxelPart volume={block}/>
            <VoxelPart volume={block}/>
        </VoxelBox>
        let scene=VoxelScene(layout=layout)
        scene.fade_in(duration=100ms)
        animate(spacing,to=2,duration=1s,ease="linear")
    "#,
    )
    .unwrap();
    for frame in [1, 2, 29, 30, 31, 59, 60u64] {
        let elements = prepared
            .animation
            .sample(Fraction::new(1u64, 10u64) + Fraction::new(frame, 60u64))
            .unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        let snapped = (2 * frame / 60) as u32;
        let parts = scene
            .placements
            .iter()
            .filter(|part| part.leaf)
            .collect::<Vec<_>>();
        assert_eq!(parts[0].origin, [snapped; 3], "animation frame {frame}/60");
        assert_eq!(
            parts[1].origin,
            [4 + 2 * snapped, snapped, snapped],
            "animation frame {frame}/60"
        );
    }
    let error =
        prepare("let scene=VoxelScene(volume=VoxelGrid(size=[16.25,8,8]))\nscene.fade_in()")
            .unwrap_err();
    assert!(error.message.contains("integer"), "{error}");
    let error = prepare(
        "let scene=VoxelScene(layout=VoxelBox(size=[8,8,8],padding=-0.25))\nscene.fade_in()",
    )
    .unwrap_err();
    assert!(error.message.contains("nonnegative"), "{error}");
}

#[test]
fn fractional_absolute_positions_and_anchors_round_only_the_final_origin() {
    let prepared=prepare(r#"
        let block=VoxelGrid(size=[1,1,1]).fill([0,0,0],[1,1,1],Color("orange3"))
        mut x=1.9
        sig layout=<VoxelBox size={[8,8,8]}>
            <VoxelPart volume={block} position_mode="absolute" at={[x,0,0]} anchor={[0.2,0,0]} offset={[0.4,0,0]}/>
        </VoxelBox>
        let scene=VoxelScene(layout=layout)
        scene.fade_in(duration=100ms)
        animate(x,to=2.9,duration=1s,ease="linear")
    "#).unwrap();
    for frame in [0, 1, 2, 30, 53, 54, 55, 60u64] {
        let elements = prepared
            .animation
            .sample(Fraction::new(1u64, 10u64) + Fraction::new(frame, 60u64))
            .unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        assert_eq!(
            scene
                .placements
                .iter()
                .find(|part| part.leaf)
                .unwrap()
                .origin,
            [(126 + frame) as u32 / 60, 0, 0],
            "animation frame {frame}/60"
        );
    }
}

#[test]
fn fractional_flow_alignment_and_offsets_round_the_combined_position() {
    let prepared = prepare(
        r#"
        let block=VoxelGrid(size=[2,2,2]).fill([0,0,0],[2,2,2],Color("blue3"))
        mut shift=0
        sig layout=<VoxelBox size={[9,9,9]} justify="center" align={["center","center","center"]}>
            <VoxelPart volume={block} offset={[shift,shift,shift]}/>
        </VoxelBox>
        let scene=VoxelScene(layout=layout)
        scene.fade_in(duration=100ms)
        animate(shift,to=0.8,duration=1s,ease="linear")
    "#,
    )
    .unwrap();
    for frame in [0, 1, 37, 38, 59, 60u64] {
        let elements = prepared
            .animation
            .sample(Fraction::new(1u64, 10u64) + Fraction::new(frame, 60u64))
            .unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        let position = if frame < 38 { 3 } else { 4 };
        assert_eq!(
            scene
                .placements
                .iter()
                .find(|part| part.leaf)
                .unwrap()
                .origin,
            [position; 3],
            "animation frame {frame}/60"
        );
    }
}

#[test]
fn pure_layout_memo_is_bounded_and_does_not_retain_source_trees() {
    use sanim_voxel::layout::{Node, Placement};
    let node = Arc::new(
        Node::part(
            Arc::new(sanim_voxel::Volume::new([4, 4, 4]).unwrap()),
            Placement::default(),
        )
        .unwrap(),
    );
    let weak_node = Arc::downgrade(&node);
    let resolved = sanim_core::voxel::resolve_layout(&node).unwrap();
    let weak_output = Arc::downgrade(&resolved);
    drop(resolved);
    drop(node);
    assert!(
        weak_node.upgrade().is_none(),
        "layout memo retained its source tree"
    );
    for _ in 0..32 {
        let next = Arc::new(
            Node::part(
                Arc::new(sanim_voxel::Volume::new([4, 4, 4]).unwrap()),
                Placement::default(),
            )
            .unwrap(),
        );
        sanim_core::voxel::resolve_layout(&next).unwrap();
    }
    assert!(
        weak_output.upgrade().is_none(),
        "layout memo retained an evicted revision"
    );
    let (bytes, entries) = sanim_core::voxel::layout_cache_usage();
    assert!(bytes <= 8 * 1024 * 1024);
    assert!(entries <= 16);
}

#[test]
#[ignore]
fn held_voxel_layout_benchmark() {
    use sanim_voxel::layout::{Dimension, Node, Placement, Space};
    let volume = Arc::new(
        sanim_voxel::Volume::new([64, 16, 64])
            .unwrap()
            .fill_color([0, 0, 0], [64, 16, 64], 0x33aa55ff)
            .unwrap(),
    );
    let part = Arc::new(Node::part(volume, Placement::default()).unwrap());
    let node = Arc::new(
        Node::space(Space {
            size: [
                Dimension::Cells(64),
                Dimension::Cells(16),
                Dimension::Cells(64),
            ],
            children: std::iter::once(part).collect(),
            ..Default::default()
        })
        .unwrap(),
    );
    let resolved = sanim_core::voxel::resolve_layout(&node).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        assert!(Arc::ptr_eq(
            &resolved,
            &sanim_core::voxel::resolve_layout(&node).unwrap()
        ));
    }
    let warm = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        sanim_voxel::layout::resolve(&node).unwrap();
    }
    let cold = start.elapsed();
    println!(
        "held 64x16x64 layout: cached {}ns/sample versus direct {}ns/sample; shared-volume {} bytes; bounded cache {:?}",
        warm.as_nanos() / 1000,
        cold.as_nanos() / 100,
        resolved.source.resident_bytes(),
        sanim_core::voxel::layout_cache_usage()
    );
}

#[test]
#[ignore]
fn quantized_subcell_translation_benchmark() {
    use sanim_voxel::layout::{Dimension, Node, Placement, Space};
    let source = Arc::new(
        sanim_voxel::Volume::new([64, 16, 64])
            .unwrap()
            .fill_color([0; 3], [64, 16, 64], 0x33aa55ff)
            .unwrap(),
    );
    let layout = |x| {
        let part = Arc::new(
            Node::part(
                source.clone(),
                Placement {
                    absolute: true,
                    offset: [x, Fraction::from(0), Fraction::from(0)],
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        Arc::new(
            Node::space(Space {
                size: [
                    Dimension::Cells(68),
                    Dimension::Cells(18),
                    Dimension::Cells(66),
                ],
                children: std::iter::once(part).collect(),
                ..Default::default()
            })
            .unwrap(),
        )
    };
    let held_node = layout(Fraction::new(11u64, 10u64));
    let held = sanim_core::voxel::resolve_layout(&held_node).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..60 {
        let next = sanim_core::voxel::resolve_layout(&held_node).unwrap();
        assert!(held.source.same_snapshot(&next.source));
    }
    let cached = start.elapsed();
    let mut frames = Vec::with_capacity(60);
    let start = std::time::Instant::now();
    for frame in 0..60u64 {
        let node = layout(Fraction::new(11u64, 10u64) + Fraction::new(8 * frame, 590u64));
        frames.push(sanim_core::voxel::resolve_layout(&node).unwrap());
    }
    let animated = start.elapsed();
    for frame in &frames {
        assert!(
            held.source.same_snapshot(&frame.source),
            "subcell animation must reuse its snapped voxel assembly"
        );
        assert_eq!(&held.source, &frame.source);
        assert_eq!(
            frame
                .placements
                .iter()
                .find(|placed| placed.leaf)
                .unwrap()
                .origin,
            [1, 0, 0]
        );
    }
    let unique = frames
        .iter()
        .map(|frame| frame.source.revision())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!(
        "64x16x64 part, x=1.1..1.9 across 60 identical-snapped frames: held {}ns/frame, distinct fraction recipes {}us/frame, unique resolved volumes {unique}, same root Arc {}, retained output {} bytes",
        cached.as_nanos() / 60,
        animated.as_micros() / 60,
        frames
            .iter()
            .all(|frame| held.source.same_snapshot(&frame.source)),
        held.source.resident_bytes()
    );
}

#[test]
#[ignore]
fn voxel_camera_hot_reload_benchmark() {
    for edge in [128, 512] {
        let mut sources = MemorySources::default();
        let source = |yaw| {
            format!(
                "let grid=VoxelGrid(size=[{edge},64,{edge}]).fill([0,0,0],[{edge},8,{edge}],Color(\"green3\"))\nlet scene=VoxelScene(volume=grid,yaw={yaw}deg)\nscene.fade_in()\nwait(1s)"
            )
        };
        let mut engine = Engine::default();
        sources.insert("main.sanim", source(0));
        let prepared = engine
            .prepare("main.sanim", &sources, &Default::default())
            .unwrap();
        let Value::VoxelGrid(original) = &prepared.bindings["grid"] else {
            panic!("grid")
        };
        let mut timings = Vec::new();
        for yaw in 1..101 {
            sources.insert("main.sanim", source(yaw));
            let start = std::time::Instant::now();
            let updated = engine
                .prepare("main.sanim", &sources, &Default::default())
                .unwrap();
            timings.push(start.elapsed().as_micros());
            let Value::VoxelGrid(grid) = &updated.bindings["grid"] else {
                panic!("grid")
            };
            assert!(Arc::ptr_eq(original, grid));
        }
        timings.sort_unstable();
        let mut cold = Vec::new();
        for yaw in 0..20 {
            sources.insert("main.sanim", source(yaw));
            let start = std::time::Instant::now();
            Engine::default()
                .prepare("main.sanim", &sources, &Default::default())
                .unwrap();
            cold.push(start.elapsed().as_micros());
        }
        cold.sort_unstable();
        println!(
            "{edge}x64x{edge}, {} bricks, 100 camera hot edits: median {}us p95 {}us; cold rebuild median {}us; resident volume {} bytes; shared Arc refs {}",
            original.chunk_count(),
            timings[50],
            timings[95],
            cold[10],
            original.resident_bytes(),
            Arc::strong_count(original)
        );
    }
}
