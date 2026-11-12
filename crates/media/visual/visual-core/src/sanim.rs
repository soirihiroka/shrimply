//! Shared native Sanim source for all preview/export backends.
pub use sanim_core::Viewport;
use sanim_runtime::{
    Runtime,
    sanim_render::{Options, Recording, Renderer, Scene},
};
use shrimply_asset::Asset;
use shrimply_project_document::project::{
    CanvasSize, Time, VideoItem, VideoItemContent, generated_source_time_with_duration,
};
use std::{collections::BTreeMap, sync::Arc};

pub enum Status {
    Empty,
    Loading(Arc<Recording>),
    Failed(Arc<Recording>),
    Ready(Arc<Recording>),
}

mod feedback;
pub mod reflection;
#[cfg(test)]
mod tests;
pub struct Source {
    file: Asset,
    parameters: BTreeMap<String, serde_json::Value>,
    runtime: Runtime,
    observation: Arc<sanim_runtime::Observation>,
    published: bool,
    viewport: sanim_core::Viewport,
    waker: Option<crate::generated::SourceWake>,
    renderer: Renderer,
    scene: Option<(String, Scene)>,
    generation: u64,
    warned_revision: u64,
    error: Option<String>,
    export: Option<Arc<sanim_runtime::sanim_lang::Prepared>>,
    feedback: Option<feedback::Feedback>,
}
impl Source {
    pub fn new(item: &VideoItem) -> Result<Self, String> {
        Self::with_viewport(item, sanim_core::Viewport::default())
    }
    pub fn with_viewport(item: &VideoItem, viewport: sanim_core::Viewport) -> Result<Self, String> {
        let VideoItemContent::Sanim(settings) = &item.content else {
            return Err("Sanim source received another clip type".into());
        };
        let runtime =
            Runtime::with_viewport(item.file.path(), viewport.clone(), Default::default())?;
        if !settings.parameters.is_empty() {
            runtime.set_inputs(settings.parameters.clone())?;
        }
        Ok(Self {
            file: item.file.clone(),
            parameters: settings.parameters.clone(),
            observation: Arc::new(runtime.observe()),
            runtime,
            published: false,
            viewport,
            waker: None,
            renderer: Renderer::default(),
            scene: None,
            generation: 0,
            warned_revision: 0,
            error: None,
            export: None,
            feedback: None,
        })
    }
    pub fn set_waker(&mut self, wake: Option<crate::generated::SourceWake>) {
        self.runtime.set_waker(wake.clone());
        self.waker = wake;
    }
    pub fn needs_update(&self) -> bool {
        let state = self.runtime.snapshot();
        state.generation != self.generation
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
    pub fn poll(
        &mut self,
        item: &VideoItem,
        position: Time,
        canvas: CanvasSize,
        rate: sanim_core::Fraction,
        strict: bool,
    ) -> Result<Status, String> {
        if !strict {
            self.export = None;
        }
        if self.file != item.file {
            let wake = self.waker.clone();
            *self = Self::with_viewport(item, self.viewport.clone())?;
            self.set_waker(wake);
        }
        let viewport = sanim_core::Viewport {
            width: canvas.width,
            height: canvas.height,
            rate,
            ..sanim_core::Viewport::default()
        };
        if self.viewport != viewport {
            self.export = None;
            self.runtime.set_viewport(viewport.clone())?;
            self.viewport = viewport;
        }
        let VideoItemContent::Sanim(settings) = &item.content else {
            return Err("Sanim source received another clip type".into());
        };
        if self.parameters != settings.parameters {
            self.export = None;
            self.runtime.set_inputs(settings.parameters.clone())?;
            self.parameters = settings.parameters.clone();
        }
        if !strict && !self.published {
            reflection::publish(item, &self.observation);
            self.published = true;
        }
        let pinned = self.export.as_ref().filter(|_| strict).cloned();
        let prepared = if let Some(prepared) = pinned {
            prepared
        } else {
            let mut state = self.runtime.snapshot();
            // Exports wait for preparation; interactive preview never blocks on compilation.
            if strict && state.preparing {
                const PREPARATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
                let deadline = std::time::Instant::now() + PREPARATION_TIMEOUT;
                while state.preparing && std::time::Instant::now() < deadline {
                    state = self.runtime.wait_for_change(
                        state.generation,
                        deadline.saturating_duration_since(std::time::Instant::now()),
                    );
                }
                if state.preparing {
                    return Err("Sanim preparation did not finish within 30 seconds".into());
                }
            }
            if state.revision != self.warned_revision
                && let Some(prepared) = &state.prepared
            {
                for warning in &prepared.performance {
                    tracing::warn!(code=%warning.code,source=%warning.source,line=warning.span.line,message=%warning.message,"Sanim preparation performance");
                }
                self.warned_revision = state.revision;
            }
            if state.generation != self.generation {
                self.generation = state.generation;
                self.error = state
                    .diagnostic
                    .as_ref()
                    .filter(|_| !state.preparing)
                    .map(ToString::to_string);
            }
            if strict && let Some(error) = &state.diagnostic {
                return Err(error.to_string());
            }
            let Some(prepared) = state.prepared.clone() else {
                let recording = self.feedback(canvas, &state, None)?;
                return if !state.preparing && state.diagnostic.is_some() {
                    Ok(Status::Failed(recording))
                } else {
                    Ok(Status::Loading(recording))
                };
            };
            if strict {
                self.export = Some(prepared.clone());
            }
            prepared
        };
        let Some(time) = generated_source_time_with_duration(
            item,
            position,
            Time {
                seconds: prepared.animation.duration,
            },
        ) else {
            return Ok(Status::Empty);
        };
        let key = format!(
            "{}:{}:{}",
            prepared.fingerprint, canvas.width, canvas.height
        );
        if self
            .scene
            .as_ref()
            .is_none_or(|(previous, _)| previous != &key)
        {
            self.scene = Some((
                key,
                self.renderer.prepare(
                    prepared
                        .animation_for_viewport(canvas.width, canvas.height)
                        .map_err(|e| e.to_string())?,
                )?,
            ));
        }
        let scene = &self.scene.as_ref().expect("prepared Sanim scene").1;
        let mut options = Options::for_animation(scene.animation());
        options.encode_png = false;
        options.width = canvas.width;
        options.height = canvas.height;
        (options.logical_width, options.logical_height) = scene.animation().logical_size();
        let frame = self.renderer.record(
            scene,
            time.seconds.min(prepared.animation.duration),
            &options,
        )?;
        if !strict {
            let state = self.runtime.snapshot();
            if !state.preparing && state.diagnostic.is_some() {
                return Ok(Status::Ready(self.feedback(canvas, &state, Some(frame))?));
            }
        }
        Ok(Status::Ready(frame))
    }
    fn feedback(
        &mut self,
        canvas: CanvasSize,
        state: &sanim_runtime::Snapshot,
        base: Option<Arc<Recording>>,
    ) -> Result<Arc<Recording>, String> {
        if let Some(previous) = &self.feedback
            && previous.generation == state.generation
            && previous.canvas == canvas
            && match (&previous.base, &base) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        {
            return Ok(previous.recording.clone());
        }
        let error = state
            .diagnostic
            .as_ref()
            .filter(|_| !state.preparing)
            .map(ToString::to_string);
        let recording =
            feedback::record(canvas, &state.progress, error.as_deref(), base.as_deref())?;
        self.feedback = Some(feedback::Feedback {
            generation: state.generation,
            canvas,
            base,
            recording: recording.clone(),
        });
        Ok(recording)
    }
}

pub struct RecordedVisual(pub Arc<Recording>);
impl crate::generated::GeneratedVisual for RecordedVisual {
    fn cache_key(&self) -> Option<(std::any::TypeId, u64)> {
        Some((std::any::TypeId::of::<Self>(), self.0.id()))
    }
    fn draw(
        &self,
        canvas: &skia_safe::Canvas,
        _evaluation: &shrimply_project_evaluation::TransformEvaluation,
        _expressions: &mut shrimply_project_evaluation::TransformExpressionCache,
        _path_effect: Option<&skia_safe::PathEffect>,
    ) {
        self.0.draw(canvas);
    }
}
