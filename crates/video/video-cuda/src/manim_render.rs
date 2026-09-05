use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

use hashbrown::HashMap;
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_manim_parser::{
    CompiledAnimation, Progress, ProgressStage, Settings, compile, reflected_parameters,
};
use shrimply_manim_wgpu::PreparedAnimation;
use shrimply_math_color::Color;
use shrimply_math_core::Fraction;
use shrimply_project::project::{
    CanvasSize, ManimParameter, ManimParameterValue, Time, VideoItem, VideoItemContent,
};
use uuid::Uuid;

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

const LOADING_COLOR: Color<u8> = Color::new(24, 108, 120, 255);
const LOADING_PROGRESS_COLOR: Color<u8> = Color::new(99, 230, 221, 255);

pub struct ManimElement {
    file: Asset,
    snapshot: AssetSnapshot,
    scene: String,
    parameters: HashMap<String, ManimParameterValue>,
    canvas_size: CanvasSize,
    compiled_fps: Option<Fraction>,
    render_slot: Arc<()>,
    frame: Option<Rc<VisualFrame>>,
    rendered_frame: Option<usize>,
    placeholder_progress: Option<Progress>,
    duration_reported: bool,
    parameters_reported: bool,
    first_frame_reported: bool,
    state: CompileState,
}

enum CompileState {
    Idle,
    Loading {
        cancelled: Arc<AtomicBool>,
        receiver: Receiver<CompileEvent>,
        progress: Option<Progress>,
        worker: Option<JoinHandle<()>>,
    },
    Ready {
        animation: Arc<CompiledAnimation>,
        prepared: Arc<PreparedAnimation>,
        parameters: Vec<ManimParameter>,
    },
    NeedsParameters {
        parameters: Vec<ManimParameter>,
    },
    Failed(String),
}

enum CompiledManim {
    Current {
        animation: Arc<CompiledAnimation>,
        prepared: Arc<PreparedAnimation>,
        parameters: Vec<ManimParameter>,
    },
    NeedsParameters(Vec<ManimParameter>),
}

enum CompileEvent {
    Progress(Progress),
    Finished(Result<CompiledManim, String>),
}

impl ManimElement {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize) -> Result<Self, String> {
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("Manim renderer received a non-Manim visual".to_string());
        };
        Ok(Self {
            file: item.file.clone(),
            snapshot: item.file.snapshot()?,
            scene: manim.scene.clone(),
            parameters: manim.parameters.clone(),
            canvas_size,
            compiled_fps: None,
            render_slot: Arc::new(()),
            frame: None,
            rendered_frame: None,
            placeholder_progress: None,
            duration_reported: false,
            parameters_reported: false,
            first_frame_reported: false,
            state: CompileState::Idle,
        })
    }

    fn settings(&self, fps: Fraction) -> Settings {
        Settings {
            source: self.file.clone(),
            scene: self.scene.clone(),
            width: self.canvas_size.width,
            height: self.canvas_size.height,
            fps,
            parameters: self.parameters.clone(),
        }
    }

    fn cancel_compile(&mut self) {
        if let CompileState::Loading {
            cancelled, worker, ..
        } = &mut self.state
        {
            cancelled.store(true, Ordering::Release);
            if let Some(worker) = worker.take() {
                worker.join().expect("Manim compiler thread panicked");
            }
        }
    }

    fn loading_placeholder(
        &mut self,
        compositor: &mut CudaVideoCompositor,
        state: crate::layer::VisualState,
    ) -> Result<VisualRender, String> {
        let progress = match &self.state {
            CompileState::Loading { progress, .. } => *progress,
            _ => None,
        };
        let refresh = self.frame.is_none() || self.placeholder_progress != progress;
        if self.frame.is_none() {
            self.frame = Some(Rc::new(compositor.allocate_cached_rgba_layer(
                self.canvas_size.width,
                self.canvas_size.height,
                "persistent Manim output",
            )?));
        }
        let frame = self
            .frame
            .as_ref()
            .expect("Manim preview image was allocated")
            .clone();
        compositor.prepare_host_backed_frame(&frame, "persistent Manim output")?;
        if !refresh {
            return Ok(VisualRender::LoadingPlaceholder(Visual::Raster(
                RasterVisual::materialized(GpuFrame::Rgba(frame), state),
            )));
        }
        let counter = match progress {
            Some(Progress {
                stage: ProgressStage::StreamingFrames,
                completed,
                total,
            }) if total > 0 => shrimply_i18n_core::text_args(
                "Frame %{completed} / %{total}",
                &[
                    ("completed", completed.to_string()),
                    ("total", total.to_string()),
                ],
            ),
            Some(Progress {
                stage: ProgressStage::StreamingFrames,
                completed,
                ..
            }) => shrimply_i18n_core::text_args(
                "Frame %{completed}",
                &[("completed", completed.to_string())],
            ),
            Some(Progress {
                stage: ProgressStage::LoadingScene,
                ..
            }) => shrimply_i18n_core::text("Loading scene…").into_owned(),
            None => shrimply_i18n_core::text("Starting Manim…").into_owned(),
        };
        let pixels = shrimply_loading_screen::render(
            self.canvas_size.width,
            self.canvas_size.height,
            &counter,
            LOADING_COLOR,
            LOADING_PROGRESS_COLOR,
        )?;
        compositor.upload_rgba_layer_into(&frame, &pixels)?;
        self.rendered_frame = None;
        self.placeholder_progress = progress;
        Ok(VisualRender::LoadingPlaceholder(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(frame), state),
        )))
    }
}

impl VisualElement for ManimElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        matches!(item.content, VideoItemContent::Manim(_)) && self.canvas_size == canvas_size
    }

    fn take_source_duration(&mut self) -> Option<Time> {
        if self.duration_reported {
            return None;
        }
        let CompileState::Ready { animation, .. } = &self.state else {
            return None;
        };
        self.duration_reported = true;
        Some(Time {
            seconds: animation.scene().duration,
        })
    }

    fn take_manim_parameters(&mut self) -> Option<(Vec<ManimParameter>, bool)> {
        if self.parameters_reported {
            return None;
        }
        let (parameters, render_is_current) = match &self.state {
            CompileState::Ready { parameters, .. } => (parameters, true),
            CompileState::NeedsParameters { parameters } => (parameters, false),
            _ => return None,
        };
        self.parameters_reported = true;
        Some((parameters.clone(), render_is_current))
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let VideoItemContent::Manim(manim) = &request.item.content else {
            return Err("Manim renderer received a non-Manim visual".to_string());
        };
        if self.file != request.item.file
            || !self.snapshot.is_current()
            || self.scene != manim.scene
            || self.parameters != manim.parameters
        {
            self.cancel_compile();
            self.file = request.item.file.clone();
            self.snapshot = request.item.file.snapshot()?;
            self.scene.clone_from(&manim.scene);
            self.parameters.clone_from(&manim.parameters);
            self.placeholder_progress = None;
            self.rendered_frame = None;
            self.duration_reported = false;
            self.parameters_reported = false;
            self.first_frame_reported = false;
            self.state = CompileState::Idle;
        }
        let Some(source_time) =
            shrimply_project::project::generated_source_time_at(request.item, request.position)
        else {
            return Ok(VisualRender::Empty);
        };
        let fps = if request.item.playback_fps == shrimply_project::project::native_playback_fps() {
            request.project.fps
        } else {
            request.item.playback_fps
        };
        if self.compiled_fps != Some(fps) {
            self.cancel_compile();
            self.compiled_fps = Some(fps);
            self.placeholder_progress = None;
            self.rendered_frame = None;
            self.duration_reported = false;
            self.parameters_reported = false;
            self.first_frame_reported = false;
            self.state = CompileState::Idle;
        }
        if matches!(self.state, CompileState::Idle) {
            let settings = self.settings(fps);
            let (sender, receiver) = mpsc::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let compiler_cancelled = cancelled.clone();
            tracing::info!(
                source = %settings.source.path().display(),
                scene = %settings.scene,
                parameters = settings.parameters.len(),
                parameter_values = ?settings.parameters,
                "Manim render requested",
            );
            let worker = thread::spawn(move || {
                let source = settings.source.path().to_path_buf();
                let scene = settings.scene.clone();
                let started = std::time::Instant::now();
                let progress_sender = sender.clone();
                let result = compile(&settings, &compiler_cancelled, move |progress| {
                    let _ = progress_sender.send(CompileEvent::Progress(progress));
                })
                .and_then(|animation| {
                    let compiled_at = std::time::Instant::now();
                    let parameters = reflected_parameters(&animation)?;
                    let parameters_at = std::time::Instant::now();
                    if !animation.scene().render_is_current {
                        return Ok(CompiledManim::NeedsParameters(parameters));
                    }
                    let prepared = Arc::new(PreparedAnimation::new(animation.clone())?);
                    let prepared_at = std::time::Instant::now();
                    tracing::info!(
                        source = %source.display(),
                        %scene,
                        frames = animation.frames().len(),
                        compile_ms = compiled_at.duration_since(started).as_millis(),
                        parameter_decode_ms = parameters_at.duration_since(compiled_at).as_millis(),
                        validation_ms = prepared_at.duration_since(parameters_at).as_millis(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "Manim WGPU preparation finished",
                    );
                    Ok(CompiledManim::Current {
                        animation,
                        prepared,
                        parameters,
                    })
                });
                match &result {
                    Ok(CompiledManim::Current { .. }) => {}
                    Ok(CompiledManim::NeedsParameters(parameters)) => tracing::info!(
                        source = %source.display(),
                        %scene,
                        parameters = parameters.len(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "Manim animation discovered missing parameter values"
                    ),
                    Err(error) => tracing::error!(
                        source = %source.display(),
                        %scene,
                        elapsed_ms = started.elapsed().as_millis(),
                        %error,
                        "Manim animation compilation or WGPU preparation failed"
                    ),
                }
                let _ = sender.send(CompileEvent::Finished(result));
            });
            self.state = CompileState::Loading {
                cancelled,
                receiver,
                progress: None,
                worker: Some(worker),
            };
            return self.loading_placeholder(compositor, request.state);
        }

        let finished = match &mut self.state {
            CompileState::Loading {
                receiver, progress, ..
            } => loop {
                match receiver.try_recv() {
                    Ok(CompileEvent::Progress(next)) => *progress = Some(next),
                    Ok(CompileEvent::Finished(result)) => break Some(result),
                    Err(TryRecvError::Empty) => break None,
                    Err(TryRecvError::Disconnected) => {
                        break Some(Err(
                            "Manim compiler stopped without returning an animation".to_string()
                        ));
                    }
                }
            },
            _ => None,
        };
        if let Some(finished) = finished {
            if let CompileState::Loading { worker, .. } = &mut self.state
                && let Some(worker) = worker.take()
            {
                worker.join().expect("Manim compiler thread panicked");
            }
            self.placeholder_progress = None;
            self.state = match finished {
                Ok(CompiledManim::Current {
                    animation,
                    prepared,
                    parameters,
                }) => CompileState::Ready {
                    animation,
                    prepared,
                    parameters,
                },
                Ok(CompiledManim::NeedsParameters(parameters)) => {
                    CompileState::NeedsParameters { parameters }
                }
                Err(error) => CompileState::Failed(error),
            };
        }

        let CompileState::Ready {
            animation,
            prepared,
            ..
        } = &self.state
        else {
            if let CompileState::Failed(error) = &self.state {
                return Err(error.clone());
            }
            return self.loading_placeholder(compositor, request.state);
        };
        if animation.frames().is_empty() {
            return Err("compiled Manim animation contains no frames".to_string());
        }
        let frame_index = animation
            .frames()
            .partition_point(|frame| frame.time <= source_time.seconds)
            .saturating_sub(1);
        let render_started = std::time::Instant::now();
        let layer = {
            let _measurement = shrimply_benchmarking::measure("Manim / WGPU");
            let frame = self
                .frame
                .as_ref()
                .expect("Manim preview image was allocated while loading");
            compositor.prepare_host_backed_frame(frame, "persistent Manim output")?;
            if self.rendered_frame != Some(frame_index) {
                compositor.render_manim(&self.render_slot, prepared, frame_index, frame)?;
                self.rendered_frame = Some(frame_index);
            }
            frame.clone()
        };
        if !self.first_frame_reported {
            tracing::info!(
                source = %self.file.path().display(),
                scene = %self.scene,
                frame = frame_index,
                elapsed_ms = render_started.elapsed().as_millis(),
                "Manim first frame displayed",
            );
            self.first_frame_reported = true;
        }
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), request.state),
        )))
    }
}

impl Drop for ManimElement {
    fn drop(&mut self) {
        self.cancel_compile();
    }
}
