#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextMenuAction {
    Copy,
    Cut,
    Paste,
    ReplaceProperties,
    PasteModifiers,
    ShowInFolder,
    FoldSequence,
    UnlinkFolder,
    AddFolderTrackTop,
    AddFolderTrackBottom,
    CopyFrame,
    SaveFrame,
    EnableBeatDetection,
    DisableBeatDetection,
    ExportAudio,
    Transcribe,
    RemoveSilences,
    GenerateSpeech,
    AddCaptionTrack,
    AddVideoTrack,
    AddAudioTrack,
    MoveOutOfSequence,
    Group,
    Ungroup,
    DeleteFoldedTrack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoFrameSelection {
    Items,
    Tracks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextMenuRequest {
    SetTimelineClipboardMarker,
    PasteFromClipboard,
    CopyFrame(VideoFrameSelection),
    SaveFrame(VideoFrameSelection),
    ShowInFolder,
    ExportAudio,
    Transcribe,
    RemoveSilences,
    GenerateSpeech,
    DeleteFoldedTrack { clip_count: usize },
}

pub const TIMELINE_CLIPBOARD_MARKER: &str = "shrimply timeline items";

impl ContextMenuAction {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "cut",
            Self::Paste => "paste",
            Self::ReplaceProperties => "replace-properties",
            Self::PasteModifiers => "paste-modifiers",
            Self::ShowInFolder => "show-folder",
            Self::FoldSequence => "fold-sequence",
            Self::UnlinkFolder => "unlink-folder",
            Self::AddFolderTrackTop => "add-folder-track-top",
            Self::AddFolderTrackBottom => "add-folder-track-bottom",
            Self::CopyFrame => "copy-frame",
            Self::SaveFrame => "save-frame",
            Self::EnableBeatDetection | Self::DisableBeatDetection => "beat-detection",
            Self::ExportAudio => "export-audio",
            Self::Transcribe => "transcribe",
            Self::RemoveSilences => "remove-silences",
            Self::GenerateSpeech => "generate-speech",
            Self::AddCaptionTrack => "add-caption-track",
            Self::AddVideoTrack => "add-video-track",
            Self::AddAudioTrack => "add-audio-track",
            Self::MoveOutOfSequence => "move-out-of-sequence",
            Self::Group => "group",
            Self::Ungroup => "ungroup",
            Self::DeleteFoldedTrack => "delete-folded-track",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::ReplaceProperties => "Replace Properties",
            Self::PasteModifiers => "Paste Modifiers",
            Self::ShowInFolder => "Show in Folder",
            Self::FoldSequence => "Fold Sequence",
            Self::UnlinkFolder => "Unlink Folder",
            Self::AddFolderTrackTop => "Add Track at Top",
            Self::AddFolderTrackBottom => "Add Track at Bottom",
            Self::CopyFrame => "Copy Frame",
            Self::SaveFrame => "Save Frame…",
            Self::EnableBeatDetection => "Enable Beat Detection",
            Self::DisableBeatDetection => "Disable Beat Detection",
            Self::ExportAudio => "Export Audio…",
            Self::Transcribe => "Transcribe",
            Self::RemoveSilences => "Remove Silences",
            Self::GenerateSpeech => "Generate Speech",
            Self::AddCaptionTrack => "Add Caption Track",
            Self::AddVideoTrack => "Add Video Track",
            Self::AddAudioTrack => "Add Audio Track",
            Self::MoveOutOfSequence => "Move Out",
            Self::Group => "Group",
            Self::Ungroup => "Ungroup",
            Self::DeleteFoldedTrack => "Delete Track",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextMenuItem {
    pub action: ContextMenuAction,
    pub enabled: bool,
}

impl ContextMenuItem {
    pub const fn new(action: ContextMenuAction) -> Self {
        Self {
            action,
            enabled: true,
        }
    }

    pub const fn enabled(action: ContextMenuAction, enabled: bool) -> Self {
        Self { action, enabled }
    }

    pub const fn label(self) -> &'static str {
        self.action.label()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextMenuControl {
    PlaybackSpeed { position: f64, mixed: bool },
    AudioTrackGain { db: f32 },
}

impl ContextMenuControl {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlaybackSpeed { .. } => "Speed",
            Self::AudioTrackGain { .. } => "Gain Offset",
        }
    }

    pub fn value(self) -> f64 {
        match self {
            Self::PlaybackSpeed { position, .. } => position,
            Self::AudioTrackGain { db } => f64::from(db),
        }
    }

    pub const fn minimum(self) -> f64 {
        match self {
            Self::PlaybackSpeed { .. } => -2.0,
            Self::AudioTrackGain { .. } => shrimply_project::AUDIO_TRACK_GAIN_MIN_DB as f64,
        }
    }

    pub const fn maximum(self) -> f64 {
        match self {
            Self::PlaybackSpeed { .. } => 2.0,
            Self::AudioTrackGain { .. } => shrimply_project::AUDIO_TRACK_GAIN_MAX_DB as f64,
        }
    }

    pub const fn step(self) -> f64 {
        match self {
            Self::PlaybackSpeed { .. } => 0.05,
            Self::AudioTrackGain { .. } => 0.5,
        }
    }

    pub const fn mixed(self) -> bool {
        matches!(self, Self::PlaybackSpeed { mixed: true, .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextMenuEntry {
    Action(ContextMenuItem),
    Control(ContextMenuControl),
}

pub type ContextMenuSection = Vec<ContextMenuEntry>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextMenu {
    pub sections: Vec<ContextMenuSection>,
}

impl ContextMenu {
    pub fn actions(&self) -> impl Iterator<Item = ContextMenuItem> + '_ {
        self.sections
            .iter()
            .flatten()
            .filter_map(|entry| match entry {
                ContextMenuEntry::Action(item) => Some(*item),
                ContextMenuEntry::Control(_) => None,
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextItemKind {
    Caption,
    Video,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemMenuContext {
    pub kind: ContextItemKind,
    pub can_replace_properties: bool,
    pub can_paste_modifiers: bool,
    pub has_file: bool,
    pub foldable: bool,
    pub unlinkable_folder: bool,
    pub folder: bool,
    pub playback_speed: Option<ContextMenuControl>,
    pub enable_beat_detection: bool,
    pub can_remove_silences: bool,
}

pub fn item_context_menu(context: ItemMenuContext) -> ContextMenu {
    let mut sections = vec![actions(&[
        ContextMenuItem::new(ContextMenuAction::Copy),
        ContextMenuItem::new(ContextMenuAction::Cut),
        ContextMenuItem::new(ContextMenuAction::Paste),
    ])];
    if matches!(
        context.kind,
        ContextItemKind::Video | ContextItemKind::Audio
    ) {
        sections.push(actions(&[
            ContextMenuItem::enabled(
                ContextMenuAction::ReplaceProperties,
                context.can_replace_properties,
            ),
            ContextMenuItem::enabled(
                ContextMenuAction::PasteModifiers,
                context.can_paste_modifiers,
            ),
        ]));
    }
    if context.has_file {
        sections.push(actions(&[ContextMenuItem::new(
            ContextMenuAction::ShowInFolder,
        )]));
    }
    let mut sequence = Vec::new();
    if context.foldable {
        sequence.push(ContextMenuItem::new(ContextMenuAction::FoldSequence));
    }
    if context.unlinkable_folder {
        sequence.push(ContextMenuItem::new(ContextMenuAction::UnlinkFolder));
    }
    if context.folder {
        sequence.extend([
            ContextMenuItem::new(ContextMenuAction::AddFolderTrackTop),
            ContextMenuItem::new(ContextMenuAction::AddFolderTrackBottom),
        ]);
    }
    if !sequence.is_empty() {
        sections.push(actions(&sequence));
    }
    if let Some(control) = context.playback_speed {
        sections.push(vec![ContextMenuEntry::Control(control)]);
    }
    sections.push(match context.kind {
        ContextItemKind::Caption => {
            actions(&[ContextMenuItem::new(ContextMenuAction::GenerateSpeech)])
        }
        ContextItemKind::Video => actions(&[
            ContextMenuItem::new(ContextMenuAction::CopyFrame),
            ContextMenuItem::new(ContextMenuAction::SaveFrame),
        ]),
        ContextItemKind::Audio => {
            let beat = if context.enable_beat_detection {
                ContextMenuAction::EnableBeatDetection
            } else {
                ContextMenuAction::DisableBeatDetection
            };
            let mut items = vec![
                ContextMenuItem::new(beat),
                ContextMenuItem::new(ContextMenuAction::ExportAudio),
                ContextMenuItem::new(ContextMenuAction::Transcribe),
            ];
            if context.can_remove_silences {
                items.push(ContextMenuItem::new(ContextMenuAction::RemoveSilences));
            }
            actions(&items)
        }
    });
    ContextMenu { sections }
}

pub fn empty_track_context_menu() -> ContextMenu {
    ContextMenu {
        sections: vec![actions(&[
            ContextMenuItem::new(ContextMenuAction::AddCaptionTrack),
            ContextMenuItem::new(ContextMenuAction::AddVideoTrack),
            ContextMenuItem::new(ContextMenuAction::AddAudioTrack),
        ])],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackMenuContext {
    Caption,
    Video,
    Audio {
        can_remove_silences: bool,
        gain_db: f32,
    },
}

pub fn track_context_menu(context: TrackMenuContext) -> ContextMenu {
    let sections = match context {
        TrackMenuContext::Caption => vec![actions(&[ContextMenuItem::new(
            ContextMenuAction::GenerateSpeech,
        )])],
        TrackMenuContext::Video => vec![actions(&[
            ContextMenuItem::new(ContextMenuAction::CopyFrame),
            ContextMenuItem::new(ContextMenuAction::SaveFrame),
        ])],
        TrackMenuContext::Audio {
            can_remove_silences,
            gain_db,
        } => {
            let mut items = vec![
                ContextMenuItem::new(ContextMenuAction::ExportAudio),
                ContextMenuItem::new(ContextMenuAction::Transcribe),
            ];
            if can_remove_silences {
                items.push(ContextMenuItem::new(ContextMenuAction::RemoveSilences));
            }
            vec![
                actions(&items),
                vec![ContextMenuEntry::Control(
                    ContextMenuControl::AudioTrackGain { db: gain_db },
                )],
            ]
        }
    };
    ContextMenu { sections }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldedItemMenuContext {
    pub groupable: bool,
    pub ungroupable: bool,
    pub folder: bool,
    pub can_replace_properties: bool,
    pub can_paste_modifiers: bool,
}

pub fn folded_item_context_menu(context: FoldedItemMenuContext) -> ContextMenu {
    let mut sections = vec![actions(&[ContextMenuItem::new(
        ContextMenuAction::MoveOutOfSequence,
    )])];
    let mut grouping = Vec::new();
    if context.groupable {
        grouping.push(ContextMenuItem::new(ContextMenuAction::Group));
    }
    if context.ungroupable {
        grouping.push(ContextMenuItem::new(ContextMenuAction::Ungroup));
    }
    if !grouping.is_empty() {
        sections.push(actions(&grouping));
    }
    if context.folder {
        sections.push(actions(&[
            ContextMenuItem::new(ContextMenuAction::AddFolderTrackTop),
            ContextMenuItem::new(ContextMenuAction::AddFolderTrackBottom),
        ]));
    }
    sections.push(actions(&[
        ContextMenuItem::enabled(
            ContextMenuAction::ReplaceProperties,
            context.can_replace_properties,
        ),
        ContextMenuItem::enabled(
            ContextMenuAction::PasteModifiers,
            context.can_paste_modifiers,
        ),
    ]));
    ContextMenu { sections }
}

pub fn folded_track_context_menu() -> ContextMenu {
    ContextMenu {
        sections: vec![actions(&[ContextMenuItem::new(
            ContextMenuAction::DeleteFoldedTrack,
        )])],
    }
}

fn actions(items: &[ContextMenuItem]) -> ContextMenuSection {
    items
        .iter()
        .copied()
        .map(ContextMenuEntry::Action)
        .collect()
}
