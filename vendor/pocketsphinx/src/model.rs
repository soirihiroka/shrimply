use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub(crate) const PHONE_COUNT: usize = 46;
pub(crate) const SENONE_COUNT: usize = 138;
pub(crate) const DENSITY_COUNT: usize = 32;
pub(crate) const FEATURE_DIM: usize = 36;
pub(crate) const LDA_INPUT_DIM: usize = 39;
pub(crate) const LM_WORD_COUNT: usize = 43;

const MAGIC: &[u8; 8] = b"SPHCI004";
const CHECKSUM_LENGTH: usize = 32;

pub(crate) struct AcousticModel {
    pub states: Vec<u16>,
    pub transition_ids: Vec<u16>,
    pub lm_words: Vec<u8>,
    pub means: Vec<f32>,
    pub inverse_variances: Vec<f32>,
    pub determinants: Vec<f32>,
    pub mixture_weights: Vec<f32>,
    pub transitions: Vec<f32>,
    pub lda: Vec<f32>,
    pub unigram_probability: Vec<f32>,
    pub unigram_backoff: Vec<f32>,
    pub bigram_probability: Vec<f32>,
    pub bigram_backoff: Vec<f32>,
    pub trigram_probability: Vec<f32>,
}

impl AcousticModel {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|error| format!("cannot read model {}: {error}", path.display()))?;
        if bytes.len() < MAGIC.len() + CHECKSUM_LENGTH {
            return Err("truncated compact model".to_owned());
        }
        let (bytes, checksum) = bytes.split_at(bytes.len() - CHECKSUM_LENGTH);
        if Sha256::digest(bytes).as_slice() != checksum {
            return Err("compact model checksum mismatch".to_owned());
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.take(8)? != MAGIC {
            return Err("unsupported PocketSphinx compact model version".to_owned());
        }
        for (actual, expected, name) in [
            (reader.u32()? as usize, PHONE_COUNT, "phones"),
            (reader.u32()? as usize, SENONE_COUNT, "senones"),
            (reader.u32()? as usize, DENSITY_COUNT, "densities"),
            (reader.u32()? as usize, FEATURE_DIM, "feature dimension"),
            (reader.u32()? as usize, LDA_INPUT_DIM, "LDA input dimension"),
            (reader.u32()? as usize, LM_WORD_COUNT, "LM words"),
        ] {
            if actual != expected {
                return Err(format!("invalid {name}: expected {expected}, got {actual}"));
            }
        }

        let model = Self {
            states: reader.u16s(PHONE_COUNT * 3)?,
            transition_ids: reader.u16s(PHONE_COUNT)?,
            lm_words: reader.take(PHONE_COUNT)?.to_vec(),
            means: reader.f32s(SENONE_COUNT * DENSITY_COUNT * FEATURE_DIM)?,
            inverse_variances: reader.f32s(SENONE_COUNT * DENSITY_COUNT * FEATURE_DIM)?,
            determinants: reader.f32s(SENONE_COUNT * DENSITY_COUNT)?,
            mixture_weights: reader.f32s(SENONE_COUNT * DENSITY_COUNT)?,
            transitions: reader.f32s(PHONE_COUNT * 3 * 4)?,
            lda: reader.f32s(FEATURE_DIM * LDA_INPUT_DIM)?,
            unigram_probability: reader.f32s(LM_WORD_COUNT)?,
            unigram_backoff: reader.f32s(LM_WORD_COUNT)?,
            bigram_probability: reader.f32s(LM_WORD_COUNT * LM_WORD_COUNT)?,
            bigram_backoff: reader.f32s(LM_WORD_COUNT * LM_WORD_COUNT)?,
            trigram_probability: reader.f32s(LM_WORD_COUNT.pow(3))?,
        };
        if reader.offset != bytes.len() {
            return Err(format!(
                "model has {} trailing bytes",
                bytes.len() - reader.offset
            ));
        }
        if model
            .states
            .iter()
            .any(|&state| state as usize >= SENONE_COUNT)
            || model
                .transition_ids
                .iter()
                .any(|&id| id as usize >= PHONE_COUNT)
            || model
                .lm_words
                .iter()
                .any(|&word| word as usize >= LM_WORD_COUNT)
        {
            return Err("model contains an out-of-range index".to_owned());
        }
        Ok(model)
    }

    pub(crate) fn language_score(&self, previous: Option<u8>, current: u8, next: u8) -> f32 {
        let count = LM_WORD_COUNT;
        if let Some(previous) = previous {
            let index = (previous as usize * count + current as usize) * count + next as usize;
            let probability = self.trigram_probability[index];
            if probability.is_finite() {
                return probability;
            }
            let history = previous as usize * count + current as usize;
            let backoff = self.bigram_backoff[history];
            let backoff = if backoff.is_finite() { backoff } else { 0.0 };
            return backoff + self.bigram_score(current, next);
        }
        self.bigram_score(current, next)
    }

    fn bigram_score(&self, current: u8, next: u8) -> f32 {
        let index = current as usize * LM_WORD_COUNT + next as usize;
        let probability = self.bigram_probability[index];
        if probability.is_finite() {
            probability
        } else {
            let backoff = self.unigram_backoff[current as usize];
            let backoff = if backoff.is_finite() { backoff } else { 0.0 };
            backoff + self.unigram_probability[next as usize]
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or("model size overflow")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or("truncated compact model")?;
        self.offset = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("length checked"),
        ))
    }

    fn u16s(&mut self, count: usize) -> Result<Vec<u16>, String> {
        self.take(count * 2)?
            .chunks_exact(2)
            .map(|bytes| {
                Ok(u16::from_le_bytes(
                    bytes.try_into().expect("length checked"),
                ))
            })
            .collect()
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, String> {
        self.take(count * 4)?
            .chunks_exact(4)
            .map(|bytes| {
                let value = f32::from_le_bytes(bytes.try_into().expect("length checked"));
                if value.is_infinite() {
                    Err("model contains an infinite float".to_owned())
                } else {
                    Ok(value)
                }
            })
            .collect()
    }
}
