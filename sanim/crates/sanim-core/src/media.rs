//! Immutable encoded media or shared raster pixels; native objects live in the renderer cache.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Bitmap,
    Svg,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Encoded(Arc<Vec<u8>>),
    Texture(Arc<sanim_voxel::Texture>),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Asset {
    pub kind: Kind,
    source: Source,
    #[serde(skip)]
    fingerprint: String,
    pub width: u32,
    pub height: u32,
}
impl Asset {
    pub fn new(kind: Kind, bytes: Arc<Vec<u8>>, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("media intrinsic dimensions must be positive".into());
        }
        let fingerprint = crate::digest(&bytes);
        Ok(Self {
            kind,
            source: Source::Encoded(bytes),
            fingerprint,
            width,
            height,
        })
    }
    pub fn from_texture(texture: Arc<sanim_voxel::Texture>) -> Self {
        Self {
            kind: Kind::Bitmap,
            width: texture.width(),
            height: texture.height(),
            fingerprint: format!("texture:{}", texture.revision()),
            source: Source::Texture(texture),
        }
    }
    pub fn source(&self) -> &Source {
        &self.source
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}
impl<'de> Deserialize<'de> for Asset {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Source {
            kind: Kind,
            source: super::media::Source,
            width: u32,
            height: u32,
        }
        let source = Source::deserialize(deserializer)?;
        match source.source {
            super::media::Source::Encoded(bytes) => {
                Self::new(source.kind, bytes, source.width, source.height)
                    .map_err(serde::de::Error::custom)
            }
            super::media::Source::Texture(texture) => {
                if source.kind != Kind::Bitmap
                    || [source.width, source.height] != [texture.width(), texture.height()]
                {
                    return Err(serde::de::Error::custom(
                        "texture media dimensions or kind disagree with its pixel source",
                    ));
                }
                Ok(Self::from_texture(texture))
            }
        }
    }
}
