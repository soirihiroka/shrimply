//! Canonical add-item menus and pure item preparation shared by every frontend.
use crate::{TrackKey, TrackKind, selection_state};
use shrimply_project_document::{
    AudioItem, AudioSource, FontFamily, Project, ProjectItem, Time, TrackAddress, VideoItem,
    VideoItemContent,
};
use shrimply_property_model::timeline_value::TimelineValue;
const DEFAULT_GENERATOR_GAIN_DB: f32 = -12.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddAction {
    Comment,
    Import,
    Text,
    Shape,
    Paint,
    Background,
    Scene3d,
    VideoGeneration,
    TextToSpeech,
    AudioGenerator,
}

impl TrackAddAction {
    pub const fn label(self, kind: TrackKind) -> &'static str {
        match self {
            Self::Import if matches!(kind, TrackKind::Caption) => "Import Captions…",
            Self::Import => "Import Media…",
            Self::Comment => "Comment",
            Self::Text => "Text",
            Self::Shape => "Shape",
            Self::Paint => "Paint",
            Self::Background => "Background",
            Self::Scene3d => "3D Scene",
            Self::VideoGeneration => "Video Generation",
            Self::TextToSpeech => "Text to Speech",
            Self::AudioGenerator => "Audio Generator",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Import => "document-open-symbolic",
            Self::Comment => "rich-text-symbolic",
            Self::Text => "draw-text-symbolic",
            Self::Shape => "shapes-large-symbolic",
            Self::Paint => "applications-graphics-symbolic",
            Self::Background => "preferences-desktop-wallpaper-symbolic",
            Self::Scene3d => "3d-object-symbolic",
            Self::VideoGeneration => "video-generation-symbolic",
            Self::TextToSpeech => "font-x-generic-symbolic",
            Self::AudioGenerator => "sound-symbolic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddMenuEntry {
    Action(TrackAddAction),
    Separator,
}

pub const fn track_add_menu(kind: TrackKind) -> &'static [TrackAddMenuEntry] {
    use TrackAddAction::*;
    use TrackAddMenuEntry::*;
    match kind {
        TrackKind::Comment => &[Action(Comment)],
        TrackKind::Caption => &[Action(Import)],
        TrackKind::Video => &[
            Action(Import),
            Separator,
            Action(Text),
            Action(Shape),
            Action(Paint),
            Action(Background),
            Action(Scene3d),
            Action(VideoGeneration),
        ],
        TrackKind::Audio => &[
            Action(Import),
            Separator,
            Action(TextToSpeech),
            Action(AudioGenerator),
        ],
    }
}

pub struct TrackAddSettings<'a> {
    pub default_visual_duration: Time,
    pub default_text_font_family: &'a FontFamily,
}

pub fn prepare_track_add(
    project: &Project,
    key: TrackKey,
    action: TrackAddAction,
    position: Time,
    settings: TrackAddSettings<'_>,
) -> Result<Option<(TrackAddress, ProjectItem)>, String> {
    if !track_add_menu(key.kind)
        .iter()
        .any(|entry| matches!(entry, TrackAddMenuEntry::Action(candidate) if *candidate == action))
    {
        return Err("this add action is not available for the selected track kind".into());
    }
    if action == TrackAddAction::Import {
        return Err("choose a source file before adding imported media".into());
    }
    let Some(track_address) = selection_state::track_address(project, key) else {
        return Ok(None);
    };
    let frame_step = project.frame_step();
    let start = position.snapped(frame_step);
    let Some((occupied, next_start)) = project.track(&track_address).map(|track| match track {
        shrimply_project_document::project::TrackRef::Comment(track) => {
            track_span(track.items.iter().map(|item| (item.start, item.end)), start)
        }
        shrimply_project_document::project::TrackRef::Caption(track) => {
            track_span(track.items.iter().map(|item| (item.start, item.end)), start)
        }
        shrimply_project_document::project::TrackRef::Video(track) => {
            track_span(track.items.iter().map(|item| (item.start, item.end)), start)
        }
        shrimply_project_document::project::TrackRef::Audio(track) => {
            track_span(track.items.iter().map(|item| (item.start, item.end)), start)
        }
    }) else {
        return Ok(None);
    };
    if occupied {
        return Ok(None);
    }
    let mut end = start
        .saturating_add(settings.default_visual_duration)
        .snapped(frame_step);
    if let Some(next_start) = next_start {
        end = end.min(next_start);
    }
    if end <= start {
        return Ok(None);
    }

    let canvas_size = project.canvas_size;
    let item = match action {
        TrackAddAction::Comment => ProjectItem::Comment(
            shrimply_project_document::CommentItem::new(start, end, String::new()),
        ),
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
    Ok(Some((track_address, item)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_native_menu_actions_prepare_real_valid_items() {
        for kind in [
            TrackKind::Comment,
            TrackKind::Caption,
            TrackKind::Video,
            TrackKind::Audio,
        ] {
            for entry in track_add_menu(kind) {
                let TrackAddMenuEntry::Action(action) = entry else {
                    continue;
                };
                let mut project = Project::default();
                match kind {
                    TrackKind::Comment => project.comment_tracks.push(Default::default()),
                    TrackKind::Caption => project.caption_tracks.push(Default::default()),
                    TrackKind::Video => project.video_tracks.push(Default::default()),
                    TrackKind::Audio => project.audio_tracks.push(Default::default()),
                }
                project.ensure_ids();
                let prepared = prepare_track_add(
                    &project,
                    TrackKey {
                        kind,
                        track_index: 0,
                    },
                    *action,
                    Time::ZERO,
                    TrackAddSettings {
                        default_visual_duration: Time::from_seconds(3),
                        default_text_font_family: &FontFamily::Local {
                            name: shrimply_project_document::DEFAULT_TEXT_FONT_FAMILY.into(),
                        },
                    },
                );
                if *action == TrackAddAction::Import {
                    assert!(prepared.is_err());
                    continue;
                }
                let (address, item) = prepared.unwrap().unwrap();
                assert_eq!(item.times(), (Time::ZERO, Time::from_seconds(3)));
                project.insert_item(&address, item).unwrap();
                project.validate().unwrap();
                assert!(
                    prepare_track_add(
                        &project,
                        TrackKey {
                            kind,
                            track_index: 0
                        },
                        *action,
                        Time::ZERO,
                        TrackAddSettings {
                            default_visual_duration: Time::from_seconds(3),
                            default_text_font_family: &FontFamily::Local {
                                name: shrimply_project_document::DEFAULT_TEXT_FONT_FAMILY.into()
                            }
                        }
                    )
                    .unwrap()
                    .is_none(),
                    "occupied destination must not be overwritten"
                );
            }
        }
    }
}
