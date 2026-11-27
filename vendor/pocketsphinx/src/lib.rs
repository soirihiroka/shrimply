//! The small, fixed English all-phone recognizer used by Shrimply.
//!
//! This is a clean Rust implementation of the PocketSphinx acoustic path.  It
//! deliberately supports only the embedded 16 kHz CI model format.

mod decoder;
mod frontend;
mod model;

use std::path::Path;

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
    pub fn load_default() -> Result<Self, String> {
        if let Some(path) = std::env::var_os("SHRIMPLY_LIP_SYNC_MODEL") {
            return Self::load(Path::new(&path));
        }
        model::AcousticModel::parse(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/pocketsphinx-ci.model"
        )))
        .map(Self)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read model {}: {error}", path.display()))?;
        model::AcousticModel::parse(&bytes).map(Self)
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
