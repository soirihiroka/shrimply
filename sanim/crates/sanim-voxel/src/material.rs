use crate::revision;
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

pub type MaterialId = u32;
pub const MAX_MATERIALS: usize = 4095;
const MAX_TEXTURE_DIMENSION: u32 = 4096;
const CHANNELS: usize = 4;

/// Immutable shared raster. Pixels are straight RGBA8, including continuous alpha.
#[derive(Clone, Debug)]
pub struct Texture {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
    has_cutout: bool,
    has_partial_alpha: bool,
    revision: u64,
}

impl Texture {
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Result<Self, String> {
        let bytes = texture_bytes(width, height)?;
        if pixels.len() != bytes {
            return Err(format!(
                "texture has {} bytes; {width}×{height} RGBA requires {bytes}",
                pixels.len()
            ));
        }
        let mut has_cutout = false;
        let mut has_partial_alpha = false;
        for pixel in pixels.chunks_exact(CHANNELS) {
            match pixel[3] {
                0 => has_cutout = true,
                255 => {}
                _ => has_partial_alpha = true,
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
            has_cutout,
            has_partial_alpha,
            revision: revision(),
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &Arc<[u8]> {
        &self.pixels
    }
    pub fn has_cutout(&self) -> bool {
        self.has_cutout
    }
    pub fn has_partial_alpha(&self) -> bool {
        self.has_partial_alpha
    }
    pub fn has_alpha(&self) -> bool {
        self.has_cutout || self.has_partial_alpha
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            || (self.width == other.width
                && self.height == other.height
                && self.pixels == other.pixels)
    }
}
impl Eq for Texture {}

impl Serialize for Texture {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.width, &self.height, &self.pixels).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Texture {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EncodedTexture;
        impl<'de> Visitor<'de> for EncodedTexture {
            type Value = Texture;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("[width, height, RGBA8 pixels]")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Texture, A::Error> {
                let width = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("texture width is missing"))?;
                let height = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("texture height is missing"))?;
                let expected = texture_bytes(width, height).map_err(serde::de::Error::custom)?;
                let pixels = seq
                    .next_element_seed(Pixels(expected))?
                    .ok_or_else(|| serde::de::Error::custom("texture pixels are missing"))?;
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("texture has extra fields"));
                }
                Texture::new(width, height, pixels).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_tuple(3, EncodedTexture)
    }
}

struct Pixels(usize);
impl<'de> DeserializeSeed<'de> for Pixels {
    type Value = Arc<[u8]>;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Pixels {
    type Value = Arc<[u8]>;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "exactly {} texture bytes", self.0)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        if seq.size_hint().is_some_and(|length| length != self.0) {
            return Err(serde::de::Error::custom(
                "texture byte count does not match its dimensions",
            ));
        }
        let mut pixels = Vec::new();
        while let Some(pixel) = seq.next_element()? {
            if pixels.len() == self.0 {
                return Err(serde::de::Error::custom(
                    "texture has more pixels than its dimensions permit",
                ));
            }
            pixels.push(pixel);
        }
        if pixels.len() != self.0 {
            return Err(serde::de::Error::custom(
                "texture has fewer pixels than its dimensions require",
            ));
        }
        Ok(pixels.into())
    }
}

/// Validate dimensions before allocating decoded or generated RGBA8 pixels.
pub fn texture_bytes(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIMENSION || height > MAX_TEXTURE_DIMENSION
    {
        return Err(format!(
            "texture dimensions {width}×{height} must each be 1..={MAX_TEXTURE_DIMENSION}"
        ));
    }
    Ok(width as usize * height as usize * CHANNELS)
}

/// Geometry stays inside its occupied voxel. Cards have cutout coverage, never a bulk interior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u32)]
pub enum MaterialGeometry {
    #[default]
    Cube = 0,
    Cross = 1,
}
impl MaterialGeometry {
    pub const ALL: [Self; 2] = [Self::Cube, Self::Cross];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cube => "cube",
            Self::Cross => "cross",
        }
    }
    pub fn is_default(&self) -> bool {
        *self == Self::Cube
    }
}
impl std::str::FromStr for MaterialGeometry {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|geometry| geometry.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown voxel material geometry '{name}'; use {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

/// Homogeneous dielectric properties. Color is linear RGB transmittance at the given cell distance.
/// Texture alpha remains binary cutout coverage, separate from physical transmission.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpticalProperties {
    pub transmission: f32,
    pub ior: f32,
    pub roughness: f32,
    pub attenuation_color: [f32; 3],
    pub attenuation_distance: f32,
    pub wave_strength: f32,
    pub wave_length: f32,
    pub wave_speed: f32,
}
impl Default for OpticalProperties {
    fn default() -> Self {
        Self {
            transmission: 0.,
            ior: 1.5,
            roughness: 0.,
            attenuation_color: [1.; 3],
            attenuation_distance: 1.,
            wave_strength: 0.,
            wave_length: 8.,
            wave_speed: 1.,
        }
    }
}
impl OpticalProperties {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    /// Canonical immutable material identity, shared by palette and language value keys.
    pub fn cache_key(&self) -> [u32; 10] {
        [
            self.transmission,
            self.ior,
            self.roughness,
            self.attenuation_color[0],
            self.attenuation_color[1],
            self.attenuation_color[2],
            self.attenuation_distance,
            self.wave_strength,
            self.wave_length,
            self.wave_speed,
        ]
        .map(f32::to_bits)
    }
}

/// Faces are ordered +X, −X, +Y, −Y, +Z, −Z. Missing textures use the solid color.
#[derive(Clone, Debug, Serialize)]
pub struct Material {
    color: u32,
    faces: [Option<Arc<Texture>>; 6],
    emission: f32,
    #[serde(default, skip_serializing_if = "OpticalProperties::is_default")]
    optics: OpticalProperties,
    #[serde(default, skip_serializing_if = "MaterialGeometry::is_default")]
    geometry: MaterialGeometry,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct Key(u32, u32, [Option<usize>; 6], [u32; 10], u32);

impl Material {
    pub fn solid(color: u32) -> Result<Self, String> {
        Self::textured(color, std::array::from_fn(|_| None))
    }
    pub fn textured(color: u32, faces: [Option<Arc<Texture>>; 6]) -> Result<Self, String> {
        if color & 0xff != 255 {
            return Err("voxel material colors must have opaque alpha (255); use VoxelMaterial(transmission=...) for glass/water transparency, or erase cells using material zero".into());
        }
        for (face, texture) in faces
            .iter()
            .enumerate()
            .filter_map(|(face, texture)| texture.as_ref().map(|texture| (face, texture)))
        {
            if texture.has_partial_alpha() {
                let (index, pixel) = texture
                    .pixels()
                    .chunks_exact(4)
                    .enumerate()
                    .find(|(_, pixel)| pixel[3] != 0 && pixel[3] != 255)
                    .expect("partial alpha metadata");
                return Err(format!(
                    "voxel material face {face} texture pixel [{},{}] has alpha {}; voxel surfaces require opaque or binary cutout coverage. Use texture.cutout() for foliage, or VoxelMaterial(transmission=...) for glass/water",
                    index % texture.width() as usize,
                    index / texture.width() as usize,
                    pixel[3]
                ));
            }
        }
        Ok(Self {
            color,
            faces,
            emission: 0.0,
            optics: OpticalProperties::default(),
            geometry: MaterialGeometry::default(),
        })
    }
    pub fn with_emission(mut self, emission: f32) -> Result<Self, String> {
        if !emission.is_finite() || emission < 0.0 {
            return Err("voxel material emission must be finite and non-negative".into());
        }
        self.emission = if emission == 0.0 { 0.0 } else { emission };
        Ok(self)
    }
    fn closed_medium(
        &self,
        geometry: MaterialGeometry,
        optics: &OpticalProperties,
    ) -> Result<(), String> {
        if optics.transmission > 0. && geometry != MaterialGeometry::Cube {
            return Err("transmissive voxel materials require closed cube geometry; cross cards support cutout coverage only".into());
        }
        if optics.transmission > 0. && self.faces.iter().flatten().any(|face| face.has_cutout()) {
            return Err("transmissive voxel materials require closed opaque texture coverage; use transmission for glass/water transparency instead of cutout holes".into());
        }
        Ok(())
    }
    pub fn with_geometry(mut self, geometry: MaterialGeometry) -> Result<Self, String> {
        self.closed_medium(geometry, &self.optics)?;
        self.geometry = geometry;
        Ok(self)
    }
    pub fn geometry(&self) -> MaterialGeometry {
        self.geometry
    }
    pub fn with_optics(mut self, mut optics: OpticalProperties) -> Result<Self, String> {
        if optics.is_default() {
            self.optics = OpticalProperties::default();
            return Ok(self);
        }
        optics.validate()?;
        self.closed_medium(self.geometry, &optics)?;
        for value in [
            &mut optics.transmission,
            &mut optics.ior,
            &mut optics.roughness,
            &mut optics.attenuation_distance,
            &mut optics.wave_strength,
            &mut optics.wave_length,
            &mut optics.wave_speed,
        ]
        .into_iter()
        .chain(optics.attenuation_color.iter_mut())
        {
            if *value == 0. {
                *value = 0.;
            }
        }
        self.optics = optics;
        Ok(self)
    }
    pub fn optics(&self) -> &OpticalProperties {
        &self.optics
    }
    pub fn emission(&self) -> f32 {
        self.emission
    }
    pub fn color(&self) -> u32 {
        self.color
    }
    pub fn faces(&self) -> &[Option<Arc<Texture>>; 6] {
        &self.faces
    }
    /// Unlit albedo for a nearest-sampled block face; zero denotes a cutout texel.
    pub fn sample(&self, face: usize, uv: [f64; 2]) -> u32 {
        let Some(texture) = &self.faces[face] else {
            return self.color;
        };
        let x = ((uv[0].clamp(0.0, 1.0) * f64::from(texture.width)).floor() as u32)
            .min(texture.width - 1);
        let y = ((uv[1].clamp(0.0, 1.0) * f64::from(texture.height)).floor() as u32)
            .min(texture.height - 1);
        let start = (y as usize * texture.width as usize + x as usize) * CHANNELS;
        let pixel = &texture.pixels[start..start + CHANNELS];
        if pixel[3] == 0 {
            return 0;
        }
        let tint = self.color.to_be_bytes();
        u32::from_be_bytes(std::array::from_fn(|axis| {
            ((u32::from(pixel[axis]) * u32::from(tint[axis]) + 127) / 255) as u8
        }))
    }
    pub(crate) fn key(&self) -> Key {
        Key(
            self.color,
            self.emission.to_bits(),
            self.faces
                .each_ref()
                .map(|face| face.as_ref().map(|texture| Arc::as_ptr(texture) as usize)),
            self.optics.cache_key(),
            self.geometry as u32,
        )
    }
}

impl<'de> Deserialize<'de> for Material {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            color: u32,
            faces: [Option<Arc<Texture>>; 6],
            emission: f32,
            #[serde(default)]
            optics: OpticalProperties,
            #[serde(default)]
            geometry: MaterialGeometry,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::textured(fields.color, fields.faces)
            .and_then(|material| material.with_emission(fields.emission))
            .and_then(|material| material.with_optics(fields.optics))
            .and_then(|material| material.with_geometry(fields.geometry))
            .map_err(serde::de::Error::custom)
    }
}

impl PartialEq for Material {
    fn eq(&self, other: &Self) -> bool {
        self.color == other.color
            && self.emission == other.emission
            && self.faces == other.faces
            && self.optics == other.optics
            && self.geometry == other.geometry
    }
}
impl Eq for Material {}
