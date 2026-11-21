use super::*;
use crate::LayeredState;
const PREFIX: &str = "/content/layers/";
fn normalized(item: &VideoItem, layers: &[Layer]) -> Vec<LayerVisibility> {
    let VideoItemContent::LayeredImage(image) = &item.content else {
        unreachable!("layered image controls")
    };
    let mut values = image.layers.clone();
    for layer in layers {
        if let Some(stored) = values
            .iter_mut()
            .find(|value| value.path == layer.entry.path)
        {
            if stored.visibility.is_none() {
                stored.visibility = layer.default.visibility.clone();
            }
        } else {
            values.push(layer.default.clone());
        }
    }
    values
}
pub(super) fn controls(
    item: &VideoItem,
    layers: &[Layer],
    runtime: InspectorRuntime,
) -> Vec<InspectorControl> {
    let values = normalized(item, layers);
    layers
        .iter()
        .map(|layer| {
            let (index, value) = values
                .iter()
                .enumerate()
                .find(|(_, value)| value.path == layer.entry.path)
                .expect("normalized image layer");
            let visibility = value.visibility.as_ref().expect("layer visibility default");
            let path = format!("{PREFIX}{index}/visibility");
            let label = format!(
                "{}{}{}",
                "  ".repeat(layer.entry.depth),
                if layer.entry.group { "▾ " } else { "" },
                layer
                    .entry
                    .name
                    .chars()
                    .filter(|character| *character != '\0')
                    .collect::<String>()
            );
            InspectorControl::new(ControlKind::LayeredBoolean, &path, label)
                .value(
                    visibility
                        .value_at(runtime.local_time.unwrap_or(shrimply_math_core::Time::ZERO))
                        .get()
                        .to_string(),
                )
                .layered(&path, LayeredState::from(visibility))
                .target(value.id)
                .timeline(
                    visibility.id,
                    crate::selector::step_graph(visibility, runtime),
                )
                .live_commit("visual-bool")
                .timeline_commits("visual-bool-keyframes", "visual-bool-expression")
        })
        .collect()
}
pub(super) fn ensure(
    controller: &InspectorController,
    target: &InspectorTarget,
    control: &InspectorControl,
) -> Result<(), String> {
    let InspectorTarget::Item(address) = target else {
        return Err("Layer visibility requires a video item".into());
    };
    let project = controller.project.borrow();
    let item = project
        .video_item(address)
        .ok_or("Layered-image item is no longer available")?;
    if !matches!(item.content, VideoItemContent::LayeredImage(_)) {
        return Err("Item is no longer a layered image".into());
    }
    let State::Ready(snapshot, data) = controller.visual_sources.borrow_mut().request(item) else {
        return Err("Layered-image source metadata is not ready".into());
    };
    snapshot.verify_current()?;
    let Data::Layered(layers) = &*data else {
        return Err("Source is no longer a layered image".into());
    };
    let values = normalized(item, layers);
    let (index, field) = control
        .path
        .strip_prefix(PREFIX)
        .and_then(|path| path.split_once('/'))
        .ok_or("Invalid layer visibility control")?;
    let layer = values
        .get(
            index
                .parse::<usize>()
                .map_err(|_| "Invalid layer visibility index")?,
        )
        .ok_or("Image layer is no longer available")?;
    if field != "visibility"
        || !layers.iter().any(|source| source.entry.path == layer.path)
        || Some(layer.id) != control.target_id
        || layer.visibility.as_ref().map(|value| value.id) != control.timeline_id
    {
        return Err("Image layers changed; refresh the inspector".into());
    }
    drop(project);
    let mut project = controller.project.borrow_mut();
    let VideoItemContent::LayeredImage(image) = &mut project
        .video_item_mut(address)
        .expect("validated layered image")
        .content
    else {
        unreachable!("validated source type")
    };
    image.layers = values;
    Ok(())
}
