use shrimply_asset::AssetSnapshot;
use shrimply_audio_modifiers::{
    AudioModifierEffect, PNEUMA_MAX_PITCH_OFFSET, PNEUMA_MAX_SPEED, PNEUMA_MIN_PITCH_OFFSET,
    PNEUMA_MIN_SPEED,
};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::AudioItem;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

const CACHE_VERSION: u32 = 1;
const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8787";

static SERVER_URL: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new(DEFAULT_SERVER_URL.to_string()));

pub fn set_server_url(url: &str) {
    let url = url.trim();
    let mut current = SERVER_URL.write().expect("Pneuma server URL lock poisoned");
    if *current != url {
        *current = url.to_string();
    }
}

pub fn server_url() -> String {
    SERVER_URL
        .read()
        .expect("Pneuma server URL lock poisoned")
        .clone()
}

pub(crate) fn source(
    item: &AudioItem,
    original: &AssetSnapshot,
) -> Result<Option<PathBuf>, String> {
    let voice_changes = item
        .modifiers
        .iter()
        .filter(|modifier| modifier.enabled)
        .filter_map(|modifier| match &modifier.effect {
            AudioModifierEffect::VoiceChange(value) => Some(value.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if voice_changes.is_empty() {
        return Ok(None);
    }

    fs::create_dir_all(project_cache_directory().join("pneuma"))
        .map_err(|error| format!("Could not create Pneuma cache: {error}"))?;
    let server_url = server_url();
    let mut input = original.path().to_path_buf();
    let mut cumulative = Vec::new();
    for voice_change in voice_changes {
        cumulative.push(voice_change);
        let output = cache_path(original, item.track_id, &server_url, &cumulative)?;
        if output.is_file() {
            shrimply_profiling::increment("Pneuma cache / Hit");
            input = output;
            continue;
        }
        shrimply_profiling::increment("Pneuma cache / Miss");
        let cancellation = shrimply_compute_client::CancellationToken::new(&server_url)?;
        let converted = shrimply_compute_client::pneuma::convert(
            shrimply_compute_client::pneuma::ConvertRequest {
                model: &voice_change.model,
                input: &input,
                pitch_offset: voice_change
                    .pitch_offset
                    .clamp(PNEUMA_MIN_PITCH_OFFSET, PNEUMA_MAX_PITCH_OFFSET),
                f0_method: voice_change.f0_method.as_str(),
                speed: voice_change.speed.clamp(PNEUMA_MIN_SPEED, PNEUMA_MAX_SPEED),
                maintain_pitch: voice_change.maintain_pitch,
            },
            &server_url,
            &cancellation,
            || original.is_current(),
        )?;
        let mut downloaded = tempfile::Builder::new()
            .prefix("shrimply-pneuma-")
            .suffix(".audio")
            .tempfile()
            .map_err(|error| format!("Could not create Pneuma download: {error}"))?;
        downloaded
            .write_all(&converted)
            .map_err(|error| format!("Could not store Pneuma download: {error}"))?;
        original.ensure_current()?;
        let temporary = temporary_cache_path(&output);
        let result = super::opus_cache::transcode(downloaded.path(), &temporary);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        original.ensure_current()?;
        fs::rename(&temporary, &output)
            .map_err(|error| format!("Could not finish Pneuma cache file: {error}"))?;
        input = output;
    }
    Ok(Some(input))
}

fn cache_path(
    original: &AssetSnapshot,
    track_id: u32,
    server_url: &str,
    voice_changes: &[&shrimply_audio_modifiers::VoiceChangeModifier],
) -> Result<PathBuf, String> {
    let settings = serde_json::to_vec(voice_changes)
        .map_err(|error| format!("Could not serialize Pneuma settings: {error}"))?;
    let mut hasher = DefaultHasher::new();
    CACHE_VERSION.hash(&mut hasher);
    original.cache_key().hash(&mut hasher);
    track_id.hash(&mut hasher);
    server_url.hash(&mut hasher);
    settings.hash(&mut hasher);
    Ok(project_cache_directory()
        .join("pneuma")
        .join(format!("{:016x}.opus", hasher.finish())))
}

fn temporary_cache_path(output: &Path) -> PathBuf {
    output.with_file_name(format!(
        ".{}.{}.opus",
        output
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("pneuma"),
        uuid::Uuid::new_v4().simple()
    ))
}
