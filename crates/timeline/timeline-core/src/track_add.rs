use crate::TrackAddAction;
use shrimply_core::timeline_value::TimelineValue;
use shrimply_project::project::{
    AudioItem, AudioSource, FontFamily, Project, ProjectItem, Time, VideoItem, VideoItemContent,
};
use shrimply_state::player_state::{self, ProjectChange, SharedPlayerState};
use shrimply_timeline::selection_state::SharedSelectionState;
use shrimply_timeline::{TrackKey, TrackKind, selection_state};
use std::cell::RefCell;
use std::rc::Rc;

const DEFAULT_GENERATOR_GAIN_DB: f32 = -12.0;

pub struct TrackAddSettings<'a> {
    pub default_visual_duration: Time,
    pub default_text_font_family: &'a FontFamily,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddOutcome {
    Import,
    Changed,
    Unchanged,
}

pub fn activate_track_add(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    key: TrackKey,
    action: TrackAddAction,
    settings: TrackAddSettings<'_>,
) -> TrackAddOutcome {
    if action == TrackAddAction::Import {
        return TrackAddOutcome::Import;
    }

    let mut project_state = project.borrow_mut();
    let Some(track_address) = selection_state::track_address(&project_state, key) else {
        return TrackAddOutcome::Unchanged;
    };
    let frame_step = project_state.frame_step();
    let start = player_state::snapshot(player_state)
        .position
        .snapped(frame_step);
    let Some((occupied, next_start)) =
        project_state
            .track(&track_address)
            .map(|track| match track {
                shrimply_project::project::TrackRef::Caption(track) => {
                    track_span(track.items.iter().map(|item| (item.start, item.end)), start)
                }
                shrimply_project::project::TrackRef::Video(track) => {
                    track_span(track.items.iter().map(|item| (item.start, item.end)), start)
                }
                shrimply_project::project::TrackRef::Audio(track) => {
                    track_span(track.items.iter().map(|item| (item.start, item.end)), start)
                }
            })
    else {
        return TrackAddOutcome::Unchanged;
    };
    if occupied {
        return TrackAddOutcome::Unchanged;
    }
    let mut end = start
        .saturating_add(settings.default_visual_duration)
        .snapped(frame_step);
    if let Some(next_start) = next_start {
        end = end.min(next_start);
    }
    if end <= start {
        return TrackAddOutcome::Unchanged;
    }

    let canvas_size = project_state.canvas_size;
    let item = match action {
        TrackAddAction::Text => {
            let mut item = VideoItem::text_item(canvas_size, start, end);
            if let VideoItemContent::Text(text) = &mut item.content {
                text.font_families = vec![settings.default_text_font_family.clone()];
            }
            ProjectItem::Video(Box::new(item))
        }
        TrackAddAction::Shape => {
            ProjectItem::Video(Box::new(VideoItem::shape_item(canvas_size, start, end)))
        }
        TrackAddAction::Paint => {
            ProjectItem::Video(Box::new(VideoItem::paint_item(canvas_size, start, end)))
        }
        TrackAddAction::Background => ProjectItem::Video(Box::new(VideoItem::background_item(
            canvas_size,
            start,
            end,
        ))),
        TrackAddAction::Scene3d => {
            ProjectItem::Video(Box::new(VideoItem::obj_scene_item(canvas_size, start, end)))
        }
        TrackAddAction::VideoGeneration => ProjectItem::Video(Box::new(
            VideoItem::video_generation_item(canvas_size, start, end),
        )),
        TrackAddAction::TextToSpeech => ProjectItem::Audio(Box::new(
            AudioItem::builder(start, end)
                .source(AudioSource::Tts(Box::default()))
                .build(),
        )),
        TrackAddAction::AudioGenerator => {
            let mut item = AudioItem::builder(start, end)
                .source(AudioSource::Generator(Box::default()))
                .build();
            item.gain.decibels = TimelineValue::new_const(DEFAULT_GENERATOR_GAIN_DB);
            ProjectItem::Audio(Box::new(item))
        }
        TrackAddAction::Import => unreachable!(),
    };
    let Some(item_address) = project_state.insert_item(&track_address, item) else {
        return TrackAddOutcome::Unchanged;
    };
    let selected = selection_state::item_key(&project_state, &item_address)
        .expect("inserted track item must have a root item key");
    let duration = project_state.duration();
    shrimply_project::project::commit_edit(&project_state, "create-generated-item");
    drop(project_state);

    selection_state::set_selected_items(selection_state, vec![selected], Some(selected));
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: key.kind == TrackKind::Audio,
            audio_waveforms: key.kind == TrackKind::Audio,
            video: key.kind == TrackKind::Video,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    TrackAddOutcome::Changed
}

fn track_span(items: impl Iterator<Item = (Time, Time)>, start: Time) -> (bool, Option<Time>) {
    let items = items.collect::<Vec<_>>();
    (
        items
            .iter()
            .any(|(item_start, item_end)| start >= *item_start && start < *item_end),
        items
            .iter()
            .filter_map(|(item_start, _)| (*item_start > start).then_some(*item_start))
            .min(),
    )
}
