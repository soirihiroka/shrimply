use crate::provider::{
    BuildContext, CameraSampler, GeometryPreparation, SnapPreparation, prepare_geometry,
    prepare_snap_scene, update_text_source_size,
};
use glam::Vec2 as GlamVec2;
use shrimply_evaluation::{FrameAudioAnalysis, TransformExpressionCache, VisualEvaluation};
use shrimply_preview_core::{
    KeyboardEvent, PointerEvent, PreviewEditSink, PreviewExtensionKey, PreviewItemGeometry,
    PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget, PreviewViewport, SnapScene,
};
use shrimply_project::project::{ItemAddress, PreviewGuides, Project, Time};
use std::{any::Any, cell::RefCell, collections::HashMap};

pub struct PreparedProvider {
    pub item: ItemAddress,
    pub target: PreviewTarget,
    pub project_revision: u64,
    pub context: PreparedContext,
    pub provider: Box<dyn PreviewProvider>,
    pub deferred_refresh: PreviewRefresh,
}

pub struct PreparedContext {
    pub evaluation: VisualEvaluation,
    pub timeline_position: Time,
    pub keyframe_time: Time,
    pub viewport: PreviewViewport,
    pub geometry: PreviewItemGeometry,
    pub source_sizes: HashMap<uuid::Uuid, GlamVec2>,
    pub snap_scene: Option<SnapScene>,
    pub tracked_camera: Option<shrimply_project::project::TrackedCameraPreview>,
    pub item_id: uuid::Uuid,
}

impl PreparedContext {
    pub fn context<'a>(
        &'a self,
        expression_cache: &'a RefCell<TransformExpressionCache>,
        extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    ) -> BuildContext<'a> {
        BuildContext::new(
            &self.evaluation,
            self.timeline_position,
            expression_cache,
            self.viewport,
            &self.source_sizes,
            self.item_id,
        )
        .geometry(self.geometry)
        .snapping(self.snap_scene.as_ref())
        .tracked_camera(self.tracked_camera.as_ref())
        .extensions(extensions)
    }
}

pub struct Edits<'a> {
    pub project: &'a mut Project,
    pub extensions: &'a mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
    pub item: &'a ItemAddress,
    pub keyframe_time: Time,
    pub context: BuildContext<'a>,
}

impl PreviewEditSink for Edits<'_> {
    fn keyframe_time(&self) -> Time {
        self.keyframe_time
    }

    fn target_mut(&mut self, target: PreviewTarget) -> &mut dyn Any {
        self.project
            .preview_target_mut(target)
            .expect("preview target is missing")
    }

    fn updated_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        let item = self.project.video_item(self.item)?;
        let mut source_sizes = self.context.source_sizes().clone();
        update_text_source_size(
            &mut source_sizes,
            item,
            self.context.evaluation(),
            self.context.expression_cache(),
        );
        item.preview_geometry(&self.context.with_source_sizes(&source_sizes))
    }

    fn extension_mut(
        &mut self,
        _target: PreviewTarget,
        key: PreviewExtensionKey,
    ) -> Option<&mut dyn Any> {
        self.extensions.get_mut(&key).map(|value| value.as_mut())
    }
}

#[derive(Clone, Copy)]
pub struct Preparation<'a> {
    pub project_revision: u64,
    pub viewport: PreviewViewport,
    pub audio_analysis: &'a FrameAudioAnalysis,
    pub expression_cache: &'a RefCell<TransformExpressionCache>,
    pub snap_enabled: bool,
    pub snap_radius_px: f32,
    pub guides: Option<&'a PreviewGuides>,
    pub camera_sampler: CameraSampler,
}

pub fn prepare_target(
    project: &Project,
    key: &ItemAddress,
    target: PreviewTarget,
    position: Time,
    preparation: Preparation<'_>,
    extensions: &HashMap<PreviewExtensionKey, Box<dyn Any>>,
) -> Option<PreparedProvider> {
    let item = project.video_item(key)?;
    if !item.owns_preview_target(target) {
        return None;
    }
    let viewport = preparation.viewport;
    let mut prepared = prepare_geometry(
        project,
        key,
        position,
        GeometryPreparation {
            audio_analysis: preparation.audio_analysis,
            expression_cache: preparation.expression_cache,
            viewport,
            extensions: Some(extensions),
            camera_sampler: preparation.camera_sampler,
        },
    )?;
    let snap_scene = preparation.snap_enabled.then(|| {
        prepare_snap_scene(
            project,
            key,
            position,
            viewport,
            &mut prepared.source_sizes,
            SnapPreparation {
                audio_analysis: preparation.audio_analysis,
                expression_cache: preparation.expression_cache,
                extensions,
                guides: preparation.guides,
                radius_px: preparation.snap_radius_px,
            },
        )
    });
    let context = prepared
        .context(position, preparation.expression_cache, viewport)
        .snapping(snap_scene.as_ref())
        .extensions(Some(extensions));
    let provider = item.preview_provider(target, &context)?;
    Some(PreparedProvider {
        item: key.clone(),
        target,
        project_revision: preparation.project_revision,
        context: PreparedContext {
            evaluation: prepared.evaluation,
            timeline_position: position,
            keyframe_time: prepared.keyframe_time,
            viewport,
            geometry: prepared.geometry,
            source_sizes: prepared.source_sizes,
            snap_scene,
            tracked_camera: prepared.tracked_camera,
            item_id: prepared.item_id,
        },
        provider,
        deferred_refresh: PreviewRefresh::NONE,
    })
}

impl PreparedProvider {
    pub fn pointer(
        &mut self,
        project: &mut Project,
        extensions: &mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
        expression_cache: &RefCell<TransformExpressionCache>,
        event: PointerEvent<'_>,
    ) -> PreviewResponse {
        let context = self.context.context(expression_cache, None);
        let mut response = self.provider.on_pointer(
            event,
            &context,
            &mut Edits {
                project,
                extensions,
                item: &self.item,
                keyframe_time: self.context.keyframe_time,
                context,
            },
        );
        if matches!(event, PointerEvent::Cancel) {
            response.edit = response.edit.canceled();
        }
        let terminal = matches!(event, PointerEvent::End(_) | PointerEvent::Cancel);
        if response.edit.changed() && !response.edit.commits() && !terminal {
            self.deferred_refresh |= response.edit.refresh;
        } else if response.edit.commits() || terminal {
            response.edit.refresh |= self.deferred_refresh;
            self.deferred_refresh = PreviewRefresh::NONE;
        }
        response
    }

    pub fn keyboard(
        &mut self,
        project: &mut Project,
        extensions: &mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
        expression_cache: &RefCell<TransformExpressionCache>,
        event: KeyboardEvent,
    ) -> PreviewResponse {
        let context = self.context.context(expression_cache, None);
        self.provider.on_keyboard(
            event,
            &context,
            &mut Edits {
                project,
                extensions,
                item: &self.item,
                keyframe_time: self.context.keyframe_time,
                context,
            },
        )
    }

    pub fn cancel(
        &mut self,
        project: &mut Project,
        extensions: &mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
        expression_cache: &RefCell<TransformExpressionCache>,
    ) -> PreviewResponse {
        let context = self.context.context(expression_cache, None);
        let mut response = self.provider.on_cancel(
            &context,
            &mut Edits {
                project,
                extensions,
                item: &self.item,
                keyframe_time: self.context.keyframe_time,
                context,
            },
        );
        response.edit.refresh |= self.deferred_refresh;
        self.deferred_refresh = PreviewRefresh::NONE;
        response.edit = response.edit.canceled();
        response
    }

    pub fn project_committed(&mut self, revision: u64) -> bool {
        self.project_revision = revision;
        self.provider.on_project_committed(revision);
        !self.provider.keeps_frame_until_base()
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum PointerSequence {
    #[default]
    Idle,
    Active,
    Guide,
    Suppressed,
}

#[derive(Default)]
pub struct Controller {
    pub provider: Option<PreparedProvider>,
    pub retiring_provider: Option<PreparedProvider>,
    pub extensions: HashMap<PreviewExtensionKey, Box<dyn Any>>,
    pub sequence: PointerSequence,
    pub context_invalidated: bool,
    pub frame_pending: bool,
    pub live_base_pending: bool,
    pub live_base_in_flight: Option<u64>,
    pub base_exclusion: Option<uuid::Uuid>,
    pub presented_base_exclusion: Option<uuid::Uuid>,
    snap_configuration: Option<SnapConfiguration>,
    audio_analysis: Option<FrameAudioAnalysis>,
}

struct SnapConfiguration {
    enabled: bool,
    radius_px: f32,
    guides: Option<Box<PreviewGuides>>,
}

impl SnapConfiguration {
    fn matches(&self, preparation: Preparation<'_>) -> bool {
        self.enabled == preparation.snap_enabled
            && self.radius_px == preparation.snap_radius_px
            && match (self.guides.as_deref(), preparation.guides) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    before.vertical == after.vertical && before.horizontal == after.horizontal
                }
                _ => false,
            }
    }
}

impl Controller {
    pub fn ensure(
        &mut self,
        project: &Project,
        selection: Option<(&ItemAddress, PreviewTarget)>,
        position: Time,
        preparation: Preparation<'_>,
    ) -> Result<bool, String> {
        if self.sequence != PointerSequence::Idle {
            return Ok(self.provider.is_some());
        }
        let stale = self.context_invalidated
            || self
                .snap_configuration
                .as_ref()
                .is_some_and(|config| !config.matches(preparation))
            || self
                .audio_analysis
                .as_ref()
                .is_some_and(|analysis| !analysis.same_frame(preparation.audio_analysis))
            || self.provider.as_ref().is_some_and(|prepared| {
                selection != Some((&prepared.item, prepared.target))
                    || prepared.project_revision != preparation.project_revision
                    || prepared.context.timeline_position != position
                    || prepared.context.viewport != preparation.viewport
            });
        if stale {
            self.provider = None;
            self.context_invalidated = false;
        }
        if self.provider.is_none()
            && let Some((address, target)) = selection
        {
            self.snap_configuration = Some(SnapConfiguration {
                enabled: preparation.snap_enabled,
                radius_px: preparation.snap_radius_px,
                guides: preparation.guides.map(|guides| Box::new(guides.clone())),
            });
            self.audio_analysis = Some(preparation.audio_analysis.clone());
            let audio_analysis = preparation.audio_analysis.for_preparation();
            let provider = prepare_target(
                project,
                address,
                target,
                position,
                Preparation {
                    audio_analysis: &audio_analysis,
                    ..preparation
                },
                &self.extensions,
            );
            let failures = audio_analysis.failures();
            if !failures.is_empty() {
                return Err(failures.join("\n"));
            }
            if !audio_analysis.pending() {
                self.provider = provider;
            }
        }
        Ok(self.provider.is_some())
    }

    pub fn accepts_pointer(&mut self, event: PointerEvent<'_>) -> bool {
        match event {
            PointerEvent::Begin(_) if self.sequence != PointerSequence::Idle => false,
            PointerEvent::Samples { .. } if self.sequence != PointerSequence::Active => false,
            PointerEvent::End(_) | PointerEvent::Cancel
                if self.sequence == PointerSequence::Suppressed =>
            {
                self.sequence = PointerSequence::Idle;
                if self.context_invalidated {
                    self.provider = None;
                    self.context_invalidated = false;
                }
                false
            }
            PointerEvent::End(_) | PointerEvent::Cancel => self.sequence == PointerSequence::Active,
            _ => true,
        }
    }

    pub fn pointer(
        &mut self,
        project: &mut Project,
        expression_cache: &RefCell<TransformExpressionCache>,
        event: PointerEvent<'_>,
    ) -> PreviewResponse {
        if !self.accepts_pointer(event) {
            return PreviewResponse::IGNORED;
        }
        let Some(prepared) = self.provider.as_mut() else {
            if matches!(event, PointerEvent::Begin(_)) {
                self.sequence = PointerSequence::Suppressed;
            }
            return PreviewResponse::IGNORED;
        };
        if matches!(event, PointerEvent::Begin(_)) {
            self.sequence = PointerSequence::Active;
            self.live_base_pending = false;
            self.live_base_in_flight = None;
        }
        let terminal = matches!(event, PointerEvent::End(_) | PointerEvent::Cancel);
        let mut response = prepared.pointer(project, &mut self.extensions, expression_cache, event);
        if response.edit.changed() && !response.edit.commits() && !terminal {
            self.live_base_pending |= response.edit.refresh.contains(PreviewRefresh::PREVIEW);
            response.edit.refresh = PreviewRefresh::NONE;
        } else if response.edit.commits() || terminal {
            self.live_base_pending = false;
        }
        self.frame_pending |= response.redraw;
        if terminal {
            self.sequence = PointerSequence::Idle;
        }
        response
    }

    pub fn keyboard(
        &mut self,
        project: &mut Project,
        expression_cache: &RefCell<TransformExpressionCache>,
        event: KeyboardEvent,
    ) -> PreviewResponse {
        let Some(prepared) = self.provider.as_mut() else {
            return PreviewResponse::IGNORED;
        };
        let response = prepared.keyboard(project, &mut self.extensions, expression_cache, event);
        self.frame_pending |= response.redraw;
        response
    }

    pub fn cancel(
        &mut self,
        project: &mut Project,
        expression_cache: &RefCell<TransformExpressionCache>,
    ) -> PreviewResponse {
        self.sequence = if self.sequence == PointerSequence::Active {
            PointerSequence::Suppressed
        } else {
            PointerSequence::Idle
        };
        self.context_invalidated = false;
        self.live_base_pending = false;
        let Some(mut prepared) = self.provider.take() else {
            return PreviewResponse::IGNORED;
        };
        let response = prepared.cancel(project, &mut self.extensions, expression_cache);
        self.retiring_provider = prepared
            .provider
            .base_frame_exclusion()
            .is_some()
            .then_some(prepared);
        self.frame_pending |= response.redraw;
        response
    }

    pub fn project_committed(&mut self, revision: u64) {
        if let Some(provider) = self.provider.as_mut() {
            self.context_invalidated = provider.project_committed(revision);
        }
    }

    pub fn accept_base_frame(
        &mut self,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
    ) -> bool {
        if excluded_item_id != self.base_exclusion {
            return false;
        }
        self.presented_base_exclusion = excluded_item_id;
        self.retiring_provider = None;
        if self
            .live_base_in_flight
            .is_some_and(|requested| revision >= requested)
        {
            self.live_base_in_flight = None;
        }
        if let Some(provider) = self.provider.as_mut() {
            provider.provider.on_base_frame_presented(revision);
            self.context_invalidated |=
                self.sequence == PointerSequence::Idle && revision >= provider.project_revision;
        }
        true
    }

    pub fn take_live_base_request(&mut self) -> bool {
        if self.live_base_pending && self.live_base_in_flight.is_none() {
            self.live_base_pending = false;
            true
        } else {
            false
        }
    }

    pub fn live_base_requested(&mut self, revision: u64) {
        self.live_base_in_flight = Some(revision);
        if let Some(provider) = self.provider.as_mut() {
            provider.project_revision = revision;
        }
    }

    pub fn draw(
        &mut self,
        canvas: &shrimply_preview_core::PreviewCanvas,
        expression_cache: &RefCell<TransformExpressionCache>,
    ) {
        let current_covers_base = self.provider.as_ref().is_some_and(|prepared| {
            prepared.provider.base_frame_exclusion() == self.presented_base_exclusion
        });
        if !current_covers_base
            && let Some(prepared) = self.retiring_provider.as_mut()
            && prepared.provider.base_frame_exclusion() == self.presented_base_exclusion
        {
            let context = prepared
                .context
                .context(expression_cache, Some(&self.extensions));
            prepared.provider.on_draw(canvas, &context);
        }
        if let Some(prepared) = self.provider.as_mut()
            && prepared.provider.base_frame_exclusion() == self.presented_base_exclusion
        {
            let context = prepared
                .context
                .context(expression_cache, Some(&self.extensions));
            prepared.provider.on_draw(canvas, &context);
        }
    }
}
