use shrimply_editor_state::player_state::{self, ProjectChange};
use shrimply_project_document::project::{ItemKind, Project, TrackAddress};
use shrimply_timeline_edit::edit;

use crate::{InspectorController, InspectorDetail};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackPresentation {
    pub address: TrackAddress,
    pub kind: ItemKind,
    pub ordinal: usize,
    pub enabled: bool,
    pub language: Option<String>,
    pub item_count: usize,
}

impl TrackPresentation {
    pub fn title(&self) -> &'static str {
        match self.kind {
            ItemKind::Video => "Video Track",
            ItemKind::Comment => "Comment Track",
            ItemKind::Caption => "Caption Track",
            ItemKind::Audio => "Audio Track",
        }
    }

    pub fn details(&self) -> Vec<InspectorDetail> {
        vec![
            InspectorDetail {
                label: "Type",
                value: self.title().to_string(),
            },
            InspectorDetail {
                label: "Track",
                value: (self.ordinal + 1).to_string(),
            },
            InspectorDetail {
                label: "Items",
                value: self.item_count.to_string(),
            },
        ]
    }
}

pub fn presentation(project: &Project, address: TrackAddress) -> Option<TrackPresentation> {
    let track = project.track(&address)?;
    let (enabled, language) = (track.enabled(), track.caption_language());
    let item_count = track.items().count();
    let ordinal = match &address {
        TrackAddress::Comment { track_id } => project
            .comment_tracks
            .iter()
            .position(|track| track.id == *track_id)?,
        TrackAddress::Caption { track_id } => project
            .caption_tracks
            .iter()
            .position(|track| track.id == *track_id)?,
        TrackAddress::Video {
            sequence_path,
            track_id,
        } => project
            .video_tracks_for_path(sequence_path)?
            .iter()
            .position(|track| track.id == *track_id)?,
        TrackAddress::Audio {
            sequence_path,
            track_id,
        } => project
            .audio_tracks_for_path(sequence_path)?
            .iter()
            .position(|track| track.id == *track_id)?,
    };
    Some(TrackPresentation {
        kind: address.kind(),
        address,
        ordinal,
        enabled,
        language,
        item_count,
    })
}

impl InspectorController {
    pub fn set_track_enabled(&self, address: &TrackAddress, next: bool) -> Result<(), String> {
        let kind = address.kind();
        let mut project = self.project.borrow_mut();
        if edit::set_track_enabled(&mut project, address, next) != Some(true) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "toggle-track-enabled");
        let duration = project.duration();
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            ProjectChange {
                duration: Some(duration),
                audio: kind == ItemKind::Audio,
                audio_beats: kind == ItemKind::Audio,
                audio_waveforms: kind == ItemKind::Audio,
                video: kind == ItemKind::Video,
                captions: kind == ItemKind::Caption,
                inspector: true,
                ..ProjectChange::default()
            },
        );
        Ok(())
    }

    pub fn set_caption_track_language(
        &self,
        address: &TrackAddress,
        language: Option<&str>,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        if project.track(address).is_none()
            || !edit::set_caption_track_language(&mut project, address, language)?
        {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "set-caption-track-language");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            ProjectChange {
                captions: true,
                inspector: true,
                ..ProjectChange::default()
            },
        );
        Ok(())
    }
}
