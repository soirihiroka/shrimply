use shrimply_project_document::project::CaptionItem;

use super::{Inspectable, InspectorContext, list, section::InspectorSection};

impl Inspectable for CaptionItem {
    fn title(&self) -> &'static str {
        "Caption"
    }

    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        for widget in self.inspect(context) {
            section.add_wide_control(&widget);
        }
    }

    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let document = shrimply_inspector_document::basic(context.inspector_core.snapshot());
        list::render_document(document, context, super::control::section)
    }
}
