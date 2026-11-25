use std::path::Path;

use super::{Project, conversion};

const SHRIMP_MAGIC: &[u8; 8] = b"SHRIMP\0\x01";
const SHRIMP_VERSION: u32 = 7;
const HEADER_LEN: usize = SHRIMP_MAGIC.len() + size_of::<u32>();

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectEncoding {
    Shrimp,
    Json,
}

impl ProjectEncoding {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "shrimp" => Some(Self::Shrimp),
            "sjson" | "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// Encode the current project schema using the selected file format.
    /// The caller supplies the asset paths appropriate to the destination.
    pub fn encode(self, project: &Project) -> Result<Vec<u8>, String> {
        match self {
            Self::Json => serde_json::to_vec_pretty(project)
                .map_err(|error| format!("could not serialize project JSON: {error}")),
            Self::Shrimp => {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(SHRIMP_MAGIC);
                bytes.extend_from_slice(&SHRIMP_VERSION.to_le_bytes());
                rmp_serde::encode::write_named(&mut bytes, project)
                    .map_err(|error| format!("could not encode MessagePack project: {error}"))?;
                Ok(bytes)
            }
        }
    }

    /// Decode the container and migrate supported payloads to the current schema.
    pub fn decode(self, bytes: &[u8]) -> Result<Project, String> {
        match self {
            Self::Json => conversion::from_json(
                std::str::from_utf8(bytes)
                    .map_err(|error| format!("project JSON is not UTF-8: {error}"))?,
            ),
            Self::Shrimp => {
                let header = bytes
                    .get(..HEADER_LEN)
                    .filter(|header| header.starts_with(SHRIMP_MAGIC))
                    .ok_or("not a supported .shrimp project file")?;
                let version = u32::from_le_bytes(
                    header[SHRIMP_MAGIC.len()..]
                        .try_into()
                        .expect("container version has a fixed length"),
                );
                if version != SHRIMP_VERSION {
                    return Err(format!(
                        "unsupported .shrimp container version {version}; expected {SHRIMP_VERSION}"
                    ));
                }
                conversion::from_messagepack(&bytes[HEADER_LEN..])
            }
        }
    }
}
