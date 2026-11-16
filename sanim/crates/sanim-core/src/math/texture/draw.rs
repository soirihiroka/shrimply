//! Optional native 2D baking; the host supplies the existing renderer, not a second painter.
use super::*;
use crate::{Element, Font, Geometry};
use std::sync::{Mutex, OnceLock, Weak};
const PATH_EXPANSION: usize = 4;
const BOX_PATH_POINTS: usize = 64;
const CODE_PATH_POINTS_PER_BYTE: usize = 20;
const MAX_LAYER_WORK_PER_ELEMENT: usize = 3;
const ENCODED_IMAGE_WORKING_BYTES_PER_PIXEL: usize = 16;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const MAX_PROOF_BYTES: usize = 1024 * 1024;
const MAX_PROOF_ENTRIES: usize = 1024;

pub type Rasterizer = fn(&Element, [u32; 2], usize) -> Result<Vec<u8>, String>;
pub struct Backend {
    pub rasterize: Rasterizer,
    pub font_epoch: fn() -> Option<String>,
}
static BACKEND: OnceLock<Backend> = OnceLock::new();
/// Install a default host service once. An explicitly installed host backend wins.
pub fn install(backend: Backend) {
    BACKEND.get_or_init(|| backend);
}
fn backend() -> Result<&'static Backend, String> {
    BACKEND.get().ok_or_else(||"Texture.draw requires the native 2D rendering feature; use a standard Sanim CLI/editor build, or install a texture drawing backend in the host".into())
}
fn content(args: &Arguments) -> Result<&Arc<Element>, String> {
    match required(args, "content")? {
        Value::Element(element) => Ok(element),
        _ => Err("Texture.draw content must be a 2D Element such as Box, Label or Image".into()),
    }
}
struct Summary {
    nodes: usize,
    points: usize,
    layers: usize,
    family: bool,
    bytes: usize,
    media_work: usize,
}
fn inspect(root: &Element) -> Result<Summary, String> {
    let mut stack = vec![(root, 0usize, 0usize)];
    let mut nodes = 0usize;
    let mut points = 0usize;
    let mut peak_layers = 0usize;
    let mut family = false;
    let mut media_bytes = 0usize;
    let mut media_work = 0usize;
    while let Some((element, depth, layers)) = stack.pop() {
        if depth >= crate::reactive::MAX_REACTIVE_DEPTH {
            return Err("Texture.draw content exceeds the element depth budget".into());
        }
        nodes = nodes
            .checked_add(1)
            .ok_or("Texture.draw model work overflow")?;
        match element.geometry.as_ref() {
            Geometry::Voxel(_)=>return Err("Texture.draw requires a 2D element; voxel scenes use the independent voxel renderer".into()),
            Geometry::Text{font,outline,..}=>{
                family|=matches!(font,Font::Family(_));
                points=points.checked_add(outline.len()).ok_or("Texture.draw path work overflow")?;
            }
            Geometry::Code{document,..}=>{
                family=true;
                points=points.checked_add(document.text.len().saturating_mul(CODE_PATH_POINTS_PER_BYTE)).ok_or("Texture.draw code work overflow")?;
            },
            Geometry::Media{asset,..} => {
                if asset.kind == crate::media::Kind::Svg {
                    return Err("Texture.draw cannot bound SVG decoding memory; use a bitmap Image or native vector elements instead".into());
                }
                let pixels=checked_product(asset.width as usize,asset.height as usize)?;
                let bytes=match asset.source() {
                    crate::media::Source::Encoded(bytes)=>checked_product(pixels,ENCODED_IMAGE_WORKING_BYTES_PER_PIXEL)?.checked_add(bytes.len()).ok_or("Texture.draw image byte overflow")?,
                    crate::media::Source::Texture(_)=>checked_product(pixels,RGBA_BYTES_PER_PIXEL)?,
                };
                media_bytes=media_bytes.checked_add(bytes).ok_or("Texture.draw image byte overflow")?;
                media_work=media_work.checked_add(pixels).ok_or("Texture.draw image work overflow")?;
            },
            Geometry::Polygon{points:vertices}|Geometry::Polyline{points:vertices,..}=>{
                points=points.checked_add(vertices.len()).ok_or("Texture.draw path work overflow")?;
            }
            _=>{}
        }
        if nodes.saturating_add(points) > crate::reactive::MAX_REACTIVE_NODES {
            return Err("Texture.draw content exceeds the element/path work budget".into());
        }
        let extra = usize::from(
            element.opacity != crate::Fraction::from(1) && element.geometry.children().is_some(),
        ) + usize::from(element.reveal.progress < crate::Fraction::from(1));
        peak_layers = peak_layers.max(layers + extra);
        if let Some(children) = element.geometry.children() {
            if children.len()
                > crate::reactive::MAX_REACTIVE_NODES.saturating_sub(nodes + stack.len())
            {
                return Err("Texture.draw content exceeds the element budget".into());
            }
            stack.extend(
                children
                    .iter()
                    .map(|child| (child, depth + 1, layers + extra)),
            );
        }
    }
    Ok(Summary {
        nodes,
        points,
        layers: peak_layers,
        family,
        bytes: media_bytes,
        media_work,
    })
}
pub(super) fn cost(args: &Arguments) -> Result<Cost, String> {
    backend()?;
    let source = texture(args, "texture")?;
    let proof = proof(content(args)?)?;
    let summary = &proof.summary;
    let bytes = source.pixels().len();
    let pixels = bytes / 4;
    Ok(Cost {
        work: checked_product(
            pixels,
            summary
                .nodes
                .saturating_mul(MAX_LAYER_WORK_PER_ELEMENT)
                .saturating_add(2),
        )?
        .checked_add(summary.points.saturating_mul(PATH_EXPANSION))
        .and_then(|work| work.checked_add(summary.nodes.saturating_mul(BOX_PATH_POINTS)))
        .and_then(|work| work.checked_add(summary.media_work))
        .ok_or("Texture.draw work overflow")?,
        // Native surface, RGBA readback, compositing output and Arc publication.
        bytes: checked_product(bytes, 4usize.saturating_add(summary.layers))?
            .checked_add(summary.bytes)
            .ok_or("Texture.draw working bytes overflow")?,
        output_bytes: bytes,
    })
}
pub(super) fn execute(args: &Arguments, limits: Cost) -> Result<Value, String> {
    let source = texture(args, "texture")?;
    let pixels = (backend()?.rasterize)(
        content(args)?,
        [source.width(), source.height()],
        limits.work,
    )?;
    let painted = sanim_voxel::Texture::new(source.width(), source.height(), pixels.into())?;
    let composite = image::composite(
        source,
        &painted,
        [0, 0],
        shrimply_math_color::LayerBlendMode::Normal,
        1.0,
    );
    publish([source.width(), source.height()], composite.into_raw())
}

struct Proof {
    element: Weak<Element>,
    key: String,
    summary: Summary,
}
fn proof(element: &Arc<Element>) -> Result<Arc<Proof>, String> {
    static PROOFS: OnceLock<Mutex<crate::cache::Cache<usize, Proof>>> = OnceLock::new();
    let proofs = PROOFS
        .get_or_init(|| Mutex::new(crate::cache::Cache::new(MAX_PROOF_BYTES, MAX_PROOF_ENTRIES)));
    let id = Arc::as_ptr(element) as usize;
    if let Some(proof) = proofs
        .lock()
        .expect("texture drawing proof cache poisoned")
        .get(&id)
        && proof
            .element
            .upgrade()
            .is_some_and(|old| Arc::ptr_eq(&old, element))
    {
        return Ok(proof);
    }
    let mut summary = inspect(element)?;
    let mut model_bytes = MAX_BYTES;
    let mut items = crate::reactive::MAX_REACTIVE_NODES;
    crate::value_budget::charge(
        &Value::Element(element.clone()),
        &mut model_bytes,
        &mut items,
        crate::reactive::MAX_REACTIVE_DEPTH,
    )?;
    summary.bytes = summary
        .bytes
        .checked_add(MAX_BYTES - model_bytes)
        .ok_or("Texture.draw model byte overflow")?;
    let mut writer = shrimply_resource_pipeline::fingerprint::Writer::default();
    serde_json::to_writer(
        &mut writer,
        &crate::value_key::ValueKey(&Value::Element(element.clone())),
    )
    .map_err(|error| format!("Texture.draw content proof: {error}"))?;
    let key = writer.finish();
    let size = std::mem::size_of::<Proof>() + std::mem::size_of::<Element>() + key.len();
    let proof = Arc::new(Proof {
        element: Arc::downgrade(element),
        key,
        summary,
    });
    proofs
        .lock()
        .expect("texture drawing proof cache poisoned")
        .insert(id, proof.clone(), size);
    Ok(proof)
}
pub(super) fn key(args: &Arguments) -> Result<Option<String>, String> {
    let source = texture(args, "texture")?;
    let element = content(args)?;
    let proof = proof(element)?;
    let epoch = if proof.summary.family {
        let Some(epoch) = (backend()?.font_epoch)() else {
            return Ok(None);
        };
        epoch
    } else {
        String::new()
    };
    let mut writer = shrimply_resource_pipeline::fingerprint::Writer::default();
    serde_json::to_writer(
        &mut writer,
        &(Op::Draw, source.revision(), &proof.key, epoch),
    )
    .map_err(|error| format!("Texture.draw cache proof: {error}"))?;
    Ok(Some(writer.finish()))
}
