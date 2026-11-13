//! Read-only bridge from render-owned preparation to reflected editor controls.
//! Observations never own workers, so UI reads and teardown cannot join a runtime.
use super::Viewport;
use sanim_core::cache::Cache;
pub use sanim_core::reflection::Declaration;
pub use sanim_core::{Diagnostic, ReflectedInput, Type, Unit, Value, math, reflection::Schema};
pub use sanim_runtime::{Observation, Snapshot, sanim_lang};
use shrimply_project_document::project::{VideoItem, VideoItemContent};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
};

const MAX_SOURCES: usize = 256;
const MAX_INDEX_BYTES: usize = 1024 * 1024;
struct Published {
    path: PathBuf,
    observation: Weak<Observation>,
}
static SOURCES: OnceLock<Mutex<Cache<uuid::Uuid, Published>>> = OnceLock::new();
fn sources() -> &'static Mutex<Cache<uuid::Uuid, Published>> {
    SOURCES.get_or_init(|| Mutex::new(Cache::new(MAX_INDEX_BYTES, MAX_SOURCES)))
}
pub(super) fn publish(item: &VideoItem, observation: &Arc<Observation>) {
    let path = item.file.path().to_owned();
    let bytes = std::mem::size_of::<Published>() + path.as_os_str().len();
    sources()
        .lock()
        .expect("Sanim reflection sources poisoned")
        .insert(
            item.id,
            Arc::new(Published {
                path,
                observation: Arc::downgrade(observation),
            }),
            bytes,
        );
}
pub fn observation(item_id: uuid::Uuid, path: &std::path::Path) -> Option<Arc<Observation>> {
    let source = sources()
        .lock()
        .expect("Sanim reflection sources poisoned")
        .get(&item_id)?;
    (source.path == path)
        .then(|| source.observation.upgrade())
        .flatten()
}
pub fn schema(prepared: &sanim_runtime::sanim_lang::Prepared) -> Result<Schema, String> {
    Schema::new(
        &prepared.source,
        prepared
            .reflected
            .iter()
            .map(|input| input.declaration.clone()),
    )
    .map_err(|e| e.to_string())
}
pub fn effective_inputs(
    prepared: &sanim_runtime::sanim_lang::Prepared,
    parameters: &BTreeMap<String, serde_json::Value>,
) -> Result<Vec<ReflectedInput>, String> {
    let supplied = schema(prepared)?
        .normalize(parameters, validate)
        .map_err(|e| e.to_string())?;
    Ok(prepared
        .reflected
        .iter()
        .cloned()
        .map(|mut input| {
            input.value = supplied.get(&input.key).unwrap_or(&input.default).clone();
            input
        })
        .collect())
}
pub fn to_json(value: &Value) -> Result<serde_json::Value, String> {
    sanim_data::to_json(value)
}
pub fn validate(ty: &Type, value: &serde_json::Value) -> Result<Value, String> {
    sanim_data::validate(value, ty, Default::default())
}
pub fn set(
    item: &mut VideoItem,
    prepared: &sanim_runtime::sanim_lang::Prepared,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<bool, String> {
    match value {
        Some(value) => patch(
            item,
            prepared,
            &BTreeMap::from([(key.to_owned(), value)]),
            &[],
        ),
        None => patch(item, prepared, &BTreeMap::new(), &[key.to_owned()]),
    }
}
/// Validate an atomic override patch; null is a value, reset is explicit.
pub fn patch(
    item: &mut VideoItem,
    prepared: &sanim_runtime::sanim_lang::Prepared,
    parameters: &BTreeMap<String, serde_json::Value>,
    reset_keys: &[String],
) -> Result<bool, String> {
    let schema = schema(prepared)?;
    let normalized = schema
        .normalize(parameters, validate)
        .map_err(|e| e.to_string())?;
    let encoded = normalized
        .iter()
        .map(|(key, value)| Ok((key.clone(), to_json(value)?)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        return Err("reflection edit requires a Sanim clip".into());
    };
    let mut resets = std::collections::BTreeSet::new();
    for key in reset_keys {
        let canonical = match schema.declaration(key) {
            Ok(declaration) => declaration.key.clone(),
            Err(_) if settings.parameters.contains_key(key) => key.clone(),
            Err(error) => return Err(error.to_string()),
        };
        if encoded.contains_key(&canonical) {
            return Err(format!(
                "reflected input '{canonical}' cannot be set and reset in the same patch"
            ));
        }
        if !resets.insert(canonical) {
            return Err(format!("reflected input '{key}' was reset more than once"));
        }
    }
    let mut changed = false;
    settings.parameters.retain(|key, _| {
        let keep = !resets.contains(key);
        changed |= !keep;
        keep
    });
    for (key, value) in encoded {
        if settings.parameters.get(&key) != Some(&value) {
            settings.parameters.insert(key, value);
            changed = true;
        }
    }
    Ok(changed)
}

pub fn viewport(width: u32, height: u32, rate: sanim_core::Fraction) -> Viewport {
    Viewport {
        width,
        height,
        rate,
        ..Default::default()
    }
}

/// One inspector-owned discovery worker. Dropping it closes the command channel;
/// worker shutdown and Runtime joins happen on that worker, never on the UI thread.
pub struct Discovery {
    item_id: uuid::Uuid,
    path: PathBuf,
    viewport: Viewport,
    inputs: Arc<Mutex<BTreeMap<String, serde_json::Value>>>,
    notify: std::sync::mpsc::SyncSender<()>,
    result: Arc<Mutex<Result<Option<Arc<Observation>>, String>>>,
}
impl Discovery {
    pub fn new(item: &VideoItem, viewport: Viewport) -> Result<Self, String> {
        let VideoItemContent::Sanim(settings) = &item.content else {
            return Err("reflection discovery requires a Sanim clip".into());
        };
        let path = item.file.path().to_owned();
        let inputs = Arc::new(Mutex::new(settings.parameters.clone()));
        let result = Arc::new(Mutex::new(Ok(None)));
        let (notify, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("sanim-inspector".into())
            .spawn({
                let path = path.clone();
                let viewport = viewport.clone();
                let inputs = inputs.clone();
                let result = result.clone();
                move || {
                    let runtime = (|| {
                        let runtime = sanim_runtime::Runtime::with_viewport(
                            path,
                            viewport,
                            Default::default(),
                        )?;
                        let current_inputs = inputs
                            .lock()
                            .expect("Sanim inspector inputs poisoned")
                            .clone();
                        runtime.set_inputs(current_inputs)?;
                        Ok::<_, String>(runtime)
                    })();
                    let runtime = match runtime {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            *result.lock().expect("Sanim inspector result poisoned") = Err(error);
                            return;
                        }
                    };
                    *result.lock().expect("Sanim inspector result poisoned") =
                        Ok(Some(Arc::new(runtime.observe())));
                    while receiver.recv().is_ok() {
                        while receiver.try_recv().is_ok() {}
                        let current_inputs = inputs
                            .lock()
                            .expect("Sanim inspector inputs poisoned")
                            .clone();
                        if let Err(error) = runtime.set_inputs(current_inputs) {
                            *result.lock().expect("Sanim inspector result poisoned") = Err(error);
                            return;
                        }
                    }
                }
            })
            .map_err(|e| format!("could not start reflection discovery: {e}"))?;
        Ok(Self {
            item_id: item.id,
            path,
            viewport,
            inputs,
            notify,
            result,
        })
    }
    pub fn matches(&self, item: &VideoItem, viewport: &Viewport) -> bool {
        self.item_id == item.id && self.path == item.file.path() && &self.viewport == viewport
    }
    pub fn set_inputs(
        &self,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        let mut inputs = self.inputs.lock().expect("Sanim inspector inputs poisoned");
        if &*inputs == parameters {
            return Ok(());
        }
        inputs.clone_from(parameters);
        drop(inputs);
        match self.notify.try_send(()) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(())) => {
                Err("Sanim reflection discovery worker stopped".into())
            }
        }
    }
    pub fn observation(&self) -> Result<Option<Arc<Observation>>, String> {
        self.result
            .lock()
            .expect("Sanim inspector result poisoned")
            .clone()
    }
}

/// Value inspection refreshes changed defaults; schema inspection reuses valid source types.
#[derive(Clone, Copy)]
pub enum Inspection {
    Values,
    Schema,
}
/// Headless metadata queries use the same background preparation as preview.
pub fn inspect(
    item: &VideoItem,
    viewport: Viewport,
    canceled: &std::sync::atomic::AtomicBool,
    inspection: Inspection,
) -> Result<Snapshot, String> {
    let VideoItemContent::Sanim(settings) = &item.content else {
        return Err("reflection inspection requires a Sanim clip".into());
    };
    let parent = item
        .file
        .path()
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let sources = sanim_lang::FileSources::new(parent)?;
    if let Some(observation) = observation(item.id, item.file.path()) {
        let state = observation.snapshot();
        if let Some(prepared) = &state.prepared {
            // Invalid overrides still need their verified schema so they can be repaired.
            let same_inputs = matches!(inspection, Inspection::Schema)
                || match effective_inputs(prepared, &settings.parameters) {
                    Ok(inputs) => inputs
                        .iter()
                        .zip(prepared.reflected.iter())
                        .all(|(current, applied)| current.value == applied.value),
                    Err(_) => true,
                };
            if same_inputs
                && prepared.animation.width == viewport.width
                && prepared.animation.height == viewport.height
                && prepared.animation.rate == viewport.rate
                && prepared.animation.dpi == viewport.dpi
                && prepared.validate_dependencies(&sources).is_ok()
            {
                return Ok(state);
            }
        }
    }
    let runtime =
        sanim_runtime::Runtime::with_viewport(item.file.path(), viewport, Default::default())?;
    runtime.set_inputs(settings.parameters.clone())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut state = wait_for_preparation(&runtime, canceled, deadline)?;
    if state.prepared.is_none()
        && !settings.parameters.is_empty()
        && state
            .diagnostic
            .as_ref()
            .is_some_and(|error| error.code == "S302")
    {
        // A bad persisted override must not prevent discovering the schema needed to repair it.
        let invalid_overrides = state.diagnostic;
        runtime.set_inputs(BTreeMap::new())?;
        state = wait_for_preparation(&runtime, canceled, deadline)?;
        if state.prepared.is_some() {
            state.diagnostic = invalid_overrides;
        }
    }
    Ok(state)
}
fn wait_for_preparation(
    runtime: &sanim_runtime::Runtime,
    canceled: &std::sync::atomic::AtomicBool,
    deadline: std::time::Instant,
) -> Result<Snapshot, String> {
    let mut state = runtime.snapshot();
    while state.preparing {
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Sanim inspection canceled".into());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("Sanim reflection preparation did not finish within 30 seconds".into());
        }
        state = runtime.wait_for_change(
            state.generation,
            remaining.min(std::time::Duration::from_secs(1)),
        );
    }
    Ok(state)
}

pub fn parameter_json(input: &ReflectedInput) -> Result<serde_json::Value, String> {
    Ok(
        serde_json::json!({"key":input.key,"name":input.name,"label":input.label,"source":input.source,"span":input.span,"type_name":input.ty.to_string(),"type_info":input.ty,"default":to_json(&input.default)?,"value":to_json(&input.value)?}),
    )
}
