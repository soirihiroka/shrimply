use sanim_voxel::{Material, MaterialGeometry, Texture, Volume, math::raycast};
use std::sync::Arc;

fn plant() -> Volume {
    let pixels: Vec<_> = (0..16)
        .flat_map(|pixel| {
            if pixel % 4 == 1 {
                [40, 180, 60, 255]
            } else {
                [0; 4]
            }
        })
        .collect();
    let texture = Arc::new(Texture::new(4, 4, pixels.into()).unwrap());
    let material = Material::textured(0xffffffff, std::array::from_fn(|_| Some(texture.clone())))
        .unwrap()
        .with_geometry(MaterialGeometry::Cross)
        .unwrap();
    let (volume, id) = Volume::new([1; 3])
        .unwrap()
        .with_material(Arc::new(material))
        .unwrap();
    volume.fill_box([0; 3], [1; 3], id).unwrap()
}

#[test]
fn crossed_cards_are_double_sided_and_cutout_holes_do_not_fill_the_cell() {
    let volume = plant();
    let front = raycast(&volume, [-1.0, 0.5, 0.3], [1.0, 0.0, 0.0])
        .unwrap()
        .unwrap();
    assert!((front.distance - 1.3).abs() < 1e-12);
    assert_eq!(front.position, [0; 3]);
    assert!((front.normal.iter().map(|x| x * x).sum::<f64>() - 1.0).abs() < 1e-12);
    assert!(front.normal[0] < 0.0 && front.normal[2] > 0.0);
    let back = raycast(&volume, [2.0, 0.5, 0.3], [-1.0, 0.0, 0.0])
        .unwrap()
        .unwrap();
    assert!((back.distance - 1.7).abs() < 1e-12);
    assert!(back.normal[0] > 0.0 && back.normal[2] < 0.0);
    assert!(
        raycast(&volume, [-1.0, 0.5, 0.5], [1.0, 0.0, 0.0])
            .unwrap()
            .is_none()
    );
    assert!(
        raycast(&volume, [0.25, 2.0, 0.25], [0.0, -1.0, 0.0])
            .unwrap()
            .is_none()
    );
}

#[test]
fn picking_inside_a_foliage_cell_finds_the_card_instead_of_a_solid_box() {
    let hit = raycast(&plant(), [0.3, 0.5, 0.1], [0.0, 0.0, 1.0])
        .unwrap()
        .unwrap();
    assert!((hit.distance - 0.2).abs() < 1e-12);
    assert!(hit.normal[2] < 0.0);
}
