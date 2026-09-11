use crate::{InspectorController, InspectorTarget};
use shrimply_editor_state::player_state::{self, ProjectChange};
use shrimply_project_document::project::{self, Asset, VideoItemContent};

impl InspectorController {
    pub(crate) fn relink_executable_source(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
    ) -> Option<Result<(), String>> {
        if path != "/file" {
            return None;
        }
        let InspectorTarget::Item(address) = target else {
            return None;
        };
        let old = {
            let project = self.project.borrow();
            let item = project.video_item(address)?;
            if !matches!(
                item.content,
                VideoItemContent::Manim(_) | VideoItemContent::Blender(_)
            ) {
                return None;
            }
            item.file.clone()
        };
        Some((|| {
            let source = project::project_directory()
                .join(text)
                .canonicalize()
                .map_err(|error| format!("Could not resolve executable source: {error}"))?;
            let review = shrimply_trust_core::Review::new(vec![source.clone()])?;
            let controller = self.clone();
            let address = address.clone();
            let apply = move || {
                shrimply_trust_core::require(&source)?;
                let mut project = controller.project.borrow_mut();
                let item = project
                    .video_item_mut(&address)
                    .ok_or("Clip no longer exists")?;
                if item.file != old
                    || !matches!(
                        item.content,
                        VideoItemContent::Manim(_) | VideoItemContent::Blender(_)
                    )
                {
                    return Err("Clip source changed while awaiting trust; retry the edit".into());
                }
                item.file = Asset::new(source);
                project::commit_edit(&project, "relink-executable-source");
                drop(project);
                player_state::refresh_project(
                    &controller.player_state,
                    ProjectChange {
                        video: true,
                        inspector: true,
                        ..Default::default()
                    },
                );
                Ok(())
            };
            if review.files.is_empty() {
                apply()
            } else {
                shrimply_trust_core::defer_edit(review, apply)
            }
        })())
    }
}
