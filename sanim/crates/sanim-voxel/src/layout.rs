//! Deterministic three-axis layout in voxel cells, independent of cameras and rendering.
mod math;
use crate::{Volume, revision};
use fraction::Fraction;
use imbl::Vector;
pub use math::{coordinate, prepare, resolve};
use math::{percent, validate_placement};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};

const MAX_DEPTH: usize = 64;
pub const MAX_NODES: usize = 4096;
pub const MAX_LAYOUT_WORK: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Dimension {
    Cells(u32),
    Percent(Fraction),
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Coordinate {
    Cells(Fraction),
    Percent(Fraction),
}
impl Default for Coordinate {
    fn default() -> Self {
        Self::Cells(Fraction::from(0))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}
impl Axis {
    fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}
impl Align {
    fn shift(self, extra: u32) -> Fraction {
        match self {
            Self::Start => Fraction::from(0),
            Self::Center => Fraction::new(u64::from(extra), 2u64),
            Self::End => Fraction::from(extra),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub absolute: bool,
    pub at: [Coordinate; 3],
    pub anchor: [Coordinate; 3],
    pub offset: [Fraction; 3],
    pub align: Option<[Align; 3]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub size: [Dimension; 3],
    pub flow: Axis,
    pub gap: u32,
    /// Symmetric padding on each axis: [left/right, bottom/top, back/front].
    pub padding: [u32; 3],
    pub justify: Justify,
    pub align: [Align; 3],
    pub children: Vector<Arc<Node>>,
    pub placement: Placement,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Content {
    Space(Space),
    Part {
        volume: Arc<Volume>,
        placement: Placement,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Node {
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    fingerprint: [u8; 32],
    content: Content,
}
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
    }
}
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            content: Content,
        }
        match Fields::deserialize(deserializer)?.content {
            Content::Space(space) => Self::space(space),
            Content::Part { volume, placement } => Self::part(volume, placement),
        }
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub path: Vec<usize>,
    pub origin: [u32; 3],
    pub size: [u32; 3],
    pub leaf: bool,
}

#[derive(Clone, Debug)]
pub struct Resolved {
    pub source: crate::SceneSource,
    pub placements: Arc<[Placed]>,
    pub work: usize,
}

/// A validated, snapped assembly. Planning never copies voxel payloads.
#[derive(Clone, Debug)]
pub struct Plan {
    dimensions: [u32; 3],
    placements: Arc<[Placed]>,
    work: usize,
    parts: Vec<(Arc<Volume>, [i32; 3])>,
    fingerprint: [u8; 32],
}
impl Plan {
    pub fn placements(&self) -> &Arc<[Placed]> {
        &self.placements
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
}

impl Node {
    pub fn space(space: Space) -> Result<Self, String> {
        if space.children.len() > MAX_NODES {
            return Err(format!("VoxelBox exceeds its {MAX_NODES}-child budget"));
        }
        for dimension in space.size {
            if let Dimension::Percent(value) = dimension {
                percent(value)?;
            }
        }
        validate_placement(&space.placement)?;
        let children: Vec<_> = space
            .children
            .iter()
            .map(|child| child.fingerprint)
            .collect();
        let encoded = serde_json::to_vec(&(
            "space",
            space.size,
            space.flow,
            space.gap,
            space.padding,
            space.justify,
            space.align,
            &space.placement,
            children,
        ))
        .map_err(|error| error.to_string())?;
        Ok(Self {
            revision: revision(),
            fingerprint: Sha256::digest(encoded).into(),
            content: Content::Space(space),
        })
    }
    pub fn part(volume: Arc<Volume>, placement: Placement) -> Result<Self, String> {
        validate_placement(&placement)?;
        let encoded = serde_json::to_vec(&("part", volume.revision(), &placement))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            revision: revision(),
            fingerprint: Sha256::digest(encoded).into(),
            content: Content::Part { volume, placement },
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
    pub fn same_recipe(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
    pub fn children(&self) -> Option<&Vector<Arc<Node>>> {
        if let Content::Space(space) = &self.content {
            Some(&space.children)
        } else {
            None
        }
    }
    pub fn volume(&self) -> Option<&Arc<Volume>> {
        if let Content::Part { volume, .. } = &self.content {
            Some(volume)
        } else {
            None
        }
    }
    fn placement(&self) -> &Placement {
        match &self.content {
            Content::Space(space) => &space.placement,
            Content::Part { placement, .. } => placement,
        }
    }
    /// Distinct immutable volume payloads; hosts can account their existing weak identities.
    pub fn volumes(&self) -> Vec<Arc<Volume>> {
        let mut stack = vec![self];
        let mut seen = BTreeSet::new();
        let mut volumes = Vec::new();
        while let Some(node) = stack.pop() {
            if !seen.insert(node.revision) {
                continue;
            }
            match &node.content {
                Content::Space(space) => stack.extend(space.children.iter().map(Arc::as_ref)),
                Content::Part { volume, .. } => volumes.push(volume.clone()),
            }
        }
        volumes
    }
    pub fn resident_bytes(&self) -> usize {
        let mut stack = vec![self];
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        while let Some(node) = stack.pop() {
            if !seen.insert(node.revision) {
                continue;
            }
            bytes = bytes.saturating_add(std::mem::size_of::<Self>());
            match &node.content {
                Content::Space(space) => {
                    bytes = bytes
                        .saturating_add(space.children.len() * std::mem::size_of::<Arc<Node>>());
                    stack.extend(space.children.iter().map(Arc::as_ref));
                }
                Content::Part { .. } => {}
            }
        }
        bytes
    }
}
