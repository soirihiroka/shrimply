use hashbrown::HashMap;
use std::sync::Arc;

use shrimply_asset::Asset;

pub use shrimply_render_core::LayerBlendMode as BlendMode;

pub struct LayeredImage {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<BitmapLayer>,
    pub groups: Vec<LayerGroup>,
    pub entries: Vec<LayerEntry>,
}

pub struct BitmapLayer {
    pub path: String,
    pub name: String,
    pub parent: Option<u32>,
    pub rgba: Vec<u8>,
    pub visible: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub clipped: bool,
}

pub struct LayerGroup {
    pub id: u32,
    pub path: String,
    pub name: String,
    pub parent: Option<u32>,
    pub visible: bool,
    pub opacity: u8,
}

#[derive(Clone, Debug)]
pub struct LayerEntry {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub visible: bool,
    pub group: bool,
}

pub fn load(source: impl Into<Asset>) -> Result<Arc<LayeredImage>, String> {
    let asset = source.into();
    let snapshot = asset.snapshot()?;
    let bytes = snapshot.read()?;
    let extension = asset
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| format!("{} has no file extension", asset.display()))?;
    let image = if extension.eq_ignore_ascii_case("psd") {
        psd_parser::parse(&bytes)
            .and_then(from_psd)
            .map_err(|error| format!("could not parse {}: {error}", asset.display()))
    } else if extension.eq_ignore_ascii_case("kra") {
        kra_parser::parse(&bytes)
            .and_then(from_kra)
            .map_err(|error| format!("could not parse {}: {error}", asset.display()))
    } else {
        Err(format!("unsupported layered image extension {extension:?}",))
    }?;
    snapshot.verify_current()?;
    Ok(Arc::new(image))
}

fn from_psd(document: psd_parser::Document) -> Result<LayeredImage, String> {
    let mut duplicates = HashMap::<String, u32>::new();
    let layers = document
        .layers
        .iter()
        .map(|layer| {
            let mut names = psd_parser::parent_names(&document, layer.parent);
            names.push(layer.name.clone());
            let base = names.join("/");
            let duplicate = duplicates.entry(base.clone()).or_default();
            let path = format!("{base}#{duplicate}");
            *duplicate += 1;
            Ok(BitmapLayer {
                path,
                name: layer.name.clone(),
                parent: layer.parent,
                rgba: layer.rgba.clone(),
                visible: layer.visible,
                opacity: layer.opacity,
                blend_mode: psd_blend_mode(&layer.blend_mode)?,
                clipped: layer.clipped,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut duplicates = HashMap::<String, u32>::new();
    let groups = document
        .groups
        .iter()
        .map(|group| {
            let mut names = psd_parser::parent_names(&document, group.parent);
            names.push(group.name.clone());
            let base = format!("{}#group", names.join("/"));
            let duplicate = duplicates.entry(base.clone()).or_default();
            let path = if *duplicate == 0 {
                base
            } else {
                format!("{base}#{duplicate}")
            };
            *duplicate += 1;
            LayerGroup {
                id: group.id,
                path,
                name: group.name.clone(),
                parent: group.parent,
                visible: group.visible,
                opacity: group.opacity,
            }
        })
        .collect::<Vec<_>>();
    let mut entries = groups
        .iter()
        .rev()
        .map(|group| LayerEntry {
            path: group.path.clone(),
            name: group.name.clone(),
            depth: parent_depth(&groups, group.parent),
            visible: group.visible,
            group: true,
        })
        .collect::<Vec<_>>();
    entries.extend(layers.iter().rev().map(|layer| LayerEntry {
        path: layer.path.clone(),
        name: layer.name.clone(),
        depth: parent_depth(&groups, layer.parent),
        visible: layer.visible,
        group: false,
    }));
    Ok(LayeredImage {
        width: document.width,
        height: document.height,
        layers,
        groups,
        entries,
    })
}

fn from_kra(document: kra_parser::Document) -> Result<LayeredImage, String> {
    let parser_groups = document
        .groups
        .iter()
        .map(|group| (group.id, group))
        .collect::<HashMap<_, _>>();
    let parent_names = |mut parent: Option<u32>| {
        let mut names = Vec::new();
        while let Some(id) = parent {
            let Some(group) = parser_groups.get(&id) else {
                break;
            };
            names.push(group.name.clone());
            parent = group.parent;
        }
        names.reverse();
        names
    };
    let mut duplicates = HashMap::<String, u32>::new();
    let layers = document
        .layers
        .iter()
        .map(|layer| {
            let mut names = parent_names(layer.parent);
            names.push(layer.name.clone());
            let base = names.join("/");
            let duplicate = duplicates.entry(base.clone()).or_default();
            let path = format!("{base}#{duplicate}");
            *duplicate += 1;
            Ok(BitmapLayer {
                path,
                name: layer.name.clone(),
                parent: layer.parent,
                rgba: layer.rgba.clone(),
                visible: layer.visible,
                opacity: layer.opacity,
                blend_mode: kra_blend_mode(&layer.blend_mode)?,
                clipped: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut duplicates = HashMap::<String, u32>::new();
    let groups = document
        .groups
        .iter()
        .map(|group| {
            let mut names = parent_names(group.parent);
            names.push(group.name.clone());
            let base = format!("{}#group", names.join("/"));
            let duplicate = duplicates.entry(base.clone()).or_default();
            let path = if *duplicate == 0 {
                base
            } else {
                format!("{base}#{duplicate}")
            };
            *duplicate += 1;
            LayerGroup {
                id: group.id,
                path,
                name: group.name.clone(),
                parent: group.parent,
                visible: group.visible,
                opacity: group.opacity,
            }
        })
        .collect::<Vec<_>>();
    let entries = document
        .nodes
        .iter()
        .filter_map(|node| match *node {
            kra_parser::Node::Layer(index) => {
                let layer = layers.get(index)?;
                Some(LayerEntry {
                    path: layer.path.clone(),
                    name: layer.name.clone(),
                    depth: parent_depth(&groups, layer.parent),
                    visible: layer.visible,
                    group: false,
                })
            }
            kra_parser::Node::Group(id) => {
                let group = groups.iter().find(|group| group.id == id)?;
                Some(LayerEntry {
                    path: group.path.clone(),
                    name: group.name.clone(),
                    depth: parent_depth(&groups, group.parent),
                    visible: group.visible,
                    group: true,
                })
            }
        })
        .collect();
    Ok(LayeredImage {
        width: document.width,
        height: document.height,
        layers,
        groups,
        entries,
    })
}

fn parent_depth(groups: &[LayerGroup], mut parent: Option<u32>) -> usize {
    let mut depth = 0;
    while let Some(id) = parent {
        let Some(group) = groups.iter().find(|group| group.id == id) else {
            break;
        };
        depth += 1;
        parent = group.parent;
    }
    depth
}

fn psd_blend_mode(name: &str) -> Result<BlendMode, String> {
    use BlendMode::*;
    Ok(match name {
        "PassThrough" => PassThrough,
        "Normal" => Normal,
        "Dissolve" => Dissolve,
        "Darken" => Darken,
        "Multiply" => Multiply,
        "ColorBurn" => ColorBurn,
        "LinearBurn" => LinearBurn,
        "DarkerColor" => DarkerColor,
        "Lighten" => Lighten,
        "Screen" => Screen,
        "ColorDodge" => ColorDodge,
        "LinearDodge" => Add,
        "LighterColor" => LighterColor,
        "Overlay" => Overlay,
        "SoftLight" => SoftLight,
        "HardLight" => HardLight,
        "VividLight" => VividLight,
        "LinearLight" => LinearLight,
        "PinLight" => PinLight,
        "HardMix" => HardMix,
        "Difference" => Difference,
        "Exclusion" => Exclusion,
        "Subtract" => Subtract,
        "Divide" => Divide,
        "Hue" => Hue,
        "Saturation" => Saturation,
        "Color" => Color,
        "Luminosity" => Luminosity,
        _ => return Err(format!("unsupported PSD blend mode {name}")),
    })
}

fn kra_blend_mode(name: &str) -> Result<BlendMode, String> {
    use BlendMode::*;
    Ok(match name {
        "normal" => Normal,
        "dissolve" => Dissolve,
        "darken" => Darken,
        "multiply" => Multiply,
        "burn" => ColorBurn,
        "linear_burn" => LinearBurn,
        "darker color" => DarkerColor,
        "lighten" => Lighten,
        "screen" => Screen,
        "dodge" => ColorDodge,
        "linear_dodge" | "add" => Add,
        "lighter color" => LighterColor,
        "overlay" => Overlay,
        "soft_light" => SoftLight,
        "hard_light" => HardLight,
        "vivid_light" => VividLight,
        "linear light" => LinearLight,
        "pin_light" => PinLight,
        "hard_mix_photoshop" | "hard mix" => HardMix,
        "diff" => Difference,
        "exclusion" => Exclusion,
        "subtract" => Subtract,
        "divide" => Divide,
        "reflect" => Reflect,
        "hue" => Hue,
        "saturation" => Saturation,
        "color" => Color,
        "luminize" => Luminosity,
        _ => return Err(format!("unsupported Krita blend mode {name}")),
    })
}
