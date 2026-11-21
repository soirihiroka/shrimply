use imbl::Vector;
use sanim_voxel::{Volume, layout::*};
use std::sync::Arc;

fn part(size: [u32; 3]) -> Arc<Node> {
    let volume = Volume::new(size)
        .unwrap()
        .fill_color([0; 3], size, 0xff7800ff)
        .unwrap();
    Arc::new(Node::part(Arc::new(volume), Placement::default()).unwrap())
}
fn space(space: Space) -> Arc<Node> {
    Arc::new(Node::space(space).unwrap())
}

#[test]
fn intrinsic_layout_uses_all_three_axes_and_padding() {
    let root = space(Space {
        padding: [1, 2, 3],
        gap: 1,
        align: [Align::Center; 3],
        children: Vector::from(vec![part([2, 3, 4]), part([4, 1, 2])]),
        ..Space::default()
    });
    let output = resolve(&root).unwrap();
    assert_eq!(output.source.dimensions(), [9, 7, 10]);
    assert_eq!(output.placements[1].origin, [1, 2, 3]);
    assert_eq!(output.placements[2].origin, [4, 3, 4]);
    assert_eq!(output.source.occupied_voxels().unwrap(), 32);
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let output = resolve(&space(Space {
            flow: axis,
            gap: 1,
            children: Vector::from(vec![part([2; 3]), part([2; 3])]),
            ..Space::default()
        }))
        .unwrap();
        let expected = match axis {
            Axis::X => [5, 2, 2],
            Axis::Y => [2, 5, 2],
            Axis::Z => [2, 2, 5],
        };
        assert_eq!(output.source.dimensions(), expected);
    }
}

#[test]
fn z_flow_distributes_space_and_aligns_cross_axes() {
    let output = resolve(&space(Space {
        size: [
            Dimension::Cells(20),
            Dimension::Cells(10),
            Dimension::Cells(30),
        ],
        flow: Axis::Z,
        padding: [1; 3],
        gap: 2,
        justify: Justify::Between,
        align: [Align::Center, Align::End, Align::Start],
        children: Vector::from(vec![part([2, 3, 4]), part([2, 3, 4])]),
        ..Space::default()
    }))
    .unwrap();
    assert_eq!(output.placements[1].origin, [9, 6, 1]);
    assert_eq!(output.placements[2].origin, [9, 6, 25]);
}

#[test]
fn nested_percentages_resolve_against_the_inner_containing_volume() {
    let child = space(Space {
        size: [
            Dimension::Percent(50.into()),
            Dimension::Cells(6),
            Dimension::Cells(4),
        ],
        justify: Justify::End,
        children: Vector::from(vec![part([2, 3, 4])]),
        ..Space::default()
    });
    let output = resolve(&space(Space {
        size: [
            Dimension::Cells(20),
            Dimension::Cells(10),
            Dimension::Cells(8),
        ],
        padding: [2, 1, 1],
        children: Vector::from(vec![child]),
        ..Space::default()
    }))
    .unwrap();
    assert_eq!(output.placements[1].size, [8, 6, 4]);
    assert_eq!(output.placements[2].origin, [8, 1, 1]);
}

#[test]
fn anchors_are_independent_of_cameras_and_round_only_the_final_origin() {
    let volume = Arc::new(
        Volume::new([3, 3, 3])
            .unwrap()
            .fill_color([0; 3], [3; 3], 0xffffffff)
            .unwrap(),
    );
    let anchored = Arc::new(
        Node::part(
            volume,
            Placement {
                absolute: true,
                at: [
                    Coordinate::Percent(50.into()),
                    Coordinate::Cells(0.into()),
                    Coordinate::Percent(50.into()),
                ],
                anchor: [
                    Coordinate::Percent(50.into()),
                    Coordinate::Cells(0.into()),
                    Coordinate::Percent(50.into()),
                ],
                ..Placement::default()
            },
        )
        .unwrap(),
    );
    let output = resolve(&space(Space {
        size: [Dimension::Cells(20); 3],
        children: Vector::from(vec![anchored]),
        ..Space::default()
    }))
    .unwrap();
    assert_eq!(output.placements[1].origin, [8, 0, 8]);
    assert_eq!(output.source.occupied_voxels().unwrap(), 27);
}

#[test]
fn sizing_cycles_and_overflow_fail_with_the_child_and_axis() {
    let percent = space(Space {
        size: [Dimension::Percent(50.into()); 3],
        ..Space::default()
    });
    let error = resolve(&space(Space {
        children: Vector::from(vec![percent]),
        ..Space::default()
    }))
    .unwrap_err();
    assert!(error.contains("child[0]"));
    assert!(error.contains("parent-relative"));
    let root = space(Space {
        size: [Dimension::Cells(2); 3],
        children: Vector::from(vec![part([3; 3])]),
        ..Space::default()
    });
    assert!(resolve(&root).unwrap_err().contains("overflows"));
}

#[test]
fn distribution_conserves_integer_space_for_every_justification() {
    for justify in [
        Justify::Start,
        Justify::Center,
        Justify::End,
        Justify::Between,
        Justify::Around,
        Justify::Evenly,
    ] {
        let output = resolve(&space(Space {
            size: [
                Dimension::Cells(17),
                Dimension::Cells(2),
                Dimension::Cells(2),
            ],
            justify,
            children: Vector::from(vec![part([2; 3]), part([2; 3]), part([2; 3])]),
            ..Space::default()
        }))
        .unwrap();
        let origins: Vec<_> = output
            .placements
            .iter()
            .filter(|part| part.leaf)
            .map(|part| part.origin[0])
            .collect();
        assert!(origins.windows(2).all(|pair| pair[0] + 2 <= pair[1]));
        assert!(origins[2] + 2 <= 17);
    }
}

#[test]
fn rebuilt_recipes_compare_without_serializing_voxel_payloads() {
    let volume = Arc::new(
        Volume::new([8; 3])
            .unwrap()
            .fill_color([0; 3], [8; 3], 0xffffffff)
            .unwrap(),
    );
    let build = || {
        space(Space {
            padding: [1; 3],
            children: Vector::from(vec![Arc::new(
                Node::part(volume.clone(), Placement::default()).unwrap(),
            )]),
            ..Space::default()
        })
    };
    let first = build();
    let second = build();
    assert_ne!(first.revision(), second.revision());
    assert!(first.same_recipe(&second));
    assert_eq!(first.fingerprint(), second.fingerprint());
    let changed = space(Space {
        padding: [2; 3],
        children: Vector::from(vec![Arc::new(
            Node::part(volume, Placement::default()).unwrap(),
        )]),
        ..Space::default()
    });
    assert!(!first.same_recipe(&changed));
}

#[test]
fn repeated_shared_parts_are_work_bounded_before_assembly() {
    let grid = Arc::new(
        Volume::new([128; 3])
            .unwrap()
            .fill_color([0; 3], [128; 3], 0xffffffff)
            .unwrap()
            .fill_color([0; 3], [1; 3], 0xff7800ff)
            .unwrap(),
    );
    let item = Arc::new(
        Node::part(
            grid.clone(),
            Placement {
                absolute: true,
                ..Placement::default()
            },
        )
        .unwrap(),
    );
    let root = space(Space {
        size: [Dimension::Cells(128); 3],
        children: (0..2000).map(|_| item.clone()).collect(),
        ..Space::default()
    });
    let error = resolve(&root).unwrap_err();
    assert!(error.contains("metadata budget"), "{error}");
    assert_eq!(grid.occupied_voxels(), 128 * 128 * 128);
}

#[test]
fn fractional_coordinates_and_offsets_round_only_after_combining() {
    use fraction::Fraction;
    let volume = Arc::new(
        Volume::new([2; 3])
            .unwrap()
            .fill_color([0; 3], [2; 3], 0xffffffff)
            .unwrap(),
    );
    let item = Arc::new(
        Node::part(
            volume,
            Placement {
                absolute: true,
                at: [
                    Coordinate::Cells(Fraction::new(162u64, 10u64)),
                    Coordinate::default(),
                    Coordinate::default(),
                ],
                anchor: [
                    Coordinate::Cells(Fraction::new(87u64, 10u64)),
                    Coordinate::default(),
                    Coordinate::default(),
                ],
                offset: [Fraction::new(1u64, 4u64), 0.into(), 0.into()],
                ..Placement::default()
            },
        )
        .unwrap(),
    );
    let output = resolve(&space(Space {
        size: [Dimension::Cells(20); 3],
        children: Vector::from(vec![item]),
        ..Space::default()
    }))
    .unwrap();
    assert_eq!(output.placements[1].origin, [7, 0, 0]);
}

#[test]
fn alignment_and_distribution_do_not_snap_before_fractional_offsets() {
    use fraction::Fraction;
    let grid = Arc::new(
        Volume::new([2; 3])
            .unwrap()
            .fill_color([0; 3], [2; 3], 0xffffffff)
            .unwrap(),
    );
    let item = Arc::new(
        Node::part(
            grid,
            Placement {
                offset: [Fraction::new(3u64, 5u64), 0.into(), 0.into()],
                ..Placement::default()
            },
        )
        .unwrap(),
    );
    let aligned = resolve(&space(Space {
        size: [
            Dimension::Cells(9),
            Dimension::Cells(2),
            Dimension::Cells(2),
        ],
        flow: Axis::Y,
        align: [Align::Center, Align::Start, Align::Start],
        children: Vector::from(vec![item.clone()]),
        ..Space::default()
    }))
    .unwrap();
    assert_eq!(aligned.placements[1].origin, [4, 0, 0]);
    for justify in [Justify::Center, Justify::Around, Justify::Evenly] {
        let distributed = resolve(&space(Space {
            size: [
                Dimension::Cells(9),
                Dimension::Cells(2),
                Dimension::Cells(2),
            ],
            justify,
            children: Vector::from(vec![item.clone()]),
            ..Space::default()
        }))
        .unwrap();
        assert_eq!(distributed.placements[1].origin, [4, 0, 0]);
    }
}

#[test]
fn assembly_keys_share_snapped_geometry_and_preserve_inspector_metadata() {
    use fraction::Fraction;
    let grid = Arc::new(
        Volume::new([2; 3])
            .unwrap()
            .fill_color([0; 3], [2; 3], 0xffffffff)
            .unwrap(),
    );
    let moving = |x| {
        space(Space {
            size: [Dimension::Cells(20); 3],
            children: Vector::from(vec![Arc::new(
                Node::part(
                    grid.clone(),
                    Placement {
                        absolute: true,
                        at: [
                            Coordinate::Cells(x),
                            Coordinate::default(),
                            Coordinate::default(),
                        ],
                        ..Placement::default()
                    },
                )
                .unwrap(),
            )]),
            ..Space::default()
        })
    };
    let first = moving(Fraction::new(11u64, 10u64));
    let second = moving(Fraction::new(19u64, 10u64));
    assert!(!first.same_recipe(&second));
    assert_eq!(
        prepare(&first).unwrap().fingerprint(),
        prepare(&second).unwrap().fingerprint()
    );
    let nested = |size| {
        space(Space {
            size: [Dimension::Cells(20); 3],
            children: Vector::from(vec![space(Space {
                size: [Dimension::Cells(size); 3],
                children: Vector::from(vec![Arc::new(
                    Node::part(grid.clone(), Placement::default()).unwrap(),
                )]),
                ..Space::default()
            })]),
            ..Space::default()
        })
    };
    let small = prepare(&nested(4)).unwrap();
    let large = prepare(&nested(5)).unwrap();
    assert_ne!(small.fingerprint(), large.fingerprint());
    assert_eq!(
        small.assemble().unwrap().source,
        large.assemble().unwrap().source
    );
}
