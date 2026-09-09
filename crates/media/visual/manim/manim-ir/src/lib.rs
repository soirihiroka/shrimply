use hashbrown::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use shrimply_math_color::{Color, deserialize_array, serialize_array};
use shrimply_math_core::{Fraction, deserialize_fraction, fraction_is_finite, serialize_fraction};

pub const SCHEMA_VERSION: u16 = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneHeader {
    pub scene: String,
    pub scenes: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    #[serde(
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub fps: Fraction,
    #[serde(
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub duration: Fraction,
    pub frame_count: u64,
    pub complete: bool,
    pub render_is_current: bool,
    #[serde(with = "serde_bytes")]
    pub parameters: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StencilOperation {
    Keep,
    Zero,
    IncrementWrap,
    DecrementWrap,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompareFunction {
    Always,
    Equal,
    NotEqual,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StencilFaceState(
    pub StencilOperation,
    pub StencilOperation,
    pub StencilOperation,
);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PipelineState {
    pub depth_test: bool,
    pub depth_write: bool,
    pub color_write: bool,
    pub stencil_compare: CompareFunction,
    pub stencil_front: StencilFaceState,
    pub stencil_back: StencilFaceState,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PipelineResource {
    pub id: u32,
    pub source: String,
    pub state: PipelineState,
    pub texture_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct GeometryResource {
    pub id: u32,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureFormat {
    Rgba8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureFilter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureAddress {
    Clamp,
    Repeat,
    Mirror,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TextureResource {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub filter: TextureFilter,
    pub address: TextureAddress,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct UniformBlock {
    pub id: u32,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceBatch {
    pub pipelines: Vec<PipelineResource>,
    pub geometry: Vec<GeometryResource>,
    pub textures: Vec<TextureResource>,
    pub uniforms: Vec<UniformBlock>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    pub view: Mat4,
    pub frame_scale: f32,
    pub frame_rescale_factors: Vec3,
    pub pixel_size: f32,
    pub camera_position: Vec3,
    pub light_position: Vec3,
    #[serde(
        deserialize_with = "deserialize_array",
        serialize_with = "serialize_array"
    )]
    pub background_rgba: Color,
    #[serde(with = "serde_bytes")]
    pub uniforms: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TextureBinding {
    pub binding: u16,
    pub name: String,
    pub texture: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawCall {
    pub pipeline: u32,
    pub geometry: u32,
    pub uniforms: u32,
    pub vertex_count: u32,
    pub indices: Vec<u32>,
    pub textures: Vec<TextureBinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub index: u64,
    #[serde(
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub time: Fraction,
    pub camera: CameraState,
    pub draws: Vec<DrawCall>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameBatch {
    pub frames: Vec<Frame>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    PreparingEnvironment,
    LoadingScene,
    StreamingFrames,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub stage: ProgressStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum PacketBody {
    Scene(SceneHeader),
    Resources(ResourceBatch),
    Frames(FrameBatch),
    Progress(Progress),
    Finished,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    pub schema_version: u16,
    pub body: PacketBody,
}

impl Packet {
    pub fn new(body: PacketBody) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            body,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::message(format!(
                "unsupported Manim IR schema {}, expected {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if let PacketBody::Scene(scene) = &self.body {
            if scene.width == 0 || scene.height == 0 {
                return Err(Error::message("Manim IR scene dimensions are zero"));
            }
            if !fraction_is_finite(scene.fps) || !fraction_is_finite(scene.duration) {
                return Err(Error::message("Manim IR time is not finite"));
            }
        }
        Ok(())
    }
}

pub fn encode_packet(packet: &Packet) -> Result<Vec<u8>, Error> {
    packet.validate()?;
    rmp_serde::to_vec_named(packet)
        .map_err(|error| Error::message(format!("encode Manim IR packet: {error}")))
}

pub fn decode_packet(bytes: &[u8]) -> Result<Packet, Error> {
    let packet: Packet = rmp_serde::from_slice(bytes)
        .map_err(|error| Error::message(format!("decode Manim IR packet: {error}")))?;
    packet.validate()?;
    Ok(packet)
}

#[derive(Clone, Debug)]
pub struct CompiledAnimation {
    scene: SceneHeader,
    pipelines: HashMap<u32, PipelineResource>,
    geometry: HashMap<u32, GeometryResource>,
    textures: HashMap<u32, TextureResource>,
    uniforms: HashMap<u32, UniformBlock>,
    frames: Vec<Frame>,
}

impl CompiledAnimation {
    pub fn scene(&self) -> &SceneHeader {
        &self.scene
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn pipeline(&self, id: u32) -> Option<&PipelineResource> {
        self.pipelines.get(&id)
    }

    pub fn geometry_resource(&self, id: u32) -> Option<&GeometryResource> {
        self.geometry.get(&id)
    }

    pub fn texture(&self, id: u32) -> Option<&TextureResource> {
        self.textures.get(&id)
    }

    pub fn uniform_block(&self, id: u32) -> Option<&UniformBlock> {
        self.uniforms.get(&id)
    }
}

#[derive(Default)]
pub struct CompiledAnimationBuilder {
    scene: Option<SceneHeader>,
    pipelines: HashMap<u32, PipelineResource>,
    geometry: HashMap<u32, GeometryResource>,
    textures: HashMap<u32, TextureResource>,
    uniforms: HashMap<u32, UniformBlock>,
    pipeline_candidates: HashMap<u64, Vec<u32>>,
    geometry_candidates: HashMap<u64, Vec<u32>>,
    texture_candidates: HashMap<u64, Vec<u32>>,
    uniform_candidates: HashMap<u64, Vec<u32>>,
    pipeline_ids: HashMap<u32, u32>,
    geometry_ids: HashMap<u32, u32>,
    texture_ids: HashMap<u32, u32>,
    uniform_ids: HashMap<u32, u32>,
    frames: Vec<Frame>,
    finished: bool,
}

impl CompiledAnimationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&mut self, packet: Packet) -> Result<(), Error> {
        if self.finished {
            return Err(Error::message("Manim IR packet follows Finished"));
        }
        match packet.body {
            PacketBody::Scene(scene) => {
                if self.scene.replace(scene).is_some() {
                    return Err(Error::message("Manim IR contains multiple scene headers"));
                }
            }
            PacketBody::Resources(resources) => {
                for pipeline in resources.pipelines {
                    let incoming = pipeline.id;
                    let hash =
                        content_hash(&(&pipeline.source, pipeline.state, &pipeline.texture_names));
                    let canonical = self.pipeline_candidates.get(&hash).and_then(|ids| {
                        ids.iter().copied().find(|id| {
                            let existing = &self.pipelines[id];
                            existing.source == pipeline.source
                                && existing.state == pipeline.state
                                && existing.texture_names == pipeline.texture_names
                        })
                    });
                    let canonical = canonical.unwrap_or_else(|| {
                        self.pipeline_candidates
                            .entry(hash)
                            .or_default()
                            .push(incoming);
                        self.pipelines.insert(incoming, pipeline);
                        incoming
                    });
                    if self.pipeline_ids.insert(incoming, canonical).is_some() {
                        return Err(Error::message(format!(
                            "duplicate incoming Manim pipeline {incoming}"
                        )));
                    }
                }
                for geometry in resources.geometry {
                    let incoming = geometry.id;
                    let hash = content_hash(&geometry.bytes);
                    let canonical = self.geometry_candidates.get(&hash).and_then(|ids| {
                        ids.iter().copied().find(|id| {
                            let existing = &self.geometry[id];
                            existing.bytes == geometry.bytes
                        })
                    });
                    let canonical = canonical.unwrap_or_else(|| {
                        self.geometry_candidates
                            .entry(hash)
                            .or_default()
                            .push(incoming);
                        self.geometry.insert(incoming, geometry);
                        incoming
                    });
                    if self.geometry_ids.insert(incoming, canonical).is_some() {
                        return Err(Error::message(format!(
                            "duplicate incoming Manim geometry {incoming}"
                        )));
                    }
                }
                for texture in resources.textures {
                    let incoming = texture.id;
                    let hash = content_hash(&(
                        texture.width,
                        texture.height,
                        texture.format,
                        texture.filter,
                        texture.address,
                        &texture.bytes,
                    ));
                    let canonical = self.texture_candidates.get(&hash).and_then(|ids| {
                        ids.iter().copied().find(|id| {
                            let existing = &self.textures[id];
                            existing.width == texture.width
                                && existing.height == texture.height
                                && existing.format == texture.format
                                && existing.filter == texture.filter
                                && existing.address == texture.address
                                && existing.bytes == texture.bytes
                        })
                    });
                    let canonical = canonical.unwrap_or_else(|| {
                        self.texture_candidates
                            .entry(hash)
                            .or_default()
                            .push(incoming);
                        self.textures.insert(incoming, texture);
                        incoming
                    });
                    if self.texture_ids.insert(incoming, canonical).is_some() {
                        return Err(Error::message(format!(
                            "duplicate incoming Manim texture {incoming}"
                        )));
                    }
                }
                for uniforms in resources.uniforms {
                    let incoming = uniforms.id;
                    let hash = content_hash(&uniforms.bytes);
                    let canonical = self.uniform_candidates.get(&hash).and_then(|ids| {
                        ids.iter()
                            .copied()
                            .find(|id| self.uniforms[id].bytes == uniforms.bytes)
                    });
                    let canonical = canonical.unwrap_or_else(|| {
                        self.uniform_candidates
                            .entry(hash)
                            .or_default()
                            .push(incoming);
                        self.uniforms.insert(incoming, uniforms);
                        incoming
                    });
                    if self.uniform_ids.insert(incoming, canonical).is_some() {
                        return Err(Error::message(format!(
                            "duplicate incoming Manim uniforms {incoming}"
                        )));
                    }
                }
            }
            PacketBody::Frames(batch) => {
                for mut frame in batch.frames {
                    for draw in &mut frame.draws {
                        draw.pipeline =
                            self.pipeline_ids
                                .get(&draw.pipeline)
                                .copied()
                                .ok_or_else(|| {
                                    Error::message(format!(
                                        "Manim frame {} references unknown incoming pipeline {}",
                                        frame.index, draw.pipeline
                                    ))
                                })?;
                        draw.geometry =
                            self.geometry_ids
                                .get(&draw.geometry)
                                .copied()
                                .ok_or_else(|| {
                                    Error::message(format!(
                                        "Manim frame {} references unknown incoming geometry {}",
                                        frame.index, draw.geometry
                                    ))
                                })?;
                        draw.uniforms =
                            self.uniform_ids
                                .get(&draw.uniforms)
                                .copied()
                                .ok_or_else(|| {
                                    Error::message(format!(
                                        "Manim frame {} references unknown incoming uniforms {}",
                                        frame.index, draw.uniforms
                                    ))
                                })?;
                        for binding in &mut draw.textures {
                            binding.texture =
                                self.texture_ids.get(&binding.texture).copied().ok_or_else(
                                    || {
                                        Error::message(format!(
                                            "Manim frame {} references unknown incoming texture {}",
                                            frame.index, binding.texture
                                        ))
                                    },
                                )?;
                        }
                    }
                    self.frames.push(frame);
                }
            }
            PacketBody::Progress(_) => {}
            PacketBody::Finished => self.finished = true,
            PacketBody::Error(error) => return Err(Error::message(error)),
        }
        Ok(())
    }

    pub fn finish(self) -> Result<CompiledAnimation, Error> {
        if !self.finished {
            return Err(Error::message("Manim IR ended before Finished"));
        }
        let scene = self
            .scene
            .ok_or_else(|| Error::message("Manim IR has no scene header"))?;
        if !scene.complete {
            return Err(Error::message("Manim IR scene is incomplete"));
        }
        if scene.frame_count != self.frames.len() as u64 {
            return Err(Error::message(format!(
                "Manim IR declares {} frames but contains {}",
                scene.frame_count,
                self.frames.len()
            )));
        }
        for (index, frame) in self.frames.iter().enumerate() {
            if frame.index != index as u64 {
                return Err(Error::message(format!(
                    "Manim IR frame {} has index {}",
                    index, frame.index
                )));
            }
            for draw in &frame.draws {
                if !self.pipelines.contains_key(&draw.pipeline) {
                    return Err(Error::message(format!(
                        "Manim frame {} references missing pipeline {}",
                        frame.index, draw.pipeline
                    )));
                }
                self.geometry.get(&draw.geometry).ok_or_else(|| {
                    Error::message(format!(
                        "Manim frame {} references missing geometry {}",
                        frame.index, draw.geometry
                    ))
                })?;
                if !self.uniforms.contains_key(&draw.uniforms) {
                    return Err(Error::message(format!(
                        "Manim frame {} references missing uniforms {}",
                        frame.index, draw.uniforms
                    )));
                }
                if draw.vertex_count == 0 {
                    return Err(Error::message(format!(
                        "Manim frame {} contains an empty draw",
                        frame.index
                    )));
                }
                for binding in &draw.textures {
                    if !self.textures.contains_key(&binding.texture) {
                        return Err(Error::message(format!(
                            "Manim frame {} references missing texture {}",
                            frame.index, binding.texture
                        )));
                    }
                }
            }
        }
        Ok(CompiledAnimation {
            scene,
            pipelines: self.pipelines,
            geometry: self.geometry,
            textures: self.textures,
            uniforms: self.uniforms,
            frames: self.frames,
        })
    }
}

fn content_hash(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error(String);

impl Error {
    fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}
