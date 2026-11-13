//! Immutable encoded media; native decoder objects belong to the shared renderer cache.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Bitmap,
    Svg,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Asset {
    pub kind: Kind,
    bytes: Arc<Vec<u8>>,
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
            bytes,
            fingerprint,
            width,
            height,
        })
    }
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
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
            bytes: Arc<Vec<u8>>,
            width: u32,
            height: u32,
        }
        let source = Source::deserialize(deserializer)?;
        Self::new(source.kind, source.bytes, source.width, source.height)
            .map_err(serde::de::Error::custom)
    }
}
