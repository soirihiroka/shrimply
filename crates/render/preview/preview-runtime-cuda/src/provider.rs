use shrimply_preview_interaction_skia::provider::{self, GeometryPreparation};
pub use shrimply_preview_interaction_skia::provider::{
    BuildContext, PreparedGeometry, SnapPreparation, prepare_snap_scene, sample_camera,
    update_text_source_size,
};

pub fn prepare_geometry(
    project: &shrimply_project_document::project::Project,
    address: &shrimply_project_document::project::ItemAddress,
    position: shrimply_project_document::project::Time,
    audio_analysis: &shrimply_project_evaluation::FrameAudioAnalysis,
    expression_cache: &std::cell::RefCell<shrimply_project_evaluation::TransformExpressionCache>,
    viewport: shrimply_preview_provider_skia::PreviewViewport,
    extensions: Option<
        &std::collections::HashMap<
            shrimply_preview_provider_skia::PreviewExtensionKey,
            Box<dyn std::any::Any>,
        >,
    >,
) -> Option<PreparedGeometry> {
    provider::prepare_geometry(
        project,
        address,
        position,
        GeometryPreparation {
            audio_analysis,
            expression_cache,
            viewport,
            extensions,
            camera_sampler: sample_camera,
        },
    )
}
