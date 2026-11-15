//! Native requests forward to the shared timeline service session.
use super::Runtime;
use serde_json::{Value, json};
use shrimply_inspector_document::timeline_service::Session;
use shrimply_timeline_skia::ContextMenuRequest;

impl Runtime {
    pub(super) fn open_timeline_service(
        &mut self,
        request: ContextMenuRequest,
    ) -> Result<(), String> {
        if self.timeline_service.is_some() {
            return Err("A timeline service is already open".into());
        }
        let scene = self.timeline.as_ref().ok_or("Open a project first")?;
        self.timeline_service = Some(Session::open(
            request,
            scene,
            &self.backend.preferences,
            shrimply_player_core::snapshot(&self.player).revision,
            self.backend.wake.clone(),
        )?);
        Ok(())
    }

    pub fn poll_timeline_service(&mut self) -> bool {
        let mut changed = match (&mut self.timeline_service, &mut self.timeline) {
            (Some(service), Some(scene)) => service.poll(scene, &self.backend.preferences),
            _ => false,
        };
        if self
            .timeline_service
            .as_ref()
            .is_some_and(Session::ready_to_close)
        {
            if let Err(error) = self
                .timeline_service
                .as_mut()
                .expect("closing service")
                .cleanup(self.timeline.as_ref())
            {
                self.error = Some(error);
            }
            self.timeline_service = None;
            changed = true;
        }
        changed
    }

    pub(super) fn timeline_service_request(
        &mut self,
        request: &Value,
        source: Option<std::fs::File>,
    ) -> Result<Value, String> {
        self.backend.validate_target(request)?;
        let command = if request["op"] == "timeline_service_file" {
            &request["command"]
        } else {
            request
        };
        let service = self
            .timeline_service
            .as_ref()
            .ok_or("The timeline service is closed")?;
        service.accepts(command)?;
        let mut command = command.clone();
        let staged = if request["op"] == "timeline_service_file" {
            service.validate_file(&command)?;
            Some(self.backend.stage_picked_file(request, source)?)
        } else {
            None
        };
        if let Some(staged) = &staged {
            command["path"] = json!(staged.path);
        }
        let inspected = staged
            .as_ref()
            .map(|staged| shrimply_asset::Asset::new(&staged.path).snapshot())
            .transpose();
        let inspected = match inspected {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.backend
                    .discard_staged(staged.as_ref().expect("failed picked file"), None)?;
                return Err(error);
            }
        };
        let result = self
            .timeline_service
            .as_mut()
            .expect("validated timeline service")
            .apply(
                self.timeline.as_mut().ok_or("Open a project first")?,
                &self.backend.preferences,
                shrimply_player_core::snapshot(&self.player).revision,
                &command,
            );
        if result.is_ok() && staged.as_ref().is_some_and(|staged| staged.created) {
            self.timeline_service
                .as_mut()
                .expect("file service")
                .retain_reference(inspected.clone().expect("staged asset snapshot"));
        }
        match result {
            Ok(close) => {
                if close {
                    self.timeline_service = None;
                }
                self.timeline_changed()?;
            }
            Err(error) => {
                if let Some(staged) = &staged {
                    self.backend.discard_staged(staged, inspected.as_ref())?;
                }
                self.timeline_service
                    .as_mut()
                    .expect("failed timeline service")
                    .set_error(error);
            }
        }
        Ok(json!({}))
    }
}
