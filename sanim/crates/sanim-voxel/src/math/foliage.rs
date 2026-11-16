//! Exact intersections with the two upright, diagonal cards of a foliage cell.
use super::Hit;
use crate::{EMPTY, Material, MaterialId};
use std::f64::consts::FRAC_1_SQRT_2;

pub(super) struct CellRay {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub cell: [u32; 3],
    pub entry: f64,
    pub exit: f64,
}

pub(super) fn intersect_cross(
    ray: CellRay,
    surface: &Material,
    material: MaterialId,
) -> Option<Hit> {
    let local_origin =
        std::array::from_fn::<_, 3, _>(|axis| ray.origin[axis] - f64::from(ray.cell[axis]));
    let mut closest = ray.exit;
    let mut hit = None;
    for plane in 0..2 {
        let outward = [
            FRAC_1_SQRT_2,
            0.0,
            if plane == 0 {
                -FRAC_1_SQRT_2
            } else {
                FRAC_1_SQRT_2
            },
        ];
        let denominator = outward
            .iter()
            .zip(ray.direction)
            .map(|(a, b)| a * b)
            .sum::<f64>();
        if denominator == 0.0 {
            continue;
        }
        let distance = (0..3)
            .map(|axis| (0.5 - local_origin[axis]) * outward[axis])
            .sum::<f64>()
            / denominator;
        if distance < ray.entry || distance > closest {
            continue;
        }
        let local = std::array::from_fn::<_, 3, _>(|axis| {
            local_origin[axis] + ray.direction[axis] * distance
        });
        if local
            .iter()
            .any(|coordinate| !(0.0..=1.0).contains(coordinate))
        {
            continue;
        }
        let point = [
            f64::from(ray.cell[0]) + local[0],
            f64::from(ray.cell[1]) + local[1],
            f64::from(ray.cell[2]) + if plane == 0 { local[0] } else { 1.0 - local[0] },
        ];
        let normal = outward.map(|axis| if denominator > 0.0 { -axis } else { axis });
        let uv = [
            point[0] - point[0].floor(),
            1.0 - (point[1] - point[1].floor()),
        ];
        // The diagonal normal's X component chooses the same face as the GPU.
        let color = surface.sample(usize::from(normal[0] < 0.0), uv);
        if color == EMPTY {
            continue;
        }
        closest = distance;
        hit = Some(Hit {
            position: ray.cell,
            normal,
            distance,
            material,
            color,
        });
    }
    hit
}
