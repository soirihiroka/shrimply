pub use shrimply_timeline_core::{
    ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry, ContextMenuItem,
    ContextMenuRequest, CursorTool, DragCollisionMode, TIMELINE_CLIPBOARD_MARKER, TrackAddAction,
    TrackAddMenuEntry, VideoFrameSelection,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuEntry {
    Separator,
    Action(ContextMenuItem),
    Control(ContextMenuControl),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackAddMenuModel {
    entries: Vec<TrackAddMenuEntry>,
}

impl TrackAddMenuModel {
    pub fn new(kind: shrimply_timeline::TrackKind) -> Self {
        Self {
            entries: shrimply_timeline_core::track_add_menu(kind).to_vec(),
        }
    }

    pub fn entries(&self) -> &[TrackAddMenuEntry] {
        &self.entries
    }

    pub fn action(&self, index: usize) -> Option<TrackAddAction> {
        match self.entries.get(index) {
            Some(TrackAddMenuEntry::Action(action)) => Some(*action),
            Some(TrackAddMenuEntry::Separator) | None => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuModel {
    entries: Vec<MenuEntry>,
}

impl MenuModel {
    pub fn new(contract: &ContextMenu) -> Self {
        let mut entries = Vec::new();
        for section in &contract.sections {
            if !entries.is_empty() && !section.is_empty() {
                entries.push(MenuEntry::Separator);
            }
            entries.extend(section.iter().map(|entry| match entry {
                ContextMenuEntry::Action(item) => MenuEntry::Action(*item),
                ContextMenuEntry::Control(control) => MenuEntry::Control(*control),
            }));
        }
        Self { entries }
    }

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    pub fn action(&self, index: usize) -> Option<ContextMenuAction> {
        match self.entries.get(index) {
            Some(MenuEntry::Action(item)) if item.enabled => Some(item.action),
            _ => None,
        }
    }

    pub fn control(&self, index: usize) -> Option<ContextMenuControl> {
        match self.entries.get(index) {
            Some(MenuEntry::Control(control)) => Some(*control),
            _ => None,
        }
    }
}
