use shrimply_preview_interaction_core::provider::{self, GeometryPreparation};
pub use shrimply_preview_interaction_core::provider::{
    BuildContext, PreparedGeometry, SnapPreparation, prepare_snap_scene, update_text_source_size,
};

pub fn prepare_geometry(
    project: &shrimply_project::project::Project,
    address: &shrimply_project::project::ItemAddress,
    position: shrimply_project::project::Time,
    audio_analysis: &shrimply_evaluation::FrameAudioAnalysis,
    expression_cache: &std::cell::RefCell<shrimply_evaluation::TransformExpressionCache>,
    viewport: shrimply_preview_core::PreviewViewport,
    extensions: Option<
        &std::collections::HashMap<
            shrimply_preview_core::PreviewExtensionKey,
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

pub fn sample_camera(
    id: uuid::Uuid,
    source: &shrimply_3dgs::TrackingCameraSource,
    time: shrimply_project::project::Time,
) -> Option<shrimply_project::project::TrackedCameraPreview> {
    shrimply_video_cuda::camera_reconstruction::sample(id, source, time).map(|camera| {
        shrimply_project::project::TrackedCameraPreview {
            position: camera.position,
            rotation: camera.rotation,
            projection: camera.projection,
            vertical_fov_degrees: camera.vertical_fov_degrees,
        }
    })
}
