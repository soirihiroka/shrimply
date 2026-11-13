//! Event-driven rendered-item inspection. The UI submits clicks; this worker owns Skia caches.
use sanim_core::Fraction;
use sanim_runtime::{
    Observation,
    sanim_lang::{Prepared, SourceLocation},
    sanim_render::{Options, Renderer, Scene},
};
use shrimply_project_document::project::{
    CanvasSize, ItemAddress, Time, VideoItem, generated_source_time_with_duration,
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, Weak, mpsc},
};

/// Immutable identity of the exact native scene/time used to draw a source frame.
/// Only presented frames retain these snapshots; no global scene history is kept.
pub struct FrameSource {
    prepared: Arc<Prepared>,
    time: Fraction,
    viewport: sanim_core::Viewport,
    observation: Weak<Observation>,
    revision: u64,
    item_id: uuid::Uuid,
    path: PathBuf,
}
/// Each nested clip instance has its own key; conflicting samples of one instance are omitted.
#[derive(Default)]
pub struct FrameSources {
    sources: std::collections::HashMap<ItemAddress, Arc<FrameSource>>,
    ambiguous: std::collections::HashSet<ItemAddress>,
}
impl FrameSources {
    pub fn get(&self, address: &ItemAddress) -> Option<&Arc<FrameSource>> {
        self.sources.get(address)
    }
    /// Preserve repeated identical samples; omit an instance if it drew conflicting samples.
    pub fn insert(&mut self, address: ItemAddress, source: Arc<FrameSource>) {
        if self.ambiguous.contains(&address) {
            return;
        }
        if self
            .sources
            .get(&address)
            .is_some_and(|previous| !same_sample(previous, &source))
        {
            self.sources.remove(&address);
            self.ambiguous.insert(address);
        } else {
            self.sources.insert(address, source);
        }
    }
}
impl FrameSource {
    pub(super) fn new(
        item: &VideoItem,
        prepared: Arc<Prepared>,
        time: Fraction,
        viewport: sanim_core::Viewport,
        observation: &Arc<Observation>,
    ) -> Arc<Self> {
        Arc::new(Self {
            prepared,
            time,
            viewport,
            observation: Arc::downgrade(observation),
            revision: observation.snapshot().revision,
            item_id: item.id,
            path: item.file.path().to_owned(),
        })
    }
}
pub struct Request {
    pub source: Arc<FrameSource>,
    pub item: VideoItem,
    pub position: Time,
    pub canvas: CanvasSize,
    pub rate: Fraction,
    /// Source-canvas pixels, before the Shrimply clip transform.
    pub point: [f32; 2],
    pub tolerance_px: f32,
}
#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub line_text: Arc<str>,
}
pub struct Inspection {
    pub id: String,
    pub kind: &'static str,
    pub ancestors: Vec<String>,
    pub quad: [[f32; 2]; 4],
    /// Explicitly absent for scenes with no recorded constructor origin.
    pub creation: Option<Location>,
    /// The constructor owner when a lazily generated descendant has no separate source node.
    pub source_element: Option<String>,
    pub call_sites: Vec<Location>,
    source: Arc<FrameSource>,
}
/// Source edits never leave a result pointing into a different prepared revision.
pub fn is_current(inspection: &Inspection) -> bool {
    let source = &inspection.source;
    let Some(observation) = source.observation.upgrade() else {
        return false;
    };
    let snapshot = observation.snapshot();
    !snapshot.preparing
        && snapshot.diagnostic.is_none()
        && snapshot.revision == source.revision
        && snapshot.prepared.as_ref().is_some_and(|prepared| {
            std::ptr::eq(Arc::as_ptr(prepared), Arc::as_ptr(&source.prepared))
        })
}
/// A result remains attached to its exact presented source scene and sample time.
pub fn matches_frame(inspection: &Inspection, frame: &FrameSource) -> bool {
    same_sample(&inspection.source, frame)
}
fn same_sample(source: &FrameSource, frame: &FrameSource) -> bool {
    source.item_id == frame.item_id
        && source.path == frame.path
        && source.time == frame.time
        && source.viewport == frame.viewport
        && source.revision == frame.revision
        && Arc::ptr_eq(&source.prepared, &frame.prepared)
        && Weak::ptr_eq(&source.observation, &frame.observation)
}
type ResultSender = mpsc::SyncSender<Result<Option<Inspection>, String>>;
struct Job {
    request: Request,
    result: ResultSender,
}
static WORKER: OnceLock<Result<mpsc::SyncSender<Job>, String>> = OnceLock::new();
/// Queue a click without source I/O, evaluation, recording, or GPU readback on the caller.
/// A bounded queue refuses excess clicks instead of retaining unbounded source snapshots.
pub fn request(
    request: Request,
) -> Result<mpsc::Receiver<Result<Option<Inspection>, String>>, String> {
    if request.source.item_id != request.item.id || request.source.path != request.item.file.path()
    {
        return Err("Sanim inspection source does not match the selected clip".into());
    }
    if request.source.observation.strong_count() == 0 {
        return Err(
            "Sanim preview source is no longer active; inspect the refreshed preview".into(),
        );
    }
    let sender = WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Job>(1);
            std::thread::Builder::new()
                .name("sanim-element-inspection".into())
                .spawn(move || {
                    let mut renderer = Renderer::default();
                    let mut scene: Option<(Weak<Prepared>, sanim_core::Viewport, Scene)> = None;
                    while let Ok(job) = receiver.recv() {
                        let result = inspect(&mut renderer, &mut scene, &job.request);
                        let _ = job.result.send(result);
                    }
                })
                .map_err(|error| format!("could not start Sanim element inspection: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (result, receiver) = mpsc::sync_channel(1);
    sender
        .try_send(Job { request, result })
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                "Sanim element inspection is busy; try the click again".to_owned()
            }
            mpsc::TrySendError::Disconnected(_) => {
                "Sanim element inspection worker stopped".to_owned()
            }
        })?;
    Ok(receiver)
}
fn inspect(
    renderer: &mut Renderer,
    scene: &mut Option<(Weak<Prepared>, sanim_core::Viewport, Scene)>,
    request: &Request,
) -> Result<Option<Inspection>, String> {
    let token = &request.source;
    let prepared = &token.prepared;
    let animation = &prepared.animation;
    if token.viewport.width != request.canvas.width
        || token.viewport.height != request.canvas.height
        || token.viewport.rate != request.rate
    {
        return Err("Sanim preview viewport changed; inspect the refreshed preview".into());
    }
    let Some(time) = generated_source_time_with_duration(
        &request.item,
        request.position,
        Time {
            seconds: animation.duration,
        },
    ) else {
        return Ok(None);
    };
    if time.seconds.min(animation.duration) != token.time {
        return Err("Sanim preview time changed; inspect the refreshed preview".into());
    }
    if scene.as_ref().is_none_or(|(previous, viewport, _)| {
        viewport != &token.viewport || !std::ptr::eq(previous.as_ptr(), Arc::as_ptr(prepared))
    }) {
        *scene = Some((
            Arc::downgrade(prepared),
            token.viewport.clone(),
            renderer.prepare(
                prepared
                    .animation_for_viewport(token.viewport.width, token.viewport.height)
                    .map_err(|error| error.to_string())?,
            )?,
        ));
    }
    let scene = &scene.as_ref().expect("prepared inspection scene").2;
    let options = Options::for_animation(scene.animation());
    let Some(hit) = renderer.pick(
        scene,
        token.time,
        &options,
        request.point,
        request.tolerance_px,
    )?
    else {
        return Ok(None);
    };
    let origin = std::iter::once(&hit.id)
        .chain(hit.ancestors.iter().rev())
        .find_map(|id| prepared.element_sources.get(id).map(|source| (id, source)));
    let source = origin.map(|(_, source)| source);
    let source_element = origin.map(|(id, _)| id.clone());
    let parent = request
        .item
        .file
        .path()
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let location = |source: &SourceLocation| Location {
        path: parent.join(source.source.as_ref()),
        line: source.span.line,
        column: source.span.column,
        line_text: Arc::from(prepared.source_line(source).unwrap_or("")),
    };
    let inspection = Inspection {
        id: hit.id,
        kind: hit.kind,
        ancestors: hit.ancestors,
        quad: hit.quad,
        creation: source.map(|source| location(&source.creation)),
        source_element,
        call_sites: source
            .map(|source| source.call_sites.iter().map(location).collect())
            .unwrap_or_default(),
        source: token.clone(),
    };
    if !is_current(&inspection) {
        return Err("Sanim source changed during inspection; inspect the refreshed preview".into());
    }
    Ok(Some(inspection))
}
