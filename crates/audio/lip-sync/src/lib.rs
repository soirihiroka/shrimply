use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use hashbrown::{HashMap, HashSet};
use shrimply_project::project::Time;

#[repr(C)]
struct NativeMouthCue {
    start_centiseconds: i64,
    end_centiseconds: i64,
    shape: u8,
}

#[repr(C)]
struct NativeResult {
    cues: *mut NativeMouthCue,
    cue_count: usize,
    error: *mut c_char,
}

type AnalyzeFunction =
    unsafe extern "C" fn(*const c_char, *const c_char, i32, *mut NativeResult) -> i32;
type FreeResultFunction = unsafe extern "C" fn(*mut NativeResult);

struct NativeApi {
    _handle: *mut c_void,
    analyze: AnalyzeFunction,
    free_result: FreeResultFunction,
}

// SAFETY: dlopen handles and immutable function pointers may be called from any thread. Rhubarb's
// own shared state is initialized once in the C++ shim.
unsafe impl Send for NativeApi {}
// SAFETY: See the Send implementation above.
unsafe impl Sync for NativeApi {}

static NATIVE_API: OnceLock<Result<NativeApi, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MouthShape {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    #[default]
    X,
}

impl MouthShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::X => "X",
        }
    }
}

impl TryFrom<u8> for MouthShape {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            b'A' => Ok(Self::A),
            b'B' => Ok(Self::B),
            b'C' => Ok(Self::C),
            b'D' => Ok(Self::D),
            b'E' => Ok(Self::E),
            b'F' => Ok(Self::F),
            b'G' => Ok(Self::G),
            b'H' => Ok(Self::H),
            b'X' => Ok(Self::X),
            _ => Err(format!("Rhubarb returned unknown mouth shape byte {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouthCue {
    pub start: Time,
    pub end: Time,
    pub shape: MouthShape,
}

pub fn analyze_wave(wave_path: &Path, model_directory: &Path) -> Result<Vec<MouthCue>, String> {
    let api = NATIVE_API
        .get_or_init(load_native_api)
        .as_ref()
        .map_err(Clone::clone)?;
    let wave_path = path_c_string(wave_path)?;
    let model_directory = path_c_string(model_directory)?;
    let max_threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(i32::MAX as usize) as i32;
    let mut native = NativeResult {
        cues: ptr::null_mut(),
        cue_count: 0,
        error: ptr::null_mut(),
    };
    // SAFETY: Both strings and the result storage remain alive for the call. The shim owns any
    // returned allocations until shrimply_rhubarb_free_result is called below.
    let status = unsafe {
        (api.analyze)(
            wave_path.as_ptr(),
            model_directory.as_ptr(),
            max_threads,
            &mut native,
        )
    };
    let result = if status == 0 {
        if native.cue_count > 0 && native.cues.is_null() {
            Err("Rhubarb returned a null cue array".to_string())
        } else {
            // SAFETY: The shim returns cue_count initialized entries when cues is non-null.
            let cues = if native.cue_count == 0 {
                &[]
            } else {
                // SAFETY: A successful non-empty result contains cue_count initialized entries.
                unsafe { std::slice::from_raw_parts(native.cues, native.cue_count) }
            };
            cues.iter()
                .map(|cue| {
                    Ok(MouthCue {
                        start: Time::from_fraction(cue.start_centiseconds, 100),
                        end: Time::from_fraction(cue.end_centiseconds, 100),
                        shape: MouthShape::try_from(cue.shape)?,
                    })
                })
                .collect()
        }
    } else if native.error.is_null() {
        Err("Rhubarb analysis failed without an error message".to_string())
    } else {
        // SAFETY: A non-null error from the shim is a null-terminated string.
        Err(unsafe { CStr::from_ptr(native.error) }
            .to_string_lossy()
            .into_owned())
    };
    // SAFETY: native was initialized above and has not been freed yet.
    unsafe { (api.free_result)(&mut native) };
    result
}

fn load_native_api() -> Result<NativeApi, String> {
    let library = native_library_path()?;
    let library_path = path_c_string(&library)?;
    // SAFETY: library_path is a valid null-terminated path. The handle remains open for the
    // process lifetime in NativeApi.
    let handle = unsafe { libc::dlopen(library_path.as_ptr(), libc::RTLD_NOW) };
    if handle.is_null() {
        return Err(format!(
            "could not load Rhubarb library {}: {}",
            library.display(),
            dynamic_loader_error()
        ));
    }
    // SAFETY: The crate builds these exact C ABI symbols into the library above.
    let analyze = unsafe {
        std::mem::transmute::<*mut c_void, AnalyzeFunction>(load_symbol(
            handle,
            c"shrimply_rhubarb_analyze",
        )?)
    };
    // SAFETY: The crate builds these exact C ABI symbols into the library above.
    let free_result = unsafe {
        std::mem::transmute::<*mut c_void, FreeResultFunction>(load_symbol(
            handle,
            c"shrimply_rhubarb_free_result",
        )?)
    };
    Ok(NativeApi {
        _handle: handle,
        analyze,
        free_result,
    })
}

unsafe fn load_symbol(handle: *mut c_void, name: &CStr) -> Result<*mut c_void, String> {
    // SAFETY: handle is returned by dlopen and name is null-terminated.
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if symbol.is_null() {
        Err(format!(
            "Rhubarb library is missing {}: {}",
            name.to_string_lossy(),
            dynamic_loader_error()
        ))
    } else {
        Ok(symbol)
    }
}

fn dynamic_loader_error() -> String {
    // SAFETY: dlerror returns either null or a process-owned null-terminated error string.
    let error = unsafe { libc::dlerror() };
    if error.is_null() {
        "unknown dynamic loader error".to_string()
    } else {
        // SAFETY: The non-null value from dlerror is null-terminated.
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

fn native_library_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHRIMPLY_RHUBARB_LIBRARY") {
        return validate_native_library(PathBuf::from(path));
    }
    if let Some(path) = option_env!("SHRIMPLY_BUILD_RHUBARB_LIBRARY") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the shrimply executable: {error}"))?;
    if let Some(directory) = executable.parent() {
        let path = directory.join(format!(
            "libshrimply-rhubarb{}",
            std::env::consts::DLL_SUFFIX
        ));
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("Rhubarb native library could not be located".to_string())
}

fn validate_native_library(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "Rhubarb native library is missing from {}",
            path.display()
        ))
    }
}

pub fn model_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHRIMPLY_RHUBARB_RESOURCES") {
        return validate_model_directory(PathBuf::from(path));
    }
    if let Some(path) = option_env!("SHRIMPLY_BUILD_RHUBARB_RESOURCES") {
        let path = PathBuf::from(path);
        if path.join("cmudict-en-us.dict").is_file() {
            return Ok(path);
        }
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the shrimply executable: {error}"))?;
    if let Some(directory) = executable.parent() {
        for candidate in [
            directory.join("res/sphinx"),
            directory.join("../share/shrimply/rhubarb/sphinx"),
        ] {
            if candidate.join("cmudict-en-us.dict").is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("Rhubarb model resources could not be located".to_string())
}

fn validate_model_directory(directory: PathBuf) -> Result<PathBuf, String> {
    if directory.join("cmudict-en-us.dict").is_file() {
        Ok(directory)
    } else {
        Err(format!(
            "Rhubarb model resources are missing from {}",
            directory.display()
        ))
    }
}

fn path_c_string(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| format!("path contains a null byte: {}", path.display()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MouthValue {
    Ready(MouthShape),
    Pending,
    Failed(String),
}

type MouthResolver = dyn Fn(&[usize], u128, Time, Time) -> MouthValue + Send + Sync;

#[derive(Clone, Eq, Hash, PartialEq)]
struct MouthRequest {
    indices: Vec<usize>,
    item_id: u128,
    start_nanos: i128,
    end_nanos: i128,
}

#[derive(Clone)]
pub struct FrameMouthMixer {
    track_count: usize,
    resolver: Arc<MouthResolver>,
    resolved: Arc<Mutex<HashMap<MouthRequest, MouthValue>>>,
    pending: Arc<AtomicBool>,
    frame: Arc<()>,
}

impl FrameMouthMixer {
    pub fn resolving(
        track_count: usize,
        resolver: impl Fn(&[usize], u128, Time, Time) -> MouthValue + Send + Sync + 'static,
    ) -> Self {
        Self {
            track_count,
            resolver: Arc::new(resolver),
            resolved: Default::default(),
            pending: Default::default(),
            frame: Arc::new(()),
        }
    }

    pub fn silent(track_count: usize) -> Self {
        Self::resolving(track_count, |_, _, _, _| MouthValue::Ready(MouthShape::X))
    }

    pub fn all(&self, item_id: u128, start: Time, end: Time) -> MouthValue {
        self.resolve((0..self.track_count).collect(), item_id, start, end)
    }

    pub fn selected(
        &self,
        indices: &[usize],
        item_id: u128,
        start: Time,
        end: Time,
    ) -> Result<MouthValue, MouthSelectionError> {
        let mut selected = HashSet::with_capacity(indices.len());
        for &index in indices {
            if index >= self.track_count {
                return Err(MouthSelectionError::OutOfRange {
                    index,
                    track_count: self.track_count,
                });
            }
            if !selected.insert(index) {
                return Err(MouthSelectionError::Duplicate(index));
            }
        }
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        Ok(self.resolve(indices, item_id, start, end))
    }

    fn resolve(&self, indices: Vec<usize>, item_id: u128, start: Time, end: Time) -> MouthValue {
        let request = MouthRequest {
            indices,
            item_id,
            start_nanos: start.as_nanos_i128(),
            end_nanos: end.as_nanos_i128(),
        };
        if let Some(value) = self
            .resolved
            .lock()
            .expect("frame mouth cache mutex poisoned")
            .get(&request)
            .cloned()
        {
            return value;
        }
        let value = (self.resolver)(&request.indices, item_id, start, end);
        if matches!(value, MouthValue::Pending) {
            self.pending.store(true, Ordering::Relaxed);
        } else {
            self.resolved
                .lock()
                .expect("frame mouth cache mutex poisoned")
                .insert(request, value.clone());
        }
        value
    }

    pub fn pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed)
    }

    pub fn failures(&self) -> Vec<String> {
        self.resolved
            .lock()
            .expect("frame mouth cache mutex poisoned")
            .values()
            .filter_map(|value| match value {
                MouthValue::Failed(error) => Some(error.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn same_frame(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
    }
}

impl Default for FrameMouthMixer {
    fn default() -> Self {
        Self::silent(0)
    }
}

#[derive(Clone)]
pub struct FrameAudioAnalysis {
    pub volume: shrimply_math_media::FrameVolumeMixer,
    pub mouth: FrameMouthMixer,
}

impl FrameAudioAnalysis {
    pub fn silent(track_count: usize) -> Self {
        Self {
            volume: shrimply_math_media::FrameVolumeMixer::silent(track_count),
            mouth: FrameMouthMixer::silent(track_count),
        }
    }

    pub fn same_frame(&self, other: &Self) -> bool {
        self.volume.same_frame(&other.volume) && self.mouth.same_frame(&other.mouth)
    }
}

impl Default for FrameAudioAnalysis {
    fn default() -> Self {
        Self::silent(0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouthSelectionError {
    Duplicate(usize),
    OutOfRange { index: usize, track_count: usize },
}

impl std::fmt::Display for MouthSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(index) => {
                write!(formatter, "audio track {index} was selected more than once")
            }
            Self::OutOfRange { index, track_count } => write!(
                formatter,
                "audio track {index} is out of range for {track_count} tracks"
            ),
        }
    }
}

impl std::fmt::Debug for FrameMouthMixer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameMouthMixer")
            .field("track_count", &self.track_count)
            .finish_non_exhaustive()
    }
}
