//! Source metadata and controls shared by native SVG and layered-image inspectors.
mod layered;
mod svg;
use crate::{
    ControlKind, InspectorControl, InspectorController, InspectorRuntime, InspectorSection,
    InspectorTarget, VideoCard,
};
use shrimply_project_document::{
    project::{AssetSnapshot, LayerVisibility, VideoItem, VideoItemContent},
    svg_color::SvgPaintColor,
};
use shrimply_property_model::timeline_value::{TimelineBool, TimelineValue};
use std::sync::{Arc, mpsc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Svg,
    Layered,
}
#[derive(Clone, PartialEq, Eq)]
struct Key {
    snapshot: AssetSnapshot,
    kind: Kind,
}
struct Layer {
    entry: shrimply_visual_core::layered_image::LayerEntry,
    default: LayerVisibility,
}
enum Data {
    Svg(Vec<SvgPaintColor>),
    Layered(Vec<Layer>),
}
enum State {
    Loading,
    Ready(AssetSnapshot, Arc<Data>),
    Failed(String),
}
type Response = (Key, Result<Arc<Data>, String>);
pub(crate) struct Sources {
    sender: mpsc::Sender<Response>,
    receiver: mpsc::Receiver<Response>,
    active: Option<Key>,
    pending: Vec<Key>,
    state: Option<Result<Arc<Data>, String>>,
}
impl Default for Sources {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active: None,
            pending: Vec::new(),
            state: None,
        }
    }
}
impl Sources {
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, result)) = self.receiver.try_recv() {
            self.pending.retain(|pending| pending != &key);
            if self.active.as_ref() == Some(&key) {
                self.state = Some(result);
                changed = true;
            }
        }
        changed
    }
    fn request(&mut self, item: &VideoItem) -> State {
        let kind = match item.content {
            VideoItemContent::Svg => Kind::Svg,
            VideoItemContent::LayeredImage(_) => Kind::Layered,
            _ => unreachable!("source image metadata kind"),
        };
        let snapshot = match item.file.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => return State::Failed(error),
        };
        let key = Key { snapshot, kind };
        if self.active.as_ref() != Some(&key) {
            self.active = Some(key.clone());
            self.state = None;
        }
        if self.state.is_none() && !self.pending.contains(&key) {
            self.pending.push(key.clone());
            let sender = self.sender.clone();
            let key = key.clone();
            std::thread::spawn(move || {
                let result = (|| {
                    let data = match key.kind {
                        Kind::Svg => Data::Svg(shrimply_project_document::svg_color::paint_colors(
                            &key.snapshot.read_to_string()?,
                        )),
                        Kind::Layered => Data::Layered(
                            shrimply_visual_core::layered_image::entries(
                                key.snapshot.asset().clone(),
                            )?
                            .into_iter()
                            .map(|entry| {
                                let default = LayerVisibility {
                                    id: uuid::Uuid::new_v4(),
                                    path: entry.path.clone(),
                                    visibility: Some(TimelineValue::<TimelineBool>::new_const(
                                        entry.visible.into(),
                                    )),
                                };
                                Layer { entry, default }
                            })
                            .collect(),
                        ),
                    };
                    key.snapshot.verify_current()?;
                    Ok(Arc::new(data))
                })();
                let _ = sender.send((key, result));
            });
        }
        match &self.state {
            None => State::Loading,
            Some(Ok(data)) => State::Ready(key.snapshot, data.clone()),
            Some(Err(error)) => State::Failed(error.clone()),
        }
    }
}
impl InspectorController {
    pub fn source_image_cards(
        &self,
        item: &VideoItem,
        runtime: InspectorRuntime,
    ) -> Vec<VideoCard> {
        let (key, title, reset) = match &item.content {
            VideoItemContent::Svg => ("svg-colors", "SVG Colors", "/svg_color_overrides"),
            VideoItemContent::LayeredImage(_) => {
                ("layered-image-layers", "Layers", "/content/layers")
            }
            _ => return Vec::new(),
        };
        let controls = match self.visual_sources.borrow_mut().request(item) {
            State::Loading => vec![InspectorControl::new(
                ControlKind::InfoLoading,
                "source-image-loading",
                "Loading source metadata…",
            )],
            State::Failed(error) => vec![
                InspectorControl::new(ControlKind::ReadOnly, "source-image-error", "Status")
                    .value(error),
            ],
            State::Ready(snapshot, data) => match &*data {
                Data::Svg(colors) => svg::controls(item, &snapshot, colors),
                Data::Layered(layers) => layered::controls(item, layers, runtime),
            },
        };
        vec![
            VideoCard::new(key, title, InspectorSection { controls }).reset(
                reset,
                serde_json::json!([]),
                if key == "svg-colors" {
                    "reset-svg-colors"
                } else {
                    "reset-layered-image-layers"
                },
            ),
        ]
    }
    pub(crate) fn ensure_image_layer_control(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<(), String> {
        layered::ensure(self, target, control)
    }
}
