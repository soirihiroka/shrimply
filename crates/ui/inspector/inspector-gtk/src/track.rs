use super::{Inspectable, InspectorContext, list, section::InspectorSection};
use shrimply_inspector_core::TrackPresentation;

impl Inspectable for TrackPresentation {
    fn title(&self) -> &'static str {
        self.title()
    }
    fn add_rows(&self, _section: &InspectorSection, _context: &InspectorContext) {}
    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let preferences = shrimply_editor_state::preferences::snapshot(&context.preferences);
        let document = shrimply_inspector_document::document(
            &context.inspector_core,
            super::info::format_date,
            &preferences.compute_server_url,
            &preferences.last_tts_model,
        );
        list::render_document(document, context, super::control::section)
    }
}
