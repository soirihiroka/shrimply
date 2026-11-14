use shrimply_inspector_core::InspectorTarget;
use shrimply_project_document::project::ItemAddress;

pub(crate) use shrimply_inspector_document::InspectorCategory;

pub(crate) use shrimply_inspector_core::list::InspectorListState;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectorDocument {
    pub(crate) target: InspectorTarget,
    pub(crate) title: String,
    pub(crate) categories: Vec<InspectorCategory>,
    pub(crate) preview_item: Option<PreviewItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewItem {
    pub(crate) address: ItemAddress,
    pub(crate) id: uuid::Uuid,
}
