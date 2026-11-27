use hashbrown::HashMap;

use rayon::prelude::*;

pub struct Document {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub groups: Vec<Group>,
}

pub struct Layer {
    pub name: String,
    pub parent: Option<u32>,
    pub rgba: Vec<u8>,
    pub visible: bool,
    pub opacity: u8,
    pub blend_mode: String,
    pub clipped: bool,
}

pub struct Group {
    pub id: u32,
    pub name: String,
    pub parent: Option<u32>,
    pub visible: bool,
    pub opacity: u8,
}

pub fn parse(bytes: &[u8]) -> Result<Document, String> {
    let document = psd::Psd::from_bytes(bytes).map_err(|error| error.to_string())?;
    let groups = document
        .group_ids_in_order()
        .iter()
        .filter_map(|id| {
            let group = document.groups().get(id)?;
            Some(Group {
                id: *id,
                name: group.name().to_string(),
                parent: group.parent_id(),
                // psd 0.3.5 exposes the PSD hidden bit through `visible`.
                visible: !group.visible(),
                opacity: group.opacity(),
            })
        })
        .collect();
    let layers = document
        .layers()
        .par_iter()
        .map(|layer| {
            let mut rgba = layer.rgba();
            rgba.as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
                .for_each(|(index, pixel)| {
                    let x = (index % document.width() as usize) as i32;
                    let y = (index / document.width() as usize) as i32;
                    if x < layer.layer_left()
                        || x > layer.layer_right()
                        || y < layer.layer_top()
                        || y > layer.layer_bottom()
                    {
                        pixel.fill(0);
                    }
                });
            Layer {
                name: layer.name().to_string(),
                parent: layer.parent_id(),
                rgba,
                // psd 0.3.5 exposes the PSD hidden bit through `visible`.
                visible: !layer.visible(),
                opacity: layer.opacity(),
                blend_mode: format!("{:?}", layer.blend_mode()),
                // PSD stores zero for a clipping base and one for a clipped layer.
                clipped: !layer.is_clipping_mask(),
            }
        })
        .collect();
    Ok(Document {
        width: document.width(),
        height: document.height(),
        layers,
        groups,
    })
}

pub fn parent_names(document: &Document, mut parent: Option<u32>) -> Vec<String> {
    let groups = document
        .groups
        .iter()
        .map(|group| (group.id, group))
        .collect::<HashMap<_, _>>();
    let mut names = Vec::new();
    while let Some(id) = parent {
        let Some(group) = groups.get(&id) else {
            break;
        };
        names.push(group.name.clone());
        parent = group.parent;
    }
    names.reverse();
    names
}
