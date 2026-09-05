use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

const REVISION: &str = "9b9573cd21b253c9ba58739bbd1aa0b50b991bff";
const BASE_URL: &str = "https://raw.githubusercontent.com/DanielSWolf/rhubarb-lip-sync";
const MAGIC: &[u8; 8] = b"SPHCI004";
const CHECKSUM_LENGTH: usize = 32;
const PHONE_COUNT: usize = 46;
const SENONE_COUNT: usize = 138;
const DENSITY_COUNT: usize = 32;
const FEATURE_DIM: usize = 36;
const LDA_INPUT_DIM: usize = 39;
const LM_WORD_COUNT: usize = 43;
const IMPOSSIBLE: f32 = -1.0e30;
const LOG_BASE: f32 = 1.0001;

struct Source {
    name: &'static str,
    path: &'static str,
    size: u64,
    sha256: &'static str,
}

const SOURCES: [Source; 7] = [
    Source {
        name: "mdef",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/mdef",
        size: 6_992_233,
        sha256: "a100d7401e8d59ed597ea6083e0acca77e41637b7db6db66f43a0799a2eba840",
    },
    Source {
        name: "means",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/means",
        size: 23_675_972,
        sha256: "10c8a3c1b0718bc786f4c82ba1824b22cd9cdc7ffb0589559c08c0189884f0f1",
    },
    Source {
        name: "variances",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/variances",
        size: 23_675_972,
        sha256: "66bc86ddf763cf27d6194247cbf4f2e912d78789ced246811e7f323f1e8280da",
    },
    Source {
        name: "mixture_weights",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/mixture_weights",
        size: 657_728,
        sha256: "a756459b78bfaf85ad59215c77caaf03f9e58e91956975e33de7b8179d551c1f",
    },
    Source {
        name: "transition_matrices",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/transition_matrices",
        size: 2_272,
        sha256: "020e3a8998d12db0d02b620aed95ee1534676fd8e50afaad29d5b432f1e6f893",
    },
    Source {
        name: "feature_transform",
        path: "rhubarb/lib/cmusphinx-en-us-5.2/feature_transform",
        size: 5_660,
        sha256: "05cd0ef213623137b6ac76d72922776c8a14f252e032c4a3f331d41760ef30cc",
    },
    Source {
        name: "phone_lm",
        path: "rhubarb/lib/pocketsphinx-rev13216/model/en-us/en-us-phone.lm.bin",
        size: 857_195,
        sha256: "c57e0fa4191b096b1279cfe3a77927f52568fdecfc6624ddb5cec9527c763a54",
    },
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SHRIMPLY_POCKETSPHINX_CACHE");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let profile = out
        .ancestors()
        .nth(3)
        .expect("standard Cargo OUT_DIR layout");
    let target = profile
        .parent()
        .expect("Cargo profile directory has a parent");
    let cache = env::var_os("SHRIMPLY_POCKETSPHINX_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| target.join("pocketsphinx-cache").join(REVISION));
    fs::create_dir_all(&cache).expect("create PocketSphinx cache");

    for source in &SOURCES {
        fetch(source, &cache.join(source.name));
    }
    let cached_model = cache.join("pocketsphinx-ci-v4.model");
    if !valid_compact_model(&cached_model) {
        let temporary = cache.join(format!(
            "pocketsphinx-ci-v4.model.part-{}",
            std::process::id()
        ));
        convert(&cache, &temporary)
            .unwrap_or_else(|error| panic!("model conversion failed: {error}"));
        fs::rename(&temporary, &cached_model).expect("publish compact model");
    }

    let resource_dir = profile.join("res/lip-sync");
    fs::create_dir_all(&resource_dir).expect("create profile resource directory");
    let staged = resource_dir.join("pocketsphinx-ci.model");
    fs::copy(&cached_model, &staged).expect("stage compact PocketSphinx model");
    println!(
        "cargo:rustc-env=SHRIMPLY_BUILD_POCKETSPHINX_MODEL={}",
        staged.display()
    );
}

fn fetch(source: &Source, destination: &Path) {
    if verified(destination, source) {
        return;
    }
    let url = format!("{BASE_URL}/{REVISION}/{}", source.path);
    println!(
        "cargo:warning=downloading pinned PocketSphinx resource {}",
        source.name
    );
    let response = reqwest::blocking::get(&url)
        .and_then(reqwest::blocking::Response::error_for_status)
        .unwrap_or_else(|error| panic!("download {url}: {error}"));
    let bytes = response
        .bytes()
        .unwrap_or_else(|error| panic!("read {url}: {error}"));
    let temporary = destination.with_extension(format!("part-{}", std::process::id()));
    fs::write(&temporary, &bytes).expect("write downloaded model resource");
    if !verified(&temporary, source) {
        let _ = fs::remove_file(&temporary);
        panic!("downloaded resource failed size or SHA-256 verification: {url}");
    }
    fs::rename(temporary, destination).expect("publish downloaded model resource");
}

fn verified(path: &Path, source: &Source) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.len() as u64 != source.size {
        return false;
    }
    format!("{:x}", Sha256::digest(bytes)) == source.sha256
}

fn valid_compact_model(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| {
        if bytes.len() <= 1_000_000 + CHECKSUM_LENGTH || !bytes.starts_with(MAGIC) {
            return false;
        }
        let (model, checksum) = bytes.split_at(bytes.len() - CHECKSUM_LENGTH);
        Sha256::digest(model).as_slice() == checksum
    })
}

fn convert(cache: &Path, output: &Path) -> Result<(), String> {
    let Mdef {
        states,
        transition_ids,
        labels,
    } = parse_mdef(&fs::read(cache.join("mdef")).map_err(io_error)?)?;
    let means_file = S3::new(fs::read(cache.join("means")).map_err(io_error)?)?;
    let variances_file = S3::new(fs::read(cache.join("variances")).map_err(io_error)?)?;
    let mixtures_file = S3::new(fs::read(cache.join("mixture_weights")).map_err(io_error)?)?;
    let transitions_file = S3::new(fs::read(cache.join("transition_matrices")).map_err(io_error)?)?;
    let lda_file = S3::new(fs::read(cache.join("feature_transform")).map_err(io_error)?)?;
    let means = parse_gaussians(means_file)?;
    let variances = parse_gaussians(variances_file)?;
    let mixtures = parse_mixtures(mixtures_file)?;
    let transitions = parse_transitions(transitions_file)?;
    let lda = parse_lda(lda_file)?;
    let lm = parse_lm(
        &fs::read(cache.join("phone_lm")).map_err(io_error)?,
        &labels,
    )?;

    let vector_count = SENONE_COUNT * DENSITY_COUNT * FEATURE_DIM;
    let mut selected_means = Vec::with_capacity(vector_count);
    let mut inverse_variances = Vec::with_capacity(vector_count);
    let mut determinants = Vec::with_capacity(SENONE_COUNT * DENSITY_COUNT);
    for senone in 0..SENONE_COUNT {
        for density in 0..DENSITY_COUNT {
            let start = (senone * DENSITY_COUNT + density) * FEATURE_DIM;
            let mut determinant = 0.0;
            for dimension in 0..FEATURE_DIM {
                let variance = variances[start + dimension].max(0.0001);
                selected_means.push(means[start + dimension]);
                inverse_variances.push(1.0 / (2.0 * variance));
                determinant += (1.0 / (variance * 2.0 * std::f32::consts::PI).sqrt()).ln();
            }
            determinants.push(determinant);
        }
    }
    let mut mixture_logs = Vec::with_capacity(SENONE_COUNT * DENSITY_COUNT);
    for row in mixtures.chunks_exact(DENSITY_COUNT).take(SENONE_COUNT) {
        let mut normalized = normalize(row, 1.0e-7, false);
        mixture_logs.extend(normalized.drain(..).map(quantized_log));
    }
    let mut transition_logs = Vec::with_capacity(PHONE_COUNT * 12);
    for row in transitions.chunks_exact(4) {
        transition_logs.extend(normalize(row, 0.0001, true).into_iter().map(|value| {
            if value == 0.0 {
                IMPOSSIBLE
            } else {
                quantized_log(value)
            }
        }));
    }

    let mut file = fs::File::create(output).map_err(io_error)?;
    file.write_all(MAGIC).map_err(io_error)?;
    for value in [
        PHONE_COUNT,
        SENONE_COUNT,
        DENSITY_COUNT,
        FEATURE_DIM,
        LDA_INPUT_DIM,
        LM_WORD_COUNT,
    ] {
        write_u32(&mut file, value as u32)?;
    }
    write_u16s(&mut file, &states)?;
    write_u16s(&mut file, &transition_ids)?;
    file.write_all(&lm.phone_words).map_err(io_error)?;
    for values in [
        &selected_means,
        &inverse_variances,
        &determinants,
        &mixture_logs,
        &transition_logs,
        &lda,
        &lm.unigram_probability,
        &lm.unigram_backoff,
        &lm.bigram_probability,
        &lm.bigram_backoff,
        &lm.trigram_probability,
    ] {
        write_f32s(&mut file, values)?;
    }
    file.sync_all().map_err(io_error)?;
    drop(file);
    let bytes = fs::read(output).map_err(io_error)?;
    let checksum = Sha256::digest(&bytes);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(output)
        .map_err(io_error)?;
    file.write_all(&checksum).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn normalize(row: &[f32], floor: f32, preserve_zero: bool) -> Vec<f32> {
    let sum: f32 = row.iter().sum();
    let mut values: Vec<f32> = row
        .iter()
        .map(|source| {
            let value = if sum > 0.0 {
                source / sum
            } else {
                1.0 / row.len() as f32
            };
            if preserve_zero && *source == 0.0 {
                0.0
            } else {
                value.max(floor)
            }
        })
        .collect();
    let sum: f32 = values.iter().sum();
    values.iter_mut().for_each(|value| *value /= sum);
    values
}

fn quantized_log(probability: f32) -> f32 {
    let quantum = 256.0 * LOG_BASE.ln();
    -((-probability.ln() / quantum).round().min(255.0) * quantum)
}

struct Mdef {
    states: Vec<u16>,
    transition_ids: Vec<u16>,
    labels: Vec<String>,
}

fn parse_mdef(bytes: &[u8]) -> Result<Mdef, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| format!("mdef is not UTF-8: {error}"))?;
    let mut records = text.lines().filter_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        (fields.len() >= 10 && fields.last() == Some(&"N")).then_some(fields)
    });
    let mut states = Vec::with_capacity(SENONE_COUNT);
    let mut transition_ids = Vec::with_capacity(PHONE_COUNT);
    let mut labels = Vec::with_capacity(PHONE_COUNT);
    for _ in 0..PHONE_COUNT {
        let fields = records.next().ok_or("mdef has too few CI phones")?;
        labels.push(fields[0].to_owned());
        transition_ids.push(
            fields[5]
                .parse()
                .map_err(|_| "invalid mdef transition ID")?,
        );
        for field in &fields[6..9] {
            states.push(field.parse().map_err(|_| "invalid mdef state ID")?);
        }
    }
    Ok(Mdef {
        states,
        transition_ids,
        labels,
    })
}

struct S3 {
    bytes: Vec<u8>,
    offset: usize,
    swapped: bool,
}

impl S3 {
    fn new(bytes: Vec<u8>) -> Result<Self, String> {
        let marker = b"endhdr\n";
        let header = bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .ok_or("S3 file has no endhdr")?
            + marker.len();
        let magic = bytes.get(header..header + 4).ok_or("truncated S3 magic")?;
        let little = u32::from_le_bytes(magic.try_into().expect("length checked"));
        let swapped = if little == 0x1122_3344 {
            false
        } else if little.swap_bytes() == 0x1122_3344 {
            true
        } else {
            return Err("invalid S3 byte-order magic".to_owned());
        };
        Ok(Self {
            bytes,
            offset: header + 4,
            swapped,
        })
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self
            .bytes
            .get(self.offset..self.offset + 4)
            .ok_or("truncated S3 data")?;
        self.offset += 4;
        let value = u32::from_le_bytes(bytes.try_into().expect("length checked"));
        Ok(if self.swapped {
            value.swap_bytes()
        } else {
            value
        })
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, String> {
        (0..count)
            .map(|_| Ok(f32::from_bits(self.u32()?)))
            .collect()
    }
}

fn parse_gaussians(mut file: S3) -> Result<Vec<f32>, String> {
    let gaussian_count = file.u32()? as usize;
    let streams = file.u32()? as usize;
    let densities = file.u32()? as usize;
    if streams != 1 {
        return Err("acoustic model must have one feature stream".to_owned());
    }
    let dimensions = file.u32()? as usize;
    let count = file.u32()? as usize;
    if gaussian_count < SENONE_COUNT
        || densities != DENSITY_COUNT
        || dimensions != FEATURE_DIM
        || count != gaussian_count * densities * dimensions
    {
        return Err("unexpected Gaussian model dimensions".to_owned());
    }
    file.f32s(count)
}

fn parse_mixtures(mut file: S3) -> Result<Vec<f32>, String> {
    let senones = file.u32()? as usize;
    let streams = file.u32()? as usize;
    let densities = file.u32()? as usize;
    let count = file.u32()? as usize;
    if senones < SENONE_COUNT
        || streams != 1
        || densities != DENSITY_COUNT
        || count != senones * densities
    {
        return Err("unexpected mixture-weight dimensions".to_owned());
    }
    file.f32s(count)
}

fn parse_transitions(mut file: S3) -> Result<Vec<f32>, String> {
    let matrices = file.u32()? as usize;
    let sources = file.u32()? as usize;
    let destinations = file.u32()? as usize;
    let count = file.u32()? as usize;
    if matrices != PHONE_COUNT || sources != 3 || destinations != 4 || count != matrices * 12 {
        return Err("unexpected transition-matrix dimensions".to_owned());
    }
    file.f32s(count)
}

fn parse_lda(mut file: S3) -> Result<Vec<f32>, String> {
    let transforms = file.u32()? as usize;
    let rows = file.u32()? as usize;
    let columns = file.u32()? as usize;
    let count = file.u32()? as usize;
    if transforms != 1 || rows != FEATURE_DIM || columns != LDA_INPUT_DIM || count != rows * columns
    {
        return Err("unexpected feature-transform dimensions".to_owned());
    }
    file.f32s(count)
}

struct LanguageModel {
    phone_words: Vec<u8>,
    unigram_probability: Vec<f32>,
    unigram_backoff: Vec<f32>,
    bigram_probability: Vec<f32>,
    bigram_backoff: Vec<f32>,
    trigram_probability: Vec<f32>,
}

fn parse_lm(bytes: &[u8], phone_labels: &[String]) -> Result<LanguageModel, String> {
    const HEADER: &[u8] = b"Trie Language Model";
    if !bytes.starts_with(HEADER) {
        return Err("invalid phone LM header".to_owned());
    }
    let mut offset = HEADER.len();
    let order = *bytes.get(offset).ok_or("truncated phone LM")? as usize;
    offset += 1;
    if order != 3 {
        return Err("phone LM is not a trigram model".to_owned());
    }
    let counts = [
        read_u32(bytes, &mut offset)?,
        read_u32(bytes, &mut offset)?,
        read_u32(bytes, &mut offset)?,
    ];
    if counts != [43, 1509, 21837] {
        return Err(format!("unexpected phone LM counts: {counts:?}"));
    }
    let dummy = read_u32(bytes, &mut offset)?;
    if dummy != 1 {
        return Err("unsupported phone LM quantizer".to_owned());
    }
    let table_values = 3 * (1 << 16);
    let tables: Vec<f32> = (0..table_values)
        .map(|_| read_u32(bytes, &mut offset).map(f32::from_bits))
        .collect::<Result<_, _>>()?;
    let bigram_probabilities = &tables[..1 << 16];
    let bigram_backoffs = &tables[1 << 16..2 << 16];
    let trigram_probabilities = &tables[2 << 16..];

    let mut unigrams = Vec::with_capacity(LM_WORD_COUNT + 1);
    for _ in 0..=LM_WORD_COUNT {
        let probability = f32::from_bits(read_u32(bytes, &mut offset)?);
        let backoff = f32::from_bits(read_u32(bytes, &mut offset)?);
        let next = read_u32(bytes, &mut offset)? as usize;
        unigrams.push((probability, backoff, next));
    }
    let middle_size = packed_size(counts[1] as usize, 53);
    let longest_size = packed_size(counts[2] as usize, 22);
    let middle = bytes
        .get(offset..offset + middle_size)
        .ok_or("truncated bigram trie")?;
    offset += middle_size;
    let longest = bytes
        .get(offset..offset + longest_size)
        .ok_or("truncated trigram trie")?;
    offset += longest_size;
    let word_bytes = read_u32(bytes, &mut offset)? as usize;
    let words_data = bytes
        .get(offset..offset + word_bytes)
        .ok_or("truncated phone LM vocabulary")?;
    if offset + word_bytes != bytes.len() {
        return Err("phone LM has trailing data".to_owned());
    }
    let words: Vec<String> = words_data
        .split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| {
            String::from_utf8(word.to_vec())
                .map_err(|_| "phone LM vocabulary is not UTF-8".to_owned())
        })
        .collect::<Result<_, _>>()?;
    if words.len() != LM_WORD_COUNT {
        return Err("phone LM vocabulary count mismatch".to_owned());
    }

    let scale = LOG_BASE.ln();
    let convert = |value: f32| {
        if value <= -2.0e9 {
            IMPOSSIBLE
        } else {
            value * scale
        }
    };
    let mut unigram_probability: Vec<f32> = unigrams[..LM_WORD_COUNT]
        .iter()
        .map(|value| convert(value.0))
        .collect();
    let unigram_backoff: Vec<f32> = unigrams[..LM_WORD_COUNT]
        .iter()
        .map(|value| convert(value.1))
        .collect();
    let mut bigram_probability = vec![f32::NAN; LM_WORD_COUNT * LM_WORD_COUNT];
    let mut bigram_backoff = vec![f32::NAN; LM_WORD_COUNT * LM_WORD_COUNT];
    let mut trigram_probability = vec![f32::NAN; LM_WORD_COUNT.pow(3)];
    for first in 0..LM_WORD_COUNT {
        for bigram in unigrams[first].2..unigrams[first + 1].2 {
            let bit = bigram * 53;
            let second = bits(middle, bit, 6) as usize;
            let backoff_bin = bits(middle, bit + 6, 16) as usize;
            let probability_bin = bits(middle, bit + 22, 16) as usize;
            // The Sphinx trie is prediction-first: the unigram root is the
            // predicted word and packed children are its history words.
            let bigram_index = second * LM_WORD_COUNT + first;
            bigram_probability[bigram_index] = convert(bigram_probabilities[probability_bin]);
            bigram_backoff[bigram_index] = convert(bigram_backoffs[backoff_bin]);
            let begin = bits(middle, bit + 38, 15) as usize;
            let end = bits(middle, (bigram + 1) * 53 + 38, 15) as usize;
            for trigram in begin..end {
                let trigram_bit = trigram * 22;
                let third = bits(longest, trigram_bit, 6) as usize;
                let probability_bin = bits(longest, trigram_bit + 6, 16) as usize;
                trigram_probability[(third * LM_WORD_COUNT + second) * LM_WORD_COUNT + first] =
                    convert(trigram_probabilities[probability_bin]);
            }
        }
    }
    // <UNK> has an impossible unigram in this model. It is never selected, but
    // keeping all runtime scores finite simplifies validation and arithmetic.
    unigram_probability.iter_mut().for_each(|value| {
        if *value <= IMPOSSIBLE / 2.0 {
            *value = IMPOSSIBLE;
        }
    });
    let silence = words
        .iter()
        .position(|word| word == "SIL")
        .ok_or("phone LM has no SIL")?;
    let phone_words = phone_labels
        .iter()
        .map(|label| {
            let label = if label.starts_with('+') {
                "SIL"
            } else {
                label.as_str()
            };
            words
                .iter()
                .position(|word| word == label)
                .map(|index| index as u8)
                .ok_or_else(|| format!("phone LM has no word for {label}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if phone_words.len() != PHONE_COUNT || silence >= LM_WORD_COUNT {
        return Err("invalid phone word map".to_owned());
    }
    Ok(LanguageModel {
        phone_words,
        unigram_probability,
        unigram_backoff,
        bigram_probability,
        bigram_backoff,
        trigram_probability,
    })
}

fn packed_size(entries: usize, bits: usize) -> usize {
    ((entries + 1) * bits).div_ceil(8) + 8
}

fn bits(bytes: &[u8], offset: usize, length: usize) -> u64 {
    let start = offset / 8;
    let mut buffer = [0_u8; 8];
    let available = bytes.len().saturating_sub(start).min(8);
    buffer[..available].copy_from_slice(&bytes[start..start + available]);
    (u64::from_le_bytes(buffer) >> (offset % 8)) & ((1_u64 << length) - 1)
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let value = bytes
        .get(*offset..*offset + 4)
        .ok_or("truncated binary data")?;
    *offset += 4;
    Ok(u32::from_le_bytes(
        value.try_into().expect("length checked"),
    ))
}

fn write_u32(file: &mut fs::File, value: u32) -> Result<(), String> {
    file.write_all(&value.to_le_bytes()).map_err(io_error)
}

fn write_u16s(file: &mut fs::File, values: &[u16]) -> Result<(), String> {
    for value in values {
        file.write_all(&value.to_le_bytes()).map_err(io_error)?;
    }
    Ok(())
}

fn write_f32s(file: &mut fs::File, values: &[f32]) -> Result<(), String> {
    for value in values {
        file.write_all(&value.to_le_bytes()).map_err(io_error)?;
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}
