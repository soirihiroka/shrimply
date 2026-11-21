use imbl::Vector;
use sanim_voxel::{CameraRig, Volume, layout::*, verify::*};
use std::{sync::Arc, time::Instant};

fn part(volume: Volume, at: [u32; 3]) -> Arc<Node> {
    Arc::new(
        Node::part(
            Arc::new(volume),
            Placement {
                absolute: true,
                at: at.map(|value| Coordinate::Cells(value.into())),
                ..Default::default()
            },
        )
        .unwrap(),
    )
}
fn world(children: Vec<Arc<Node>>, size: u32) -> Arc<Node> {
    Arc::new(
        Node::space(Space {
            size: [Dimension::Cells(size); 3],
            children: Vector::from(children),
            ..Default::default()
        })
        .unwrap(),
    )
}
fn solid(size: u32) -> Volume {
    Volume::new([size; 3])
        .unwrap()
        .fill_color([0; 3], [size; 3], 0xff7800ff)
        .unwrap()
}

#[test]
fn occupied_cells_not_overlapping_bounds_decide_collisions() {
    let shell = solid(8).fill_box([1; 3], [7; 3], 0).unwrap();
    let first = part(shell, [0; 3]);
    let second = part(solid(2), [3; 3]);
    let root = world(vec![first.clone(), second.clone()], 16);
    let resolved = resolve(&root).unwrap();
    assert_disjoint(
        &root,
        &resolved.placements,
        &first,
        &second,
        MAX_VERIFY_WORK,
    )
    .unwrap();
    let colliding = part(solid(2), [0; 3]);
    let root = world(vec![first.clone(), colliding.clone()], 16);
    let error = assert_disjoint(
        &root,
        &resolve(&root).unwrap().placements,
        &first,
        &colliding,
        MAX_VERIFY_WORK,
    )
    .unwrap_err();
    assert!(
        error.contains("V001")
            && error.contains("child[0]")
            && error.contains("child[1]")
            && error.contains("[0, 0, 0]"),
        "{error}"
    );
}

#[test]
fn recipe_matching_is_pure_and_checks_every_identical_occurrence() {
    let first = part(solid(2), [0; 3]);
    let independently_built = Arc::new(
        Node::part(
            first.volume().unwrap().clone(),
            Placement {
                absolute: true,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    assert_ne!(first.revision(), independently_built.revision());
    let root = world(vec![first.clone(), first.clone()], 16);
    let error = assert_disjoint(
        &root,
        &resolve(&root).unwrap().placements,
        &first,
        &independently_built,
        MAX_VERIFY_WORK,
    )
    .unwrap_err();
    assert!(error.contains("same world voxel"), "{error}");
    let absent = part(solid(3), [0; 3]);
    assert!(
        assert_disjoint(
            &root,
            &resolve(&root).unwrap().placements,
            &first,
            &absent,
            MAX_VERIFY_WORK
        )
        .unwrap_err()
        .contains("not part")
    );
}

#[test]
fn containment_checks_selected_container_and_budgets_are_hard_limits() {
    let inner = part(solid(2), [0; 3]);
    let container = world(vec![inner.clone()], 4);
    let outside = part(solid(2), [8; 3]);
    let root = world(vec![container.clone(), outside.clone()], 16);
    let resolved = resolve(&root).unwrap();
    assert_within(
        &root,
        &resolved.placements,
        &inner,
        &container,
        MAX_VERIFY_WORK,
    )
    .unwrap();
    let error = assert_within(
        &root,
        &resolved.placements,
        &outside,
        &container,
        MAX_VERIFY_WORK,
    )
    .unwrap_err();
    assert!(
        error.contains("V002") && error.contains("child[1]") && error.contains("bounds"),
        "{error}"
    );
    assert!(
        assert_disjoint(&root, &resolved.placements, &inner, &outside, 1)
            .unwrap_err()
            .contains("budget")
    );
}

#[test]
fn sparse_verification_work_does_not_depend_on_empty_world_volume() {
    let first = part(
        Volume::new([512; 3])
            .unwrap()
            .fill_color([0; 3], [1; 3], 0xffffffff)
            .unwrap(),
        [0; 3],
    );
    let second = part(
        Volume::new([512; 3])
            .unwrap()
            .fill_color([500; 3], [501; 3], 0xffffffff)
            .unwrap(),
        [0; 3],
    );
    let root = world(vec![first.clone(), second.clone()], 512);
    let resolved = resolve(&root).unwrap();
    let statistics = assert_disjoint(
        &root,
        &resolved.placements,
        &first,
        &second,
        MAX_VERIFY_WORK,
    )
    .unwrap();
    assert!(statistics.tested_cells <= 512, "{statistics:?}");
    assert!(statistics.work < 600, "{statistics:?}");
}

#[test]
fn framing_checks_perspective_and_orthographic_snapshots() {
    let viewport = [640, 480];
    for fov in [None, Some(45.)] {
        let rig = CameraRig {
            vertical_fov_degrees: fov,
            ..Default::default()
        };
        let camera = rig.resolve([32; 3], viewport).unwrap();
        assert_framed(&camera, [32; 3], viewport, 0.).unwrap();
        let camera = CameraRig { zoom: 5., ..rig }
            .resolve([32; 3], viewport)
            .unwrap();
        assert!(
            assert_framed(&camera, [32; 3], viewport, 0.)
                .unwrap_err()
                .contains("V003")
        );
    }
    assert!(
        assert_framed(
            &CameraRig::default().resolve([32; 3], viewport).unwrap(),
            [32; 3],
            viewport,
            0.5
        )
        .is_err()
    );
}

#[test]
#[ignore = "manual sparse assertion performance comparison"]
fn sparse_assertion_benchmark() {
    let first = part(
        Volume::new([128; 3])
            .unwrap()
            .fill_color([0; 3], [1; 3], 0xffffffff)
            .unwrap(),
        [0; 3],
    );
    let second = part(
        Volume::new([128; 3])
            .unwrap()
            .fill_color([120; 3], [121; 3], 0xffffffff)
            .unwrap(),
        [0; 3],
    );
    let root = world(vec![first.clone(), second.clone()], 128);
    let resolved = resolve(&root).unwrap();
    let repetitions = 200;
    let started = Instant::now();
    for _ in 0..repetitions {
        std::hint::black_box(
            assert_disjoint(
                &root,
                &resolved.placements,
                &first,
                &second,
                MAX_VERIFY_WORK,
            )
            .unwrap(),
        );
    }
    let sparse = started.elapsed() / repetitions;
    let started = Instant::now();
    let first_volume = first.volume().unwrap();
    let second_volume = second.volume().unwrap();
    for z in 0..128 {
        for y in 0..128 {
            for x in 0..128 {
                let coordinate = std::hint::black_box([x, y, z]);
                assert!(
                    !(first_volume.get(coordinate).unwrap() != 0
                        && second_volume.get(coordinate).unwrap() != 0)
                );
            }
        }
    }
    let naive = started.elapsed();
    eprintln!(
        "sparse assertion {sparse:?}; naive 128^3 scan {naive:?}; ratio {:.1}x",
        naive.as_secs_f64() / sparse.as_secs_f64()
    );
    assert!(sparse < naive / 10);
}

#[test]
fn repeated_part_containment_does_not_expand_unused_subtrees() {
    let selected = part(Volume::new([1; 3]).unwrap(), [0; 3]);
    let root = world(vec![selected.clone(); 1000], 16);
    let statistics = assert_within(
        &root,
        &resolve(&root).unwrap().placements,
        &selected,
        &root,
        MAX_VERIFY_WORK,
    )
    .unwrap();
    assert_eq!(statistics.pairs, 1000);
    assert!(statistics.work < 6000, "{statistics:?}");
}
