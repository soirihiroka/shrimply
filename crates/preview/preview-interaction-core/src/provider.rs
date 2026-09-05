use std::{any::Any, cell::RefCell, collections::HashMap};

use glam::Vec2;
use shrimply_core::timeline_value::{TimelineExpressionValue, TimelineValue};
use shrimply_evaluation::{TransformExpressionCache, VisualEvaluation};
use shrimply_math_color::Color;
use shrimply_preview_core::{
    PreviewBuilder, PreviewContext, PreviewExtensionKey, PreviewItemGeometry, PreviewTarget,
    PreviewViewport, SnapScene,
};
use shrimply_project::project::{
    ItemAddress, PreviewGuides, Project, Time, TrackedCameraPreview, VideoItemContent,
};

#[derive(Clone, Copy)]
pub struct SnapPreparation<'a> {
    pub audio_analysis: &'a shrimply_evaluation::FrameAudioAnalysis,
    pub expression_cache: &'a RefCell<TransformExpressionCache>,
    pub extensions: &'a HashMap<PreviewExtensionKey, Box<dyn Any>>,
    pub guides: Option<&'a PreviewGuides>,
    pub radius_px: f32,
}

pub type CameraSampler =
    fn(uuid::Uuid, &shrimply_3dgs::TrackingCameraSource, Time) -> Option<TrackedCameraPreview>;

#[derive(Clone, Copy)]
pub struct GeometryPreparation<'a> {
    pub audio_analysis: &'a shrimply_evaluation::FrameAudioAnalysis,
    pub expression_cache: &'a RefCell<TransformExpressionCache>,
    pub viewport: PreviewViewport,
    pub extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    pub camera_sampler: CameraSampler,
}

pub struct PreparedGeometry {
    pub evaluation: VisualEvaluation,
    pub keyframe_time: Time,
    pub geometry: PreviewItemGeometry,
    pub source_sizes: HashMap<uuid::Uuid, Vec2>,
    pub tracked_camera: Option<TrackedCameraPreview>,
    pub item_id: uuid::Uuid,
}

impl PreparedGeometry {
    pub fn context<'a>(
        &'a self,
        position: Time,
        expression_cache: &'a RefCell<TransformExpressionCache>,
        viewport: PreviewViewport,
    ) -> BuildContext<'a> {
        BuildContext::new(
            &self.evaluation,
            position,
            expression_cache,
            viewport,
            &self.source_sizes,
            self.item_id,
        )
        .geometry(self.geometry)
        .tracked_camera(self.tracked_camera.as_ref())
    }
}

pub fn prepare_geometry(
    project: &Project,
    address: &ItemAddress,
    position: Time,
    preparation: GeometryPreparation<'_>,
) -> Option<PreparedGeometry> {
    let GeometryPreparation {
        audio_analysis,
        expression_cache,
        viewport,
        extensions,
        camera_sampler,
    } = preparation;
    let item = project.video_item(address)?;
    let sequence_position = project.timeline_time_to_sequence(&address.track(), position)?;
    let keyframe_time = project.keyframe_time(address, position)?;
    let evaluation =
        VisualEvaluation::for_item_with_audio(project, item, sequence_position, audio_analysis);
    let mut source_sizes = source_sizes(project);
    update_text_source_size(&mut source_sizes, item, &evaluation, expression_cache);
    let tracked_camera = item
        .tracking_camera_source()
        .filter(|source| {
            source.track_id != address.track_id()
                && project
                    .video_tracks
                    .iter()
                    .any(|track| track.id == source.track_id)
        })
        .and_then(|source| camera_sampler(item.id, source, evaluation.local_time()));
    let context = BuildContext::new(
        &evaluation,
        position,
        expression_cache,
        viewport,
        &source_sizes,
        item.id,
    )
    .tracked_camera(tracked_camera.as_ref())
    .extensions(extensions);
    let geometry = item.preview_geometry(&context)?;
    Some(PreparedGeometry {
        evaluation,
        keyframe_time,
        geometry,
        source_sizes,
        tracked_camera,
        item_id: item.id,
    })
}

pub fn prepare_snap_scene(
    project: &Project,
    selected: &ItemAddress,
    position: Time,
    viewport: PreviewViewport,
    source_sizes: &mut HashMap<uuid::Uuid, Vec2>,
    preparation: SnapPreparation<'_>,
) -> SnapScene {
    let mut scene = SnapScene::new(viewport, preparation.radius_px);
    if let Some(guides) = preparation.guides {
        scene.add_guides(&guides.vertical, &guides.horizontal);
    }
    let Some(tracks) = project.video_tracks_for_path(selected.sequence_path()) else {
        return scene;
    };
    for track in tracks.iter().filter(|track| track.enabled) {
        for item in &track.items {
            let address = ItemAddress::Video {
                sequence_path: selected.sequence_path().to_vec(),
                track_id: track.id,
                item_id: item.id,
            };
            if &address == selected {
                continue;
            }
            let Some((start, end)) = project.projected_item_times(&address) else {
                continue;
            };
            if position < start || position >= end {
                continue;
            }
            let Some(sequence_position) =
                project.timeline_time_to_sequence(&address.track(), position)
            else {
                continue;
            };
            let evaluation = VisualEvaluation::for_item_with_audio(
                project,
                item,
                sequence_position,
                preparation.audio_analysis,
            );
            if !shrimply_evaluation::resolve_bool(
                &item.visibility,
                &evaluation,
                &mut preparation.expression_cache.borrow_mut(),
            ) {
                continue;
            }
            update_text_source_size(
                source_sizes,
                item,
                &evaluation,
                preparation.expression_cache,
            );
            let context = BuildContext::new(
                &evaluation,
                position,
                preparation.expression_cache,
                viewport,
                source_sizes,
                item.id,
            )
            .extensions(Some(preparation.extensions));
            scene.add_provider(item, &context);
        }
    }
    scene
}

#[derive(Clone, Copy)]
pub struct BuildContext<'a> {
    evaluation: &'a VisualEvaluation,
    timeline_position: Time,
    expression_cache: &'a RefCell<TransformExpressionCache>,
    viewport: PreviewViewport,
    geometry: Option<PreviewItemGeometry>,
    source_sizes: &'a HashMap<uuid::Uuid, Vec2>,
    snap_scene: Option<&'a SnapScene>,
    tracked_camera: Option<&'a TrackedCameraPreview>,
    extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    item_id: uuid::Uuid,
}

impl<'a> BuildContext<'a> {
    pub fn new(
        evaluation: &'a VisualEvaluation,
        timeline_position: Time,
        expression_cache: &'a RefCell<TransformExpressionCache>,
        viewport: PreviewViewport,
        source_sizes: &'a HashMap<uuid::Uuid, Vec2>,
        item_id: uuid::Uuid,
    ) -> Self {
        Self {
            evaluation,
            timeline_position,
            expression_cache,
            viewport,
            geometry: None,
            source_sizes,
            snap_scene: None,
            tracked_camera: None,
            extensions: None,
            item_id,
        }
    }

    pub fn geometry(mut self, geometry: PreviewItemGeometry) -> Self {
        self.geometry = Some(geometry);
        self
    }

    pub fn snapping(mut self, snap_scene: Option<&'a SnapScene>) -> Self {
        self.snap_scene = snap_scene;
        self
    }

    pub fn tracked_camera(mut self, tracked_camera: Option<&'a TrackedCameraPreview>) -> Self {
        self.tracked_camera = tracked_camera;
        self
    }

    pub fn extensions(
        mut self,
        extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    ) -> Self {
        self.extensions = extensions;
        self
    }

    pub fn evaluation(&self) -> &'a VisualEvaluation {
        self.evaluation
    }

    pub fn expression_cache(&self) -> &'a RefCell<TransformExpressionCache> {
        self.expression_cache
    }

    pub fn source_sizes(&self) -> &'a HashMap<uuid::Uuid, Vec2> {
        self.source_sizes
    }

    pub fn with_source_sizes<'b>(
        self,
        source_sizes: &'b HashMap<uuid::Uuid, Vec2>,
    ) -> BuildContext<'b>
    where
        'a: 'b,
    {
        BuildContext {
            evaluation: self.evaluation,
            timeline_position: self.timeline_position,
            expression_cache: self.expression_cache,
            viewport: self.viewport,
            geometry: self.geometry,
            source_sizes,
            snap_scene: self.snap_scene,
            tracked_camera: self.tracked_camera,
            extensions: self.extensions,
            item_id: self.item_id,
        }
    }
}

impl PreviewContext for BuildContext<'_> {
    fn timeline_position(&self) -> Time {
        self.timeline_position
    }

    fn local_time(&self) -> Time {
        self.evaluation.local_time()
    }

    fn viewport(&self) -> PreviewViewport {
        self.viewport
    }

    fn selection_color(&self) -> Color {
        Color::BLUE5
    }

    fn target_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        self.geometry
    }

    fn source_size(&self, item_id: uuid::Uuid) -> Option<Vec2> {
        self.source_sizes.get(&item_id).copied()
    }

    fn item_geometry(&self, item_id: uuid::Uuid) -> Option<PreviewItemGeometry> {
        (item_id == self.item_id).then_some(self.geometry).flatten()
    }

    fn snapping(&self) -> Option<&SnapScene> {
        self.snap_scene
    }

    fn extension(&self, _target: PreviewTarget, key: PreviewExtensionKey) -> Option<&dyn Any> {
        if key == shrimply_project::project::TRACKED_CAMERA_PREVIEW {
            return self.tracked_camera.map(|camera| camera as &dyn Any);
        }
        self.extensions?.get(&key).map(|value| value.as_ref())
    }
}

impl PreviewBuilder for BuildContext<'_> {
    fn resolve<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>) -> T {
        shrimply_evaluation::resolve(
            value,
            self.evaluation,
            &mut self.expression_cache.borrow_mut(),
        )
    }

    fn resolve_at<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>, time: Time) -> T {
        shrimply_evaluation::resolve(
            value,
            &self.evaluation.at_local_time(time),
            &mut self.expression_cache.borrow_mut(),
        )
    }
}

pub fn source_sizes(project: &Project) -> HashMap<uuid::Uuid, Vec2> {
    let fallback = Vec2::new(
        project.canvas_size.width.max(1) as f32,
        project.canvas_size.height.max(1) as f32,
    );
    project
        .video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.video_tracks)
                .flat_map(|track| &track.items),
        )
        .map(|item| {
            let size = Vec2::new(item.source_width as f32, item.source_height as f32);
            (
                item.id,
                if size.min_element() > 0.0 {
                    size
                } else {
                    fallback
                },
            )
        })
        .collect()
}

pub fn update_text_source_size(
    source_sizes: &mut HashMap<uuid::Uuid, Vec2>,
    item: &shrimply_project::project::VideoItem,
    evaluation: &VisualEvaluation,
    expression_cache: &RefCell<TransformExpressionCache>,
) {
    let VideoItemContent::Text(text) = &item.content else {
        return;
    };
    let mut expressions = expression_cache.borrow_mut();
    let content = shrimply_evaluation::resolve_text(&text.text, evaluation, &mut expressions);
    let font_size =
        shrimply_evaluation::resolve_scalar(&text.font_size, evaluation, &mut expressions).max(1.0);
    let font_weight =
        shrimply_evaluation::resolve_scalar(&text.font_weight, evaluation, &mut expressions);
    let tracking =
        shrimply_evaluation::resolve_scalar(&text.tracking, evaluation, &mut expressions);
    let line_height =
        shrimply_evaluation::resolve_scalar(&text.line_height, evaluation, &mut expressions)
            .max(f32::EPSILON);
    source_sizes.insert(
        item.id,
        shrimply_video_core::text_layout::layout(
            text,
            &content,
            font_size,
            font_weight,
            tracking,
            line_height,
            evaluation.local_time(),
        )
        .size
        .max(Vec2::ONE),
    );
}
