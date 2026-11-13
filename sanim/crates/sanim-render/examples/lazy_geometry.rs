//! Compare early-frame startup with deferred paths versus explicit full validation.
use sanim_core::{Animation, Color, Element, Fraction, Geometry, Unit, Vec2};
use sanim_render::{Options, Renderer};
use std::{sync::Arc, time::Instant};

fn main() -> Result<(), String> {
    const FUTURE: usize = 512;
    const POINTS: usize = 128;
    let mut animation = Animation::for_viewport(&Default::default());
    animation.duration = 2.into();
    let element = |id: String, geometry| Element {
        id,
        geometry: Arc::new(geometry),
        position: Vec2::zero(Unit::Length),
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: Color::parse("blue3").expect("built-in palette").into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Default::default(),
    };
    animation.add(
        element("early".into(), Geometry::Circle { radius: 10.into() }),
        0.into(),
    )?;
    for index in 0..FUTURE {
        let points = (0..POINTS)
            .map(|point| Vec2 {
                values: [point as f64, ((point * 17) % 137) as f64 + index as f64],
                unit: Unit::Length,
            })
            .collect::<Vec<_>>();
        animation.add(
            element(
                format!("future_{index}"),
                Geometry::Polygon {
                    points: points.into(),
                },
            ),
            1.into(),
        )?;
    }
    for force_all in [true, false] {
        let mut renderer = Renderer::default();
        let started = Instant::now();
        let scene = renderer.prepare(animation.clone())?;
        if force_all {
            renderer.validate_geometry(&scene)?;
        }
        let prepare_us = started.elapsed().as_micros();
        let started = Instant::now();
        renderer.record(
            &scene,
            Fraction::from(0),
            &Options::for_animation(scene.animation()),
        )?;
        println!(
            "force_all={force_all} future={FUTURE} points={POINTS} prepare_us={prepare_us} early_frame_us={} geometries_prepared={} total_startup_us={}",
            started.elapsed().as_micros(),
            renderer.geometries_prepared,
            prepare_us + started.elapsed().as_micros()
        );
    }
    Ok(())
}
