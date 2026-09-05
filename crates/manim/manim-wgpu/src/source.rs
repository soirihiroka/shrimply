use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::thread::{self, JoinHandle};

use shrimply_asset::AssetSnapshot;
use shrimply_manim_ir::CompiledAnimation;
use shrimply_manim_parser::{Progress, Settings, compile, reflected_parameters};
use shrimply_math_color::Color;
use shrimply_math_core::Fraction;
use shrimply_project::project::{
    CanvasSize, ManimParameter, ManimParameterValue, Time, VideoItem, VideoItemContent,
};

use crate::PreparedAnimation;

const LOADING_COLOR: Color<u8> = Color::new(24, 108, 120, 255);
const LOADING_PROGRESS_COLOR: Color<u8> = Color::new(99, 230, 221, 255);

pub struct CompiledFrame {
    pub prepared: Arc<PreparedAnimation>,
    pub frame_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    Loading {
        progress: Option<Progress>,
        changed: bool,
    },
    NeedsParameters,
}

pub struct Source {
    snapshot: AssetSnapshot,
    source_available: bool,
    scene: String,
    parameters: hashbrown::HashMap<String, ManimParameterValue>,
    canvas_size: CanvasSize,
    fps: Fraction,
    duration_reported: bool,
    parameters_reported: bool,
    loading_progress: Option<Option<Progress>>,
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
    NeedsParameters(Vec<ManimParameter>),
    Failed(String),
}

enum Compiled {
    Ready {
        animation: Arc<CompiledAnimation>,
        prepared: Arc<PreparedAnimation>,
        parameters: Vec<ManimParameter>,
    },
    NeedsParameters(Vec<ManimParameter>),
}

enum CompileEvent {
    Progress(Progress),
    Finished(Result<Compiled, String>),
}

impl Source {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize, fps: Fraction) -> Result<Self, String> {
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("Manim source received a non-Manim visual".into());
        };
        Ok(Self {
            snapshot: item.file.snapshot()?,
            source_available: true,
            scene: manim.scene.clone(),
            parameters: manim.parameters.clone(),
            canvas_size,
            fps,
            duration_reported: false,
            parameters_reported: false,
            loading_progress: None,
            state: CompileState::Idle,
        })
    }

    pub fn poll(
        &mut self,
        item: &VideoItem,
        canvas_size: CanvasSize,
        fps: Fraction,
        position: Time,
    ) -> Result<Result<CompiledFrame, SourceStatus>, String> {
        self.update_source(item, canvas_size, fps)?;
        let source_time = shrimply_project::project::generated_source_time_at(item, position)
            .ok_or("Manim source time is outside the clip")?;
        if matches!(self.state, CompileState::Idle) {
            self.start_compile(item);
            self.loading_progress = Some(None);
            return Ok(Err(SourceStatus::Loading {
                progress: None,
                changed: true,
            }));
        }
        self.receive_compile();
        match &self.state {
            CompileState::Idle => unreachable!("Manim compilation was started"),
            CompileState::Loading { progress, .. } => {
                let changed = self.loading_progress != Some(*progress);
                self.loading_progress = Some(*progress);
                Ok(Err(SourceStatus::Loading {
                    progress: *progress,
                    changed,
                }))
            }
            CompileState::NeedsParameters(_) => Ok(Err(SourceStatus::NeedsParameters)),
            CompileState::Failed(error) => Err(error.clone()),
            CompileState::Ready {
                animation,
                prepared,
                ..
            } => {
                if animation.frames().is_empty() {
                    return Err("compiled Manim animation contains no frames".into());
                }
                let frame_index = animation
                    .frames()
                    .partition_point(|frame| frame.time <= source_time.seconds)
                    .saturating_sub(1);
                Ok(Ok(CompiledFrame {
                    prepared: prepared.clone(),
                    frame_index,
                }))
            }
        }
    }

    pub fn loading(&self) -> bool {
        matches!(
            self.state,
            CompileState::Idle | CompileState::Loading { .. }
        )
    }

    pub fn source_revision(&self) -> u64 {
        if self.source_available {
            self.snapshot.revision()
        } else {
            0
        }
    }

    pub fn scene(&self) -> &str {
        &self.scene
    }

    pub fn input_parameters(&self) -> &hashbrown::HashMap<String, ManimParameterValue> {
        &self.parameters
    }

    pub fn take_duration(&mut self) -> Option<Time> {
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

    pub fn take_parameters(&mut self) -> Option<(Vec<ManimParameter>, bool)> {
        if self.parameters_reported {
            return None;
        }
        let (parameters, current) = match &self.state {
            CompileState::Ready { parameters, .. } => (parameters, true),
            CompileState::NeedsParameters(parameters) => (parameters, false),
            _ => return None,
        };
        self.parameters_reported = true;
        Some((parameters.clone(), current))
    }

    fn update_source(
        &mut self,
        item: &VideoItem,
        canvas_size: CanvasSize,
        fps: Fraction,
    ) -> Result<(), String> {
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("Manim source received a non-Manim visual".into());
        };
        let snapshot = match item.file.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.cancel_compile();
                self.source_available = false;
                self.scene.clone_from(&manim.scene);
                self.parameters.clone_from(&manim.parameters);
                self.canvas_size = canvas_size;
                self.fps = fps;
                self.duration_reported = false;
                self.parameters_reported = false;
                self.loading_progress = None;
                self.state = CompileState::Idle;
                return Err(error);
            }
        };
        if self.source_available
            && self.snapshot == snapshot
            && self.scene == manim.scene
            && self.parameters == manim.parameters
            && self.canvas_size == canvas_size
            && self.fps == fps
        {
            return Ok(());
        }
        self.cancel_compile();
        self.snapshot = snapshot;
        self.source_available = true;
        self.scene.clone_from(&manim.scene);
        self.parameters.clone_from(&manim.parameters);
        self.canvas_size = canvas_size;
        self.fps = fps;
        self.duration_reported = false;
        self.parameters_reported = false;
        self.loading_progress = None;
        self.state = CompileState::Idle;
        Ok(())
    }

    fn start_compile(&mut self, item: &VideoItem) {
        let settings = Settings {
            source: item.file.clone(),
            scene: self.scene.clone(),
            width: self.canvas_size.width,
            height: self.canvas_size.height,
            fps: self.fps,
            parameters: self.parameters.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let compiler_cancelled = cancelled.clone();
        tracing::info!(
            source = %settings.source.path().display(),
            scene = %settings.scene,
            parameters = settings.parameters.len(),
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
                    return Ok(Compiled::NeedsParameters(parameters));
                }
                let prepared = Arc::new(PreparedAnimation::new(animation.clone())?);
                tracing::info!(
                    source = %source.display(),
                    %scene,
                    frames = animation.frames().len(),
                    compile_ms = compiled_at.duration_since(started).as_millis(),
                    parameter_decode_ms = parameters_at.duration_since(compiled_at).as_millis(),
                    validation_ms = parameters_at.elapsed().as_millis(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "Manim shared WGPU preparation finished",
                );
                Ok(Compiled::Ready {
                    animation,
                    prepared,
                    parameters,
                })
            });
            if matches!(&result, Err(error) if error == "Manim compilation was cancelled") {
                tracing::debug!(
                    source = %source.display(),
                    %scene,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Manim compilation superseded",
                );
            } else if let Err(error) = &result {
                tracing::error!(
                    source = %source.display(),
                    %scene,
                    elapsed_ms = started.elapsed().as_millis(),
                    %error,
                    "Manim compilation or WGPU preparation failed",
                );
            }
            let _ = sender.send(CompileEvent::Finished(result));
        });
        self.state = CompileState::Loading {
            cancelled,
            receiver,
            progress: None,
            worker: Some(worker),
        };
    }

    fn receive_compile(&mut self) {
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
                            "Manim compiler stopped without returning an animation".into()
                        ));
                    }
                }
            },
            _ => None,
        };
        let Some(finished) = finished else {
            return;
        };
        if let CompileState::Loading { worker, .. } = &mut self.state
            && let Some(worker) = worker.take()
        {
            worker.join().expect("Manim compiler thread panicked");
        }
        self.state = match finished {
            Ok(Compiled::Ready {
                animation,
                prepared,
                parameters,
            }) => CompileState::Ready {
                animation,
                prepared,
                parameters,
            },
            Ok(Compiled::NeedsParameters(parameters)) => CompileState::NeedsParameters(parameters),
            Err(error) => CompileState::Failed(error),
        };
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
}

impl Drop for Source {
    fn drop(&mut self) {
        self.cancel_compile();
    }
}

pub fn loading_pixels(
    width: u32,
    height: u32,
    progress: Option<Progress>,
) -> Result<Vec<u8>, String> {
    use shrimply_manim_parser::ProgressStage;
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
    shrimply_loading_screen::render(
        width,
        height,
        &counter,
        LOADING_COLOR,
        LOADING_PROGRESS_COLOR,
    )
}
