use sanim_core::{
    Animation, Color, Direction, Easing, Element, Fraction, Geometry, Layout, List, Type, Unit,
    Value, Vec2,
};
use sanim_render::{Options, Renderer};
use std::sync::Arc;

fn tile(id: &str) -> Element {
    Element {
        id: id.into(),
        geometry: Arc::new(Geometry::Rectangle {
            width: 10.into(),
            height: 10.into(),
            children: Default::default(),
            radius: Default::default(),
        }),
        position: Vec2::zero(Unit::Length),
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: Color::parse("blue3").unwrap().into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Layout::default(),
    }
}
fn items(ids: &[&str]) -> List {
    List::typed(
        Type::Element,
        ids.iter()
            .map(|id| Value::Element(Arc::new(tile(id))))
            .collect(),
    )
}
fn animation() -> Animation {
    let mut animation = Animation::for_viewport(&sanim_core::Viewport {
        width: 100,
        height: 40,
        dpi: 72,
        ..Default::default()
    });
    let mut view = tile("view");
    view.geometry = Arc::new(Geometry::Group {
        children: Default::default(),
    });
    view.layout = Layout {
        direction: Some(Direction::Row),
        width: Some(sanim_core::Number {
            value: 80.into(),
            unit: sanim_core::Unit::Length,
        }),
        height: Some(sanim_core::Number {
            value: 20.into(),
            unit: sanim_core::Unit::Length,
        }),
        ..Default::default()
    };
    animation.bind_list(1, &view, items(&[])).unwrap();
    animation.add(view, 0.into()).unwrap();
    animation
        .write_list(
            1,
            1.into(),
            2.into(),
            items(&["first", "second"]),
            Easing::Linear,
        )
        .unwrap();
    animation
}
#[test]
fn future_items_prepare_paths_on_demand_then_removed_items_disappear() {
    let mut animation = animation();
    animation
        .write_list(1, 3.into(), 4.into(), items(&["second"]), Easing::Linear)
        .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    assert_eq!(renderer.geometries_prepared, 0);
    let initial = renderer.render(&scene, 0.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 0);
    let filled = renderer.render(&scene, 2.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    assert_ne!(initial.rgba, filled.rgba);
    let removed = renderer.render(&scene, 4.into(), &options).unwrap();
    assert_ne!(filled.rgba, removed.rgba);
    let at_two = renderer.render(&scene, 2.into(), &options).unwrap();
    assert!(Arc::ptr_eq(&filled, &at_two));
    let hold = renderer
        .render(&scene, Fraction::new(5u64, 2u64), &options)
        .unwrap();
    assert!(Arc::ptr_eq(&filled, &hold));
}
#[test]
fn late_list_edits_preserve_cached_frames_before_the_change() {
    let old = animation();
    let mut changed = old.clone();
    changed
        .write_list(1, 4.into(), 5.into(), items(&["second"]), Easing::Linear)
        .unwrap();
    let mut renderer = Renderer::default();
    let first = renderer.prepare(old).unwrap();
    let mut options = Options::for_animation(first.animation());
    options.encode_png = false;
    let frame = renderer.render(&first, 2.into(), &options).unwrap();
    let second = renderer.prepare(changed).unwrap();
    let reused = renderer.render(&second, 2.into(), &options).unwrap();
    assert!(Arc::ptr_eq(&frame, &reused));
}

#[test]
#[ignore = "measured by make list-view-bench"]
fn retained_list_renderer_prepare_benchmark() {
    const INITIAL: usize = 1_000;
    const EDITS: usize = 100;
    let mut animation = animation();
    let names: Vec<_> = (0..INITIAL)
        .map(|index| format!("initial_{index}"))
        .collect();
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    animation
        .write_list(1, 2.into(), 2.into(), items(&refs), Easing::Linear)
        .unwrap();
    for index in 0..EDITS {
        let start = Fraction::from(index + 2);
        let source = animation.sample_list(1, start).unwrap().unwrap();
        let edit = source
            .edit(sanim_core::ListEdit::Append(Value::Element(Arc::new(
                tile(&format!("extra_{index}")),
            ))))
            .unwrap();
        animation
            .write_list_edit(1, start, Fraction::from(index + 3), &edit, Easing::Linear)
            .unwrap();
    }
    let start = std::time::Instant::now();
    let full = serde_json::to_vec(&animation).unwrap();
    let full_us = start.elapsed().as_micros();
    let start = std::time::Instant::now();
    let compact = animation.visual_key().unwrap();
    let compact_us = start.elapsed().as_micros();
    assert!(compact.len() < full.len() / 10);
    let mut renderer = Renderer::default();
    let start = std::time::Instant::now();
    let first = renderer.prepare(animation.clone()).unwrap();
    let prepare_us = start.elapsed().as_micros();
    let options = Options::for_animation(first.animation());
    let prefix = renderer.frame_state(&first, 2.into(), &options).unwrap();
    let source = animation
        .sample_list(1, Fraction::from(EDITS + 2))
        .unwrap()
        .unwrap();
    let edit = source
        .edit(sanim_core::ListEdit::Append(Value::Element(Arc::new(
            tile("late_extra"),
        ))))
        .unwrap();
    animation
        .write_list_edit(
            1,
            Fraction::from(EDITS + 2),
            Fraction::from(EDITS + 3),
            &edit,
            Easing::Linear,
        )
        .unwrap();
    let start = std::time::Instant::now();
    let patched = renderer.prepare(animation).unwrap();
    let patch_us = start.elapsed().as_micros();
    assert!(Arc::ptr_eq(
        &prefix,
        &renderer.frame_state(&patched, 2.into(), &options).unwrap()
    ));
    eprintln!(
        "renderer_list_initial={INITIAL} edits={EDITS} full_export_bytes={} compact_key_bytes={} full_export_us={full_us} compact_key_us={compact_us} prepare_us={prepare_us} hot_patch_us={patch_us} prefix_reused=true",
        full.len(),
        compact.len()
    );
}
