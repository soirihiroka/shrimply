use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_project_document::{Project, TrackKind};

use crate::edit;

pub const LABEL: &str = "Project options";
pub const ICON: &str = "open-menu-symbolic";

macro_rules! actions {
    ($($action:ident, $id:literal, $label:literal;)+) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Action { $($action,)+ }

        impl Action {
            pub const ALL: &'static [Self] = &[$(Self::$action,)+];
            pub const fn id(self) -> &'static str {
                match self { $(Self::$action => $id,)+ }
            }

            pub const fn label(self) -> &'static str {
                match self { $(Self::$action => $label,)+ }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                match id { $($id => Some(Self::$action),)+ _ => None }
            }
        }
    };
}

actions! {
    NewProject, "new-project", "New Project";
    OpenProject, "open-project", "Open Project…";
    NewCommentTrack, "new-comment-track", "Comment Track";
    NewCaptionTrack, "new-caption-track", "Caption Track";
    NewVideoTrack, "new-video-track", "Video Track";
    NewAudioTrack, "new-audio-track", "Audio Track";
    Save, "save", "Save";
    SaveAs, "save-as", "Save As…";
    Import, "import", "Import";
    Undo, "undo", "Undo";
    Redo, "redo", "Redo";
    Preferences, "preferences", "Settings";
    Shortcuts, "show-shortcuts", "Keyboard Shortcuts";
    RestartService, "restart-service", "Restart Local Service";
    About, "about", "About Shrimply";
    Close, "close", "Close";
}

impl Action {
    pub fn edit(self) -> Option<Edit> {
        let kind = match self {
            Self::NewCommentTrack => TrackKind::Comment,
            Self::NewCaptionTrack => TrackKind::Caption,
            Self::NewVideoTrack => TrackKind::Video,
            Self::NewAudioTrack => TrackKind::Audio,
            _ => return None,
        };
        Some(Edit { kind })
    }

    pub fn history(self) -> Option<bool> {
        match self {
            Self::Undo => Some(false),
            Self::Redo => Some(true),
            _ => None,
        }
    }

    fn needs_host(self) -> bool {
        self.edit().is_none() && self.history().is_none() && self != Self::Save
    }

    pub fn enabled(self, can_undo: bool, can_redo: bool) -> bool {
        match self {
            Self::Undo => can_undo,
            Self::Redo => can_redo,
            _ => true,
        }
    }
}

/// A menu mutation executes through the session's usual project transaction.
#[derive(Clone, Copy)]
pub struct Edit {
    kind: TrackKind,
}

impl Edit {
    pub const DESCRIPTION: &str = edit::CREATE_TRACK_EDIT;

    pub fn apply(self, project: &mut Project) -> Result<TrackKind, String> {
        edit::create_track(
            project,
            &shrimply_project_document::SequenceScopeId::root(),
            self.kind,
            true,
        )?;
        Ok(self.kind)
    }
}

pub enum Entry {
    Action(Action),
    Submenu(&'static str, Vec<Entry>),
}

impl Entry {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Submenu(label, _) => label,
        }
    }
}

use Action::*;
use Entry::{Action as Item, Submenu};

pub fn sections(mut host_supports: impl FnMut(Action) -> bool) -> Vec<Vec<Entry>> {
    fn filter(entries: &mut Vec<Entry>, supports: &mut impl FnMut(Action) -> bool) {
        entries.retain_mut(|entry| match entry {
            Item(action) => !action.needs_host() || supports(*action),
            Submenu(_, children) => {
                filter(children, supports);
                !children.is_empty()
            }
        });
    }

    [
        vec![
            Item(NewProject),
            Item(OpenProject),
            Submenu(
                "New Track",
                vec![
                    Item(NewCommentTrack),
                    Item(NewCaptionTrack),
                    Item(NewVideoTrack),
                    Item(NewAudioTrack),
                ],
            ),
            Item(Import),
            Item(Save),
            Item(SaveAs),
        ],
        vec![Item(Undo), Item(Redo)],
        vec![Item(Preferences), Item(Shortcuts), Item(RestartService)],
        vec![Item(About), Item(Close)],
    ]
    .into_iter()
    .filter_map(|mut entries| {
        filter(&mut entries, &mut host_supports);
        (!entries.is_empty()).then_some(entries)
    })
    .collect()
}

pub fn document(
    host_supports: impl FnMut(Action) -> bool,
    can_undo: bool,
    can_redo: bool,
) -> Value {
    fn describe(entry: &Entry, can_undo: bool, can_redo: bool) -> Value {
        let label = text(entry.label());
        match entry {
            Item(action) => json!({"kind":"action","label":label,"action":action.id(),
                "enabled":action.enabled(can_undo, can_redo)}),
            Submenu(id, items) => {
                json!({"kind":"submenu","id":id,"label":label,"entries":items.iter()
                .map(|item| describe(item, can_undo, can_redo)).collect::<Vec<_>>()})
            }
        }
    }

    let mut entries = Vec::new();
    for section in sections(host_supports) {
        if !entries.is_empty() {
            entries.push(json!({"kind":"separator"}));
        }
        entries.extend(
            section
                .iter()
                .map(|item| describe(item, can_undo, can_redo)),
        );
    }
    json!({"label":text(LABEL),"icon":ICON,"entries":entries})
}
