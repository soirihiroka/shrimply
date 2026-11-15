use shrimply_layered_image_loader::LayeredImage;
use shrimply_project_document::project::LayeredImageItem;
use shrimply_project_evaluation::{TransformExpressionCache, VisualEvaluation, resolve_bool};
use std::sync::Arc;

pub use shrimply_layered_image_loader::LayerEntry;

/// Portable source hierarchy used by the inspector without a renderer dependency.
pub fn entries(source: shrimply_asset::Asset) -> Result<Vec<LayerEntry>, String> {
    shrimply_layered_image_loader::load(source).map(|document| document.entries.clone())
}

const LAYER_NOISE_MULTIPLIER: u32 = 0x85eb_ca6b;

pub struct Prepared {
    pub document: Arc<LayeredImage>,
    pub source_key: String,
    pub layers: Vec<Layer>,
}

impl Prepared {
    pub fn append_cache_discriminator(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&(self.layers.len() as u64).to_le_bytes());
        for layer in &self.layers {
            output.extend_from_slice(&(layer.source as u64).to_le_bytes());
            match layer.clipping_base {
                Some((source, opacity)) => {
                    output.push(1);
                    output.extend_from_slice(&(source as u64).to_le_bytes());
                    output.extend_from_slice(&opacity.to_bits().to_le_bytes());
                }
                None => output.push(0),
            }
            output.push(layer.mode as u8);
            output.extend_from_slice(&layer.opacity.to_bits().to_le_bytes());
            output.extend_from_slice(&layer.noise_seed.to_le_bytes());
        }
    }
}

pub struct Layer {
    pub source: usize,
    pub clipping_base: Option<(usize, f32)>,
    pub mode: shrimply_render_core::LayerBlendMode,
    pub opacity: f32,
    pub noise_seed: u32,
}

pub fn prepare(
    document: Arc<LayeredImage>,
    source_key: String,
    item: &LayeredImageItem,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> Prepared {
    let visible = document
        .layers
        .iter()
        .map(|layer| {
            resolved_visibility(&layer.path, layer.visible, item, evaluation, expressions)
                && parents_visible(&document, layer.parent, item, evaluation, expressions)
        })
        .collect::<Vec<_>>();
    let mut layers = Vec::with_capacity(document.layers.len());
    let mut clipping_base = None::<(Option<u32>, usize, f32)>;
    for (index, layer) in document.layers.iter().enumerate().rev() {
        if !visible[index] {
            if !layer.clipped {
                clipping_base = None;
            }
            continue;
        }
        let opacity =
            f32::from(layer.opacity) / f32::from(u8::MAX) * parent_opacity(&document, layer.parent);
        if !layer.clipped {
            clipping_base = Some((layer.parent, index, opacity));
        }
        let clipping_base = layer
            .clipped
            .then_some(clipping_base)
            .flatten()
            .filter(|(parent, _, _)| *parent == layer.parent)
            .map(|(_, index, opacity)| (index, opacity));
        if layer.clipped && clipping_base.is_none() {
            continue;
        }
        layers.push(Layer {
            source: index,
            clipping_base,
            mode: layer.blend_mode,
            opacity,
            noise_seed: (index as u32).wrapping_mul(LAYER_NOISE_MULTIPLIER),
        });
    }
    Prepared {
        document,
        source_key,
        layers,
    }
}

fn resolved_visibility(
    path: &str,
    source_visible: bool,
    item: &LayeredImageItem,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> bool {
    item.layers
        .iter()
        .find(|entry| entry.path == path)
        .and_then(|entry| entry.visibility.as_ref())
        .map_or(source_visible, |value| {
            resolve_bool(value, evaluation, expressions)
        })
}

fn parents_visible(
    document: &LayeredImage,
    mut parent: Option<u32>,
    item: &LayeredImageItem,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> bool {
    while let Some(id) = parent {
        let Some(group) = document.groups.iter().find(|group| group.id == id) else {
            break;
        };
        if !resolved_visibility(&group.path, group.visible, item, evaluation, expressions) {
            return false;
        }
        parent = group.parent;
    }
    true
}

fn parent_opacity(document: &LayeredImage, mut parent: Option<u32>) -> f32 {
    let mut opacity = 1.0;
    while let Some(id) = parent {
        let Some(group) = document.groups.iter().find(|group| group.id == id) else {
            break;
        };
        opacity *= f32::from(group.opacity) / f32::from(u8::MAX);
        parent = group.parent;
    }
    opacity
}
