//! Compare one sampled curve path with one Element per segment, then measure
//! deterministic statistics over the same 10,000 input values.
use sanim_core::{
    Animation, Color, Easing, Element, Fraction, Geometry, Motion, Property, Unit, Value, Vec2,
    math::chart::plot::{self, DensityOptions, Normalization},
};
use sanim_render::{Options, Renderer};
use std::{sync::Arc, time::Instant};

fn element(id: String, geometry: Geometry, position: Vec2) -> Element {
    Element {
        id,
        geometry: Arc::new(geometry),
        position,
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: Color::parse("blue3").unwrap().into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Default::default(),
    }
}
fn animation(batched: bool) -> Result<Animation, String> {
    const POINTS: usize = 1000;
    let points = (0..POINTS)
        .map(|index| Vec2 {
            values: [
                -300. + 600. * index as f64 / (POINTS - 1) as f64,
                (index as f64 / 70.).sin() * 100.,
            ],
            unit: Unit::Length,
        })
        .collect::<Vec<_>>();
    let children = if batched {
        vec![element(
            "curve/path".into(),
            Geometry::Polyline {
                points: points.into(),
                width: 2.into(),
            },
            Vec2::zero(Unit::Length),
        )]
    } else {
        points
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                element(
                    format!("curve/{index}"),
                    Geometry::Line {
                        end: Vec2 {
                            values: [
                                pair[1].values[0] - pair[0].values[0],
                                pair[1].values[1] - pair[0].values[1],
                            ],
                            unit: Unit::Length,
                        },
                        width: 2.into(),
                    },
                    Vec2 {
                        values: [
                            (pair[0].values[0] + pair[1].values[0]) / 2.,
                            (pair[0].values[1] + pair[1].values[1]) / 2.,
                        ],
                        unit: Unit::Length,
                    },
                )
            })
            .collect()
    };
    let mut group = element(
        "curve".into(),
        Geometry::Group {
            children: children.into(),
        },
        Vec2::zero(Unit::Length),
    );
    group.layout.x_align = sanim_core::Align::Start;
    group.layout.y_align = sanim_core::Align::Start;
    let mut animation = Animation::for_viewport(&Default::default());
    animation.add(group.clone(), 0.into())?;
    animation.schedule(
        &[Motion {
            element: group,
            property: Property::Position,
            from: None,
            to: Value::Vec2(Vec2 {
                values: [10., 0.],
                unit: Unit::Length,
            }),
            reveal: None,
            horizontal: None,
            easing: None,
        }],
        0.into(),
        1.into(),
        Easing::Linear,
    )?;
    Ok(animation)
}
fn main() -> Result<(), String> {
    for batched in [false, true] {
        let animation = animation(batched)?;
        let mut renderer = Renderer::default();
        let start = Instant::now();
        let scene = renderer.prepare(animation)?;
        let options = Options::for_animation(scene.animation());
        for frame in 0u64..30 {
            renderer.record(&scene, Fraction::new(frame, 30u64), &options)?;
        }
        let cold = start.elapsed().as_micros();
        let warm = Instant::now();
        for frame in 0u64..30 {
            renderer.record(&scene, Fraction::new(frame, 30u64), &options)?;
        }
        println!(
            "batched={batched} samples=1000 frames=30 cold_prepare_record_us={cold} warm_record_us={} geometries_prepared={} state_count={}",
            warm.elapsed().as_micros(),
            renderer.geometries_prepared,
            renderer.states_computed
        );
    }
    let values = (0..10_000)
        .map(|index| ((index * 7919) % 10_000) as f64 / 2500. - 2.)
        .collect::<Vec<_>>();
    let edges = plot::bin_edges([-2., 2.], 50)?;
    let start = Instant::now();
    for _ in 0..20 {
        std::hint::black_box(plot::histogram(&values, &edges, Normalization::Density)?);
    }
    println!(
        "histogram_samples=10000 bins=50 mean_us={}",
        start.elapsed().as_micros() / 20
    );
    let start = Instant::now();
    let density = plot::density(
        &values,
        DensityOptions {
            domain: [-3., 3.],
            points: 64,
            bandwidth: Some(0.3),
            max_work: 1_000_000,
        },
    )?;
    println!(
        "kde_samples=10000 points={} kernel_evaluations=640000 elapsed_us={}",
        density.len(),
        start.elapsed().as_micros()
    );
    Ok(())
}
