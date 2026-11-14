use super::Runtime;
use serde_json::Value;
use shrimply_inspector_document as document;
use shrimply_player_core as player;

impl Runtime {
    pub fn poll_inspector(&mut self) -> bool {
        if !self.inspector_visible || !self.foreground {
            return false;
        }
        let changed = self.inspector.as_ref().is_some_and(document::poll);
        if changed {
            self.inspector_stamp = None;
        }
        changed
    }

    pub(super) fn update_inspector(&mut self) {
        let Some(controller) = &self.inspector else {
            return;
        };
        if !self.inspector_visible {
            return;
        }
        let player = player::snapshot(&self.player);
        let stamp = (player.revision, player.position, controller.target());
        if self.inspector_stamp.as_ref() == Some(&stamp) {
            return;
        }
        let preferences = shrimply_preferences_core::snapshot(&self.backend.preferences);
        #[cfg(target_os = "android")]
        let format_date = crate::localization::format_date;
        #[cfg(not(target_os = "android"))]
        let format_date: fn(i64) -> Option<String> = |_| None;
        let next = document::document(
            controller,
            format_date,
            &preferences.compute_server_url,
            &preferences.last_tts_model,
        );
        self.inspector_status = document::remote::snapshot(&next).to_string();
        self.inspector_document = Some(next);
        self.inspector_stamp = Some(stamp);
    }

    pub(super) fn apply_inspector(&mut self, request: &Value) -> Result<Option<String>, String> {
        self.inspector_stamp = None;
        self.update_inspector();
        let controller = self.inspector.as_ref().ok_or("open a project first")?;
        let document = self
            .inspector_document
            .as_ref()
            .ok_or("open the inspector first")?;
        let revision = player::snapshot(&self.player).revision;
        let result = document::remote::apply(controller, document, request);
        // A native control can commit or start asynchronous work before a later
        // operation fails. Publish the canonical document in either case.
        self.inspector_stamp = None;
        if player::snapshot(&self.player).revision != revision {
            self.refresh(false);
            self.timeline_changed()?;
        }
        result
    }
}
