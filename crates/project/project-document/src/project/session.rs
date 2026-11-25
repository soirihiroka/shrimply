use std::sync::Arc;

use super::{Project, ProjectChanges};

const HISTORY_LIMIT: usize = 1000;
pub(super) const SHARED_HISTORY_LIMIT: usize = 32;

/// Committed document state and undo history for one open project.
///
/// The caller owns provisional edits and chooses when and where to persist
/// committed versions. A retained snapshot can be read while editing continues.
pub struct ProjectSession {
    versions: Vec<Arc<Project>>,
    index: usize,
    revision: u64,
    history_limit: usize,
    coalesce_group: Option<String>,
}

impl ProjectSession {
    pub fn new(project: Arc<Project>) -> Self {
        Self::at_revision(project, 0, HISTORY_LIMIT)
    }

    pub fn at_revision(project: Arc<Project>, revision: u64, history_limit: usize) -> Self {
        assert!(history_limit > 0);
        Self {
            versions: vec![project],
            index: 0,
            revision,
            history_limit,
            coalesce_group: None,
        }
    }

    pub fn current(&self) -> &Arc<Project> {
        &self.versions[self.index]
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Publish a validated project as the next committed version.
    pub fn commit(&mut self, project: Arc<Project>, coalesce_group: Option<&str>) {
        let coalesce = coalesce_group.is_some()
            && self.coalesce_group.as_deref() == coalesce_group
            && self.index + 1 == self.versions.len();
        if coalesce {
            self.versions[self.index] = project;
        } else {
            self.versions.truncate(self.index + 1);
            self.versions.push(project);
            if self.versions.len() > self.history_limit {
                self.versions.remove(0);
            }
            self.index = self.versions.len() - 1;
            self.coalesce_group = coalesce_group.map(str::to_owned);
        }
        self.advance_revision();
    }

    /// Rebase retained author history over a peer revision. Stop history at
    /// the first competing edit so undo never overwrites a peer's work.
    pub fn adopt_peer(&mut self, project: Arc<Project>) {
        let changes = ProjectChanges::between(self.current(), &project)
            .expect("committed project must serialize");
        let mut first = self.index.saturating_sub(SHARED_HISTORY_LIMIT - 1);
        let mut last = self.versions.len().min(first + SHARED_HISTORY_LIMIT);
        let rebase = |version: &mut Arc<Project>| {
            let Ok(mut rebased) = changes.apply(version) else {
                return false;
            };
            rebased.inherit_asset_readers(&project);
            *version = Arc::new(rebased);
            true
        };
        for (index, version) in self.versions[first..self.index]
            .iter_mut()
            .enumerate()
            .rev()
        {
            if !rebase(version) {
                first += index + 1;
                break;
            }
        }
        let next = self.index + 1;
        for (index, version) in self.versions[next..last].iter_mut().enumerate() {
            if !rebase(version) {
                last = next + index;
                break;
            }
        }
        self.versions[self.index] = project;
        self.versions.truncate(last);
        self.versions.drain(..first);
        self.index -= first;
        self.finish_coalesced_edit();
        self.advance_revision();
    }

    pub fn finish_coalesced_edit(&mut self) {
        self.coalesce_group = None;
    }

    pub fn history_destination(&self, redo: bool) -> Option<&Arc<Project>> {
        self.versions.get(self.history_index(redo)?)
    }

    /// Change the history position while retaining the caller's view state.
    pub fn change_history(&mut self, project: &mut Project, redo: bool) -> bool {
        let Some(index) = self.history_index(redo) else {
            return false;
        };
        let mut restored = (*self.versions[index]).clone();
        restored.copy_view_state_from(project);
        self.versions[self.index] = Arc::new(std::mem::replace(project, restored));
        self.index = index;
        self.finish_coalesced_edit();
        self.advance_revision();
        true
    }

    fn history_index(&self, redo: bool) -> Option<usize> {
        if redo {
            (self.index + 1 < self.versions.len()).then_some(self.index + 1)
        } else {
            self.index.checked_sub(1)
        }
    }

    fn advance_revision(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("project session revision overflow");
    }
}

impl Project {
    pub fn clear_view_state(&mut self) {
        self.cursor_position = None;
        self.timeline_zoom = None;
        self.expanded_sequence_paths.clear();
    }

    pub fn copy_view_state_from(&mut self, source: &Self) {
        self.cursor_position = source.cursor_position;
        self.timeline_zoom = source.timeline_zoom;
        self.expanded_sequence_paths
            .clone_from(&source.expanded_sequence_paths);
    }
}
