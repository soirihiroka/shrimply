use super::{Inspectable, InspectorContext, section::InspectorSection};
use shrimply_project_document::project::CommentItem;

impl Inspectable for CommentItem {
    fn title(&self) -> &'static str {
        "Comment"
    }
    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        for control in shrimply_inspector_core::comment::section(self).controls {
            super::control::add_control(section, context, &context.list_target, control);
        }
    }
}
