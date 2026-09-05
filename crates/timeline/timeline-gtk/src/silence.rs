use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nAlertDialogExt;
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::audio::waveform::{self, Waveform, WaveformMap};
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::project::{AudioItem, Project, Time, playback_speed_is_zero};
use crate::selection_state::{self, SharedSelectionState};

use super::items::{ItemKey, TrackKind, ripple_remove_time_ranges};
use super::{
    TimelineRuntime, TrackKey, frame_step_seconds, selected_timeline_items,
    selected_timeline_tracks, waveform_chunks_per_second_from_frame_step,
};

const MIN_DB: f64 = -100.0;
#[derive(Clone, Copy)]
pub(super) struct RemoveSilenceConfig {
    pub(super) threshold_db: f64,
    pub(super) min_silence: Time,
    pub(super) gap_tolerance: Time,
    pub(super) padding: Time,
    pub(super) delete_chunks: Time,
}

impl Default for RemoveSilenceConfig {
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

#[derive(Clone)]
struct SelectedAudio {
    item: AudioItem,
    waveform: Waveform,
}

pub(super) fn selected_audio_items(project: &Project, selected_items: &[ItemKey]) -> Vec<ItemKey> {
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

pub(super) fn can_remove(
    project: &Project,
    _selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
    hit: ItemKey,
) -> bool {
    if hit.kind != TrackKind::Audio {
        return false;
    }

    let hit_track = TrackKey {
        kind: TrackKind::Audio,
        track_index: hit.track_index,
    };
    let audio_tracks = selected_audio_tracks(project, selected_tracks);
    if !audio_tracks.is_empty() {
        return audio_tracks.contains(&hit_track);
    }

    true
}

pub(super) fn can_remove_track(
    project: &Project,
    selected_tracks: &[TrackKey],
    key: TrackKey,
) -> bool {
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

pub(super) fn show_dialog(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let defaults = RemoveSilenceConfig::default();
    let threshold = adw::SpinRow::with_range(-90.0, 0.0, 1.0);
    threshold.set_title(tr!("Silence threshold").as_ref());
    threshold.set_value(defaults.threshold_db);
    threshold.set_digits(1);

    let min_silence = adw::SpinRow::with_range(0.0, 10.0, 0.05);
    min_silence.set_title(tr!("Minimum silence").as_ref());
    min_silence.set_value(defaults.min_silence.as_secs_f64());
    min_silence.set_digits(2);

    let gap_tolerance = adw::SpinRow::with_range(0.0, 5.0, 0.01);
    gap_tolerance.set_title(tr!("Gap tolerance").as_ref());
    gap_tolerance.set_value(defaults.gap_tolerance.as_secs_f64());
    gap_tolerance.set_digits(2);

    let padding = adw::SpinRow::with_range(0.0, 2.0, 0.01);
    padding.set_title(tr!("Padding").as_ref());
    padding.set_value(defaults.padding.as_secs_f64());
    padding.set_digits(2);

    let delete_chunks = adw::SpinRow::with_range(0.0, 10.0, 0.05);
    delete_chunks.set_title(tr!("Min chunk").as_ref());
    delete_chunks.set_value(defaults.delete_chunks.as_secs_f64());
    delete_chunks.set_digits(2);

    let group = adw::PreferencesGroup::new();
    group.add(&threshold);
    group.add(&min_silence);
    group.add(&gap_tolerance);
    group.add(&padding);
    group.add(&delete_chunks);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    content.append(&group);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Remove Silences").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("remove", "Remove Silences")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("remove"));
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    let runtime = runtime.clone();
    let parent = area.clone();
    dialog.choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() != "remove" {
                return;
            }
            let config = RemoveSilenceConfig {
                threshold_db: threshold.value(),
                min_silence: Time::from_seconds_f64(min_silence.value()),
                gap_tolerance: Time::from_seconds_f64(gap_tolerance.value()),
                padding: Time::from_seconds_f64(padding.value()),
                delete_chunks: Time::from_seconds_f64(delete_chunks.value()),
            };
            remove_silences(
                &area,
                &project,
                &player_state,
                &selection_state,
                &runtime,
                config,
            );
        },
    );
}

pub(super) fn detect_ranges(
    project: &Project,
    waveforms: &WaveformMap,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
    waveform_chunks_per_second: u32,
    config: RemoveSilenceConfig,
) -> Result<Vec<(Time, Time)>, String> {
    if waveform_chunks_per_second == 0 {
        return Err("Audio waveform resolution is not available yet".to_string());
    }

    let selected = collect_selected_audio(project, waveforms, selected_items, selected_tracks)?;
    let Some(range_start) = selected.iter().map(|audio| audio.item.start).min() else {
        return Err("Select at least one audio item first".to_string());
    };
    let Some(range_end) = selected.iter().map(|audio| audio.item.end).max() else {
        return Err("Select at least one audio item first".to_string());
    };
    if range_end <= range_start {
        return Ok(Vec::new());
    }

    let threshold = db_to_amplitude(config.threshold_db);
    let first_bin = (range_start.as_secs_f64() * f64::from(waveform_chunks_per_second))
        .floor()
        .max(0.0) as usize;
    let last_bin = (range_end.as_secs_f64() * f64::from(waveform_chunks_per_second))
        .ceil()
        .max(first_bin as f64) as usize;
    let mut audible = Vec::new();
    let mut audible_start = None;

    for bin in first_bin..last_bin {
        let time = (bin as f64 + 0.5) / f64::from(waveform_chunks_per_second);
        let is_audible = mixed_peak_at(&selected, time, waveform_chunks_per_second) >= threshold;
        match (audible_start, is_audible) {
            (None, true) => audible_start = Some(bin_start(bin, waveform_chunks_per_second)),
            (Some(start), false) => {
                audible.push((start, bin_start(bin, waveform_chunks_per_second)));
                audible_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = audible_start {
        audible.push((start, range_end.as_secs_f64()));
    }

    let audible = merge_short_gaps(audible, config.gap_tolerance.as_secs_f64().max(0.0));
    let audible = ignore_short_chunks(audible, config.delete_chunks.as_secs_f64().max(0.0));
    Ok(silence_gaps(
        range_start.as_secs_f64(),
        range_end.as_secs_f64(),
        &audible,
        config.min_silence.as_secs_f64().max(0.0),
        config.padding.as_secs_f64().max(0.0),
        project.frame_step(),
    ))
}

pub(super) fn apply_ranges(
    project: &mut Project,
    ranges: &[(Time, Time)],
) -> Option<(bool, bool, bool)> {
    ripple_remove_time_ranges(project, ranges)
}

pub(super) fn shifted_position(position: Time, ranges: &[(Time, Time)]) -> Time {
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

fn remove_silences(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    config: RemoveSilenceConfig,
) {
    let result = {
        let project_state = project.borrow();
        let runtime_state = runtime.borrow();
        let waveform_chunks_per_second =
            waveform_chunks_per_second_from_frame_step(frame_step_seconds(&project_state));
        detect_ranges(
            &project_state,
            &runtime_state.waveforms,
            &selected_timeline_items(selection_state),
            &selected_timeline_tracks(selection_state),
            waveform_chunks_per_second,
            config,
        )
    };
    let ranges = match result {
        Ok(ranges) if ranges.is_empty() => {
            show_info_dialog(
                area,
                "No Silences Found",
                "No selected-audio gaps matched the configured threshold and duration.",
            );
            return;
        }
        Ok(ranges) => ranges,
        Err(error) => {
            show_info_dialog(area, "Could Not Remove Silences", &error);
            return;
        }
    };

    let position = player_state::snapshot(player_state).position;
    let (duration, changed) = {
        let mut project_state = project.borrow_mut();
        let Some((captions, video, audio)) = apply_ranges(&mut project_state, &ranges) else {
            show_info_dialog(
                area,
                "No Silences Found",
                "No timeline ranges were removed.",
            );
            return;
        };
        let duration = project_state.duration();
        project_state.normalize_clip_transitions();
        crate::project::commit_edit(&project_state, "remove-silences");
        (duration, (captions, video, audio))
    };

    selection_state::set_selected_items(selection_state, Vec::new(), None);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: changed.2,
            audio_waveforms: changed.2,
            video: changed.1,
            captions: changed.0,
            ..ProjectChange::default()
        },
    );
    player_state::seek_time(player_state, shifted_position(position, &ranges));
    area.queue_render();
}

fn show_info_dialog(area: &gtk::GLArea, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog.choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        |_| {},
    );
}

fn ignore_short_chunks(ranges: Vec<(f64, f64)>, threshold: f64) -> Vec<(f64, f64)> {
    if threshold <= 0.0 {
        return ranges;
    }

    ranges
        .into_iter()
        .filter(|(start, end)| end - start >= threshold)
        .collect()
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
        return Err("Select at least one audio item first".to_string());
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

    selected_audio_items(project, selected_items)
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

fn mixed_peak_at(
    selected: &[SelectedAudio],
    timeline_seconds: f64,
    waveform_chunks_per_second: u32,
) -> f64 {
    selected
        .iter()
        .filter_map(|audio| {
            if timeline_seconds < audio.item.start.as_secs_f64()
                || timeline_seconds >= audio.item.end.as_secs_f64()
            {
                return None;
            }
            if playback_speed_is_zero(audio.item.playback_speed) {
                return None;
            }
            let item_seconds = timeline_seconds - audio.item.start.as_secs_f64();
            let index =
                (item_seconds.max(0.0) * f64::from(waveform_chunks_per_second)).floor() as usize;
            let peak = f64::from(audio.waveform.peak(index)?);
            Some(peak / f64::from(u8::MAX))
        })
        .sum()
}

fn merge_short_gaps(mut ranges: Vec<(f64, f64)>, tolerance: f64) -> Vec<(f64, f64)> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.total_cmp(&right.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 - last.1 <= tolerance
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(range);
    }
    merged
}

fn silence_gaps(
    start: f64,
    end: f64,
    audible: &[(f64, f64)],
    min_silence: f64,
    padding: f64,
    frame_step: Time,
) -> Vec<(Time, Time)> {
    let mut ranges = Vec::new();
    let mut cursor = start;
    for &(audible_start, audible_end) in audible {
        let silence_start = cursor;
        let silence_end = (audible_start - padding).max(silence_start);
        push_gap(
            &mut ranges,
            silence_start,
            silence_end,
            min_silence,
            frame_step,
        );
        cursor = (audible_end + padding).min(end).max(cursor);
    }
    push_gap(&mut ranges, cursor, end, min_silence, frame_step);
    ranges
}

fn push_gap(
    ranges: &mut Vec<(Time, Time)>,
    start: f64,
    end: f64,
    min_silence: f64,
    frame_step: Time,
) {
    if end - start >= min_silence {
        let start = Time::from_seconds_f64(start).snapped(frame_step);
        let end = Time::from_seconds_f64(end).snapped(frame_step);
        if end > start {
            ranges.push((start, end));
        }
    }
}

fn db_to_amplitude(db: f64) -> f64 {
    if db <= MIN_DB {
        0.0
    } else {
        10.0_f64.powf(db / 20.0)
    }
}

fn bin_start(bin: usize, waveform_chunks_per_second: u32) -> f64 {
    bin as f64 / f64::from(waveform_chunks_per_second)
}
