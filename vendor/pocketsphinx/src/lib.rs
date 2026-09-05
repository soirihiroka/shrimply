//! The small, fixed English all-phone recognizer used by Shrimply.
//!
//! This is a clean Rust implementation of the PocketSphinx acoustic path.  It
//! deliberately supports only the 16 kHz CI model staged by this crate's build
//! script; it is not a general PocketSphinx binding.

mod decoder;
mod frontend;
mod model;

use std::path::{Path, PathBuf};

pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Phone {
    Breath,
    Cough,
    Noise,
    Smack,
    UhFiller,
    UmFiller,
    AA,
    AE,
    AH,
    AO,
    AW,
    AY,
    B,
    CH,
    D,
    DH,
    EH,
    ER,
    EY,
    F,
    G,
    HH,
    IH,
    IY,
    JH,
    K,
    L,
    M,
    N,
    NG,
    OW,
    OY,
    P,
    R,
    S,
    SH,
    Silence,
    T,
    TH,
    UH,
    UW,
    V,
    W,
    Y,
    Z,
    ZH,
}

pub(crate) const PHONES: [Phone; 46] = [
    Phone::Breath,
    Phone::Cough,
    Phone::Noise,
    Phone::Smack,
    Phone::UhFiller,
    Phone::UmFiller,
    Phone::AA,
    Phone::AE,
    Phone::AH,
    Phone::AO,
    Phone::AW,
    Phone::AY,
    Phone::B,
    Phone::CH,
    Phone::D,
    Phone::DH,
    Phone::EH,
    Phone::ER,
    Phone::EY,
    Phone::F,
    Phone::G,
    Phone::HH,
    Phone::IH,
    Phone::IY,
    Phone::JH,
    Phone::K,
    Phone::L,
    Phone::M,
    Phone::N,
    Phone::NG,
    Phone::OW,
    Phone::OY,
    Phone::P,
    Phone::R,
    Phone::S,
    Phone::SH,
    Phone::Silence,
    Phone::T,
    Phone::TH,
    Phone::UH,
    Phone::UW,
    Phone::V,
    Phone::W,
    Phone::Y,
    Phone::Z,
    Phone::ZH,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhoneSegment {
    pub start_centiseconds: i64,
    pub end_centiseconds: i64,
    pub phone: Phone,
}

pub struct Model(model::AcousticModel);

impl Model {
    pub fn load(path: &Path) -> Result<Self, String> {
        model::AcousticModel::load(path).map(Self)
    }

    /// Recognize one mono, 16 kHz signed-PCM utterance.
    pub fn decode(&self, samples: &[i16]) -> Result<Vec<PhoneSegment>, String> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }
        let features = frontend::extract(samples, &self.0.lda)?;
        decoder::decode(&self.0, &features)
    }
}

/// Find the build-staged acoustic model.
pub fn model_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHRIMPLY_LIP_SYNC_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "SHRIMPLY_LIP_SYNC_MODEL does not name a file: {}",
            path.display()
        ));
    }

    if let Some(path) = option_env!("SHRIMPLY_BUILD_POCKETSPHINX_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }

    let executable =
        std::env::current_exe().map_err(|error| format!("cannot locate executable: {error}"))?;
    let directory = executable
        .parent()
        .ok_or("executable has no parent directory")?;
    let candidates = [
        directory.join("res/lip-sync/pocketsphinx-ci.model"),
        directory.join("../share/shrimply/lip-sync/pocketsphinx-ci.model"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "PocketSphinx model not found; set SHRIMPLY_LIP_SYNC_MODEL".to_owned())
}
