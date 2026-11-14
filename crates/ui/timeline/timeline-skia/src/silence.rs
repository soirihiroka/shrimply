use shrimply_math_core::{fraction_ceil_i64, fraction_floor_i64, fraction_from_integer};
use shrimply_player_core::{self as player_state, ProjectChange};

use crate::{
    TrackKey,
    audio::waveform::{self, Waveform, WaveformMap},
    items::{ItemKey, TrackKind, ripple_remove_time_ranges},
    project::{AudioItem, Project, Time},
    scene::{Scene, selected_timeline_items, selected_timeline_tracks},
};

const MIN_DB: f64 = -100.0;

#[derive(Clone, Copy)]
pub struct Config {
    pub threshold_db: f64,
    pub min_silence: Time,
    pub gap_tolerance: Time,
    pub padding: Time,
    pub delete_chunks: Time,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            threshold_db: -30.0,
            min_silence: Time::from_fraction(1, 5),
            gap_tolerance: Time::from_fraction(2, 25),
            padding: Time::from_fraction(1, 5),
            delete_chunks: Time::ZERO,
        }
    }
}

impl Config {
    fn validate(self) -> Result<Self, String> {
        let within =
            |value: Time, maximum: i64| value >= Time::ZERO && value <= Time::from_seconds(maximum);
        if !self.threshold_db.is_finite() || !(-90.0..=0.0).contains(&self.threshold_db) {
            return Err("Silence threshold must be between -90 and 0 dB".into());
        }
        if !within(self.min_silence, 10) {
            return Err("Minimum silence must be between 0 and 10 seconds".into());
        }
        if !within(self.gap_tolerance, 5) {
            return Err("Gap tolerance must be between 0 and 5 seconds".into());
        }
        if !within(self.padding, 2) {
            return Err("Padding must be between 0 and 2 seconds".into());
        }
        if !within(self.delete_chunks, 10) {
            return Err("Minimum chunk must be between 0 and 10 seconds".into());
        }
        Ok(self)
    }
}

#[derive(Clone)]
struct SelectedAudio {
    item: AudioItem,
    waveform: Waveform,
}

pub fn can_remove(project: &Project, selected_tracks: &[TrackKey], hit: ItemKey) -> bool {
    if hit.kind != TrackKind::Audio {
        return false;
    }
    let hit_track = TrackKey {
        kind: TrackKind::Audio,
        track_index: hit.track_index,
    };
    let tracks = selected_audio_tracks(project, selected_tracks);
    tracks.is_empty() || tracks.contains(&hit_track)
}

pub fn can_remove_track(project: &Project, selected_tracks: &[TrackKey], key: TrackKey) -> bool {
    if key.kind != TrackKind::Audio {
        return false;
    }
    if !selected_tracks.is_empty() && selected_tracks.contains(&key) {
        return !selected_audio_tracks(project, selected_tracks).is_empty();
    }
    project
        .audio_tracks
        .get(key.track_index)
        .is_some_and(|track| !track.items.is_empty())
}

impl Scene {
    pub fn remove_silences(&mut self, config: Config) -> Result<usize, String> {
        let config = config.validate()?;
        let ranges = {
            let project = self.project.borrow();
            detect_ranges(
                &project,
                &self.waveforms,
                &selected_timeline_items(&self.selection),
                &selected_timeline_tracks(&self.selection),
                crate::waveform_chunks_per_second_from_frame_step(crate::frame_step_seconds(
                    &project,
                )),
                config,
            )?
        };
        if ranges.is_empty() {
            return Ok(0);
        }

        let position = player_state::snapshot(&self.player).position;
        let mut candidate = self.project.borrow().clone();
        let Some((captions, video, audio)) = ripple_remove_time_ranges(&mut candidate, &ranges)
        else {
            return Ok(0);
        };
        candidate.normalize_clip_transitions();
        crate::project::commit_edit_checked(&candidate, "remove-silences")?;
        let duration = candidate.duration();
        *self.project.borrow_mut() = candidate;
        crate::selection_state::set_selected_items(&self.selection, Vec::new(), None);
        player_state::refresh_project(
            &self.player,
            ProjectChange {
                duration: Some(duration),
                audio,
                audio_waveforms: audio,
                video,
                captions,
                ..ProjectChange::default()
            },
        );
        player_state::seek_time(&self.player, shifted_position(position, &ranges));
        Ok(ranges.len())
    }
}

fn detect_ranges(
    project: &Project,
    waveforms: &WaveformMap,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
    chunks_per_second: u32,
    config: Config,
) -> Result<Vec<(Time, Time)>, String> {
    if chunks_per_second == 0 {
        return Err("Audio waveform resolution is not available yet".into());
    }
    let selected = collect_selected_audio(project, waveforms, selected_items, selected_tracks)?;
    let range_start = selected
        .iter()
        .map(|audio| audio.item.start)
        .min()
        .ok_or("Select at least one audio item first")?;
    let range_end = selected
        .iter()
        .map(|audio| audio.item.end)
        .max()
        .ok_or("Select at least one audio item first")?;
    if range_end <= range_start {
        return Ok(Vec::new());
    }

    let chunks = fraction_from_integer(i64::from(chunks_per_second));
    let first_bin = fraction_floor_i64(range_start.seconds * chunks)
        .and_then(|bin| usize::try_from(bin.max(0)).ok())
        .ok_or("silence-analysis range is too large")?;
    let last_bin = fraction_ceil_i64(range_end.seconds * chunks)
        .and_then(|bin| usize::try_from(bin.max(0)).ok())
        .ok_or("silence-analysis range is too large")?
        .max(first_bin);
    let threshold = db_to_amplitude(config.threshold_db);
    let mut audible = Vec::new();
    let mut audible_start = None;
    for bin in first_bin..last_bin {
        let center = Time::from_fraction(
            i64::try_from(bin)
                .ok()
                .and_then(|bin| bin.checked_mul(2))
                .and_then(|bin| bin.checked_add(1))
                .ok_or("silence-analysis range is too large")?,
            i64::from(chunks_per_second) * 2,
        );
        let is_audible = mixed_peak_at(&selected, center, chunks_per_second) >= threshold;
        match (audible_start, is_audible) {
            (None, true) => audible_start = Some(bin_start(bin, chunks_per_second)?),
            (Some(start), false) => {
                audible.push((start, bin_start(bin, chunks_per_second)?));
                audible_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = audible_start {
        audible.push((start, range_end));
    }

    let audible = merge_short_gaps(audible, config.gap_tolerance.max(Time::ZERO));
    let audible = ignore_short_chunks(audible, config.delete_chunks.max(Time::ZERO));
    Ok(silence_gaps(
        range_start,
        range_end,
        &audible,
        config.min_silence.max(Time::ZERO),
        config.padding.max(Time::ZERO),
        project.frame_step(),
    ))
}

fn collect_selected_audio(
    project: &Project,
    waveforms: &WaveformMap,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
) -> Result<Vec<SelectedAudio>, String> {
    let mut selected = Vec::new();
    for key in analysis_audio_items(project, selected_items, selected_tracks) {
        let item = project.audio_tracks[key.track_index].items[key.item_index].clone();
        let waveform_key = waveform::audio_key(&item);
        let Some(waveform) = waveforms.get(&waveform_key) else {
            return Err(format!(
                "Waveform is still loading for {}",
                item.file.display()
            ));
        };
        let Some(waveform) = waveform.clone() else {
            return Err(format!("Could not analyze {}", item.file.display()));
        };
        if waveform.has_pending() {
            return Err(format!(
                "Waveform is still loading for {}",
                item.file.display()
            ));
        }
        selected.push(SelectedAudio { item, waveform });
    }
    if selected.is_empty() {
        return Err("Select at least one audio item first".into());
    }
    Ok(selected)
}

fn analysis_audio_items(
    project: &Project,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
) -> Vec<ItemKey> {
    if !selected_audio_tracks(project, selected_tracks).is_empty() {
        return selected_tracks
            .iter()
            .copied()
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| {
                project
                    .audio_tracks
                    .get(track.track_index)
                    .map(|audio_track| {
                        (0..audio_track.items.len()).map(move |item_index| ItemKey {
                            kind: TrackKind::Audio,
                            track_index: track.track_index,
                            item_index,
                        })
                    })
                    .into_iter()
                    .flatten()
            })
            .collect();
    }
    selected_items
        .iter()
        .copied()
        .filter(|key| {
            key.kind == TrackKind::Audio
                && project
                    .audio_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .is_some()
        })
        .collect()
}

fn selected_audio_tracks(project: &Project, selected_tracks: &[TrackKey]) -> Vec<TrackKey> {
    selected_tracks
        .iter()
        .copied()
        .filter(|track| {
            track.kind == TrackKind::Audio
                && project
                    .audio_tracks
                    .get(track.track_index)
                    .is_some_and(|track| !track.items.is_empty())
        })
        .collect()
}

fn mixed_peak_at(selected: &[SelectedAudio], time: Time, chunks_per_second: u32) -> f64 {
    let chunks = fraction_from_integer(i64::from(chunks_per_second));
    selected
        .iter()
        .filter_map(|audio| {
            if time < audio.item.start || time >= audio.item.end {
                return None;
            }
            let local = time.saturating_sub(audio.item.start);
            let index = fraction_floor_i64(local.seconds * chunks)
                .and_then(|index| usize::try_from(index).ok())?;
            Some(f64::from(audio.waveform.peak(index)?) / f64::from(u8::MAX))
        })
        .sum()
}

fn merge_short_gaps(mut ranges: Vec<(Time, Time)>, tolerance: Time) -> Vec<(Time, Time)> {
    ranges.sort_unstable();
    let mut merged: Vec<(Time, Time)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0.saturating_sub(last.1) <= tolerance
        {
            last.1 = last.1.max(range.1);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn ignore_short_chunks(ranges: Vec<(Time, Time)>, threshold: Time) -> Vec<(Time, Time)> {
    ranges
        .into_iter()
        .filter(|(start, end)| end.saturating_sub(*start) >= threshold)
        .collect()
}

fn silence_gaps(
    start: Time,
    end: Time,
    audible: &[(Time, Time)],
    min_silence: Time,
    padding: Time,
    frame_step: Time,
) -> Vec<(Time, Time)> {
    let mut ranges = Vec::new();
    let mut cursor = start;
    for &(audible_start, audible_end) in audible {
        push_gap(
            &mut ranges,
            cursor,
            audible_start.saturating_sub(padding).max(cursor),
            min_silence,
            frame_step,
        );
        cursor = audible_end.saturating_add(padding).min(end).max(cursor);
    }
    push_gap(&mut ranges, cursor, end, min_silence, frame_step);
    ranges
}

fn push_gap(
    ranges: &mut Vec<(Time, Time)>,
    start: Time,
    end: Time,
    min_silence: Time,
    frame_step: Time,
) {
    if end.saturating_sub(start) >= min_silence {
        let start = start.snapped(frame_step);
        let end = end.snapped(frame_step);
        if end > start {
            ranges.push((start, end));
        }
    }
}

fn shifted_position(position: Time, ranges: &[(Time, Time)]) -> Time {
    let mut shift = Time::ZERO;
    for (start, end) in ranges {
        if position <= *start {
            break;
        }
        shift = shift.saturating_add(position.min(*end).saturating_sub(*start));
        if position < *end {
            break;
        }
    }
    position.saturating_sub(shift)
}

fn db_to_amplitude(db: f64) -> f64 {
    if db <= MIN_DB {
        0.0
    } else {
        10.0_f64.powf(db / 20.0)
    }
}

fn bin_start(bin: usize, chunks_per_second: u32) -> Result<Time, String> {
    Ok(Time::from_fraction(
        i64::try_from(bin).map_err(|_| "silence-analysis range is too large")?,
        i64::from(chunks_per_second),
    ))
}
