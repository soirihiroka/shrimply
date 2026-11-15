use super::Runtime;
use serde_json::Value;
use shrimply_inspector_document as document;
use shrimply_player_core as player;

impl Runtime {
    pub fn poll_inspector(&mut self) -> bool {
        if !self.foreground
            || self.preview_interaction.active()
            || self.inspector_interaction.active()
            || self
                .framegraph
                .as_ref()
                .is_some_and(|graph| graph.is_editing())
        {
            return false;
        }
        let revision = player::snapshot(&self.player).revision;
        let changed = self.inspector.as_ref().is_some_and(document::poll);
        if changed {
            self.inspector_stamp = None;
            if player::snapshot(&self.player).revision != revision
                && let Err(error) = self.timeline_changed()
            {
                self.error = Some(error);
            }
        }
        changed
    }

    pub(super) fn update_inspector(&mut self) {
        let Some(controller) = &self.inspector else {
            return;
        };
        if !self.inspector_visible || self.preview_interaction.active() {
            return;
        }
        if let Err(error) = self.inspector_interaction.synchronize(controller) {
            self.error = Some(error);
        }
        if self.inspector_interaction.active()
            && let Some(document) = &self.inspector_document
        {
            self.inspector_status = self.inspector_interaction.snapshot(document).to_string();
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
        self.inspector_status = self.inspector_interaction.snapshot(&next).to_string();
        self.inspector_document = Some(next);
        self.inspector_stamp = Some(stamp);
    }

    pub fn cancel_numeric(&mut self) -> Result<(), String> {
        if let Some(controller) = &self.inspector {
            self.inspector_interaction.cancel(controller)?;
        }
        self.inspector_stamp = None;
        Ok(())
    }

    pub fn finish_numeric(&mut self) -> Result<(), String> {
        if let Some(controller) = &self.inspector {
            self.inspector_interaction.finish(controller)?;
        }
        self.inspector_stamp = None;
        Ok(())
    }

    pub(super) fn apply_inspector_numeric(&mut self, request: &Value) -> Result<Value, String> {
        if request["event"] == "cancel" && !self.inspector_interaction.active() {
            return Ok(
                serde_json::json!({"active":false,"capture":false,"editing":false,"valid":true}),
            );
        }
        self.update_inspector();
        let controller = self.inspector.as_ref().ok_or("open a project first")?;
        let document = self
            .inspector_document
            .as_ref()
            .ok_or("open the inspector first")?;
        if let Some(focus) = document::remote::preview_focus(controller, document, request) {
            self.preview_interaction.focus(Some(focus));
        }
        let revision = player::snapshot(&self.player).revision;
        let result = self
            .inspector_interaction
            .apply(controller, document, request);
        self.inspector_stamp = None;
        if player::snapshot(&self.player).revision != revision {
            self.timeline_changed()?;
        }
        result
    }

    pub(super) fn apply_inspector_file(
        &mut self,
        request: &Value,
        source: Option<std::fs::File>,
    ) -> Result<Value, String> {
        self.inspector_stamp = None;
        self.update_inspector();
        let controller = self.inspector.as_ref().ok_or("open a project first")?;
        let document = self
            .inspector_document
            .as_ref()
            .ok_or("open the inspector first")?;
        let mut command = request["command"].clone();
        document::remote::validate_file_command(controller, document, &command)?;
        let staged = self.backend.stage_picked_file(request, source)?;
        let inspected = shrimply_asset::Asset::new(&staged.path).snapshot();
        let result = (|| {
            inspected.as_ref().map_err(Clone::clone)?;
            command["path"] = serde_json::json!(staged.path);
            self.apply_inspector(&command)?;
            self.backend.snapshot()
        })();
        if let Err(error) = &result {
            self.backend
                .discard_staged(&staged, inspected.as_ref().ok())
                .map_err(|cleanup| format!("{error}; {cleanup}"))?;
        }
        result
    }

    pub(super) fn apply_inspector(&mut self, request: &Value) -> Result<Option<String>, String> {
        self.finish_numeric()?;
        self.inspector_stamp = None;
        self.update_inspector();
        let controller = self.inspector.as_ref().ok_or("open a project first")?;
        let document = self
            .inspector_document
            .as_ref()
            .ok_or("open the inspector first")?;
        if let Some(focus) = document::remote::preview_focus(controller, document, request) {
            self.preview_interaction.focus(Some(focus));
        }
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
