use shrimply_project_document::project::{ItemAddress, TrackAddress};
use std::collections::HashMap;

use crate::InspectorTarget;

#[derive(Default)]
pub struct InspectorListState {
    project: Option<TargetListState>,
    items: HashMap<ItemAddress, TargetListState>,
    tracks: HashMap<TrackAddress, TargetListState>,
    scroll_positions: HashMap<InspectorTarget, f64>,
}

#[derive(Default)]
struct TargetListState {
    active_category: String,
    expanded_items: HashMap<String, bool>,
}

impl InspectorListState {
    pub fn active_category(&self, target: &InspectorTarget) -> Option<&str> {
        self.target_state(target)
            .map(|state| state.active_category.as_str())
    }

    pub fn set_active_category(&mut self, target: &InspectorTarget, category: &str) {
        self.target_state_mut(target).active_category = category.to_string();
    }

    pub fn expanded(&self, target: &InspectorTarget, key: &str) -> bool {
        self.target_state(target)
            .and_then(|state| state.expanded_items.get(key))
            .copied()
            .unwrap_or_else(|| default_expanded(key))
    }

    pub fn set_expanded(&mut self, target: &InspectorTarget, key: &str, expanded: bool) {
        self.target_state_mut(target)
            .expanded_items
            .insert(key.to_string(), expanded);
    }

    pub fn scroll_position(&self, target: &InspectorTarget) -> f64 {
        self.scroll_positions
            .get(target)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_scroll_position(&mut self, target: &InspectorTarget, position: f64) {
        if position.is_finite() {
            self.scroll_positions
                .insert(target.clone(), position.max(0.0));
        }
    }

    fn target_state(&self, target: &InspectorTarget) -> Option<&TargetListState> {
        match target {
            InspectorTarget::Project => self.project.as_ref(),
            InspectorTarget::Item(address) | InspectorTarget::Transition { item: address, .. } => {
                self.items.get(address)
            }
            InspectorTarget::Track(address) => self.tracks.get(address),
        }
    }

    fn target_state_mut(&mut self, target: &InspectorTarget) -> &mut TargetListState {
        match target {
            InspectorTarget::Project => self.project.get_or_insert_default(),
            InspectorTarget::Item(address) | InspectorTarget::Transition { item: address, .. } => {
                self.items.entry(address.clone()).or_default()
            }
            InspectorTarget::Track(address) => self.tracks.entry(address.clone()).or_default(),
        }
    }
}

fn default_expanded(key: &str) -> bool {
    matches!(
        key,
        "transform" | "text" | "caption-text" | "tts" | "sanim-parameters"
    )
}
