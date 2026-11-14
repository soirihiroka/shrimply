use super::{Inspectable, InspectorContext, section::InspectorSection};
use shrimply_inspector_core::TransitionPresentation;

impl Inspectable for TransitionPresentation {
    fn title(&self) -> &'static str {
        self.title
    }
    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        for control in self.section().controls {
            super::control::add_control(section, context, &context.list_target, control);
        }
    }
}
