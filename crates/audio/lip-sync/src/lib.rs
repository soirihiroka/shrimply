use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use hashbrown::{HashMap, HashSet};
use shrimply_project::project::Time;

static ANALYZER: OnceLock<shrimply_rhubarb_lip_sync::Analyzer> = OnceLock::new();

pub const SAMPLE_RATE: u32 = shrimply_rhubarb_lip_sync::SAMPLE_RATE_HZ;

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
            _ => Err(format!(
                "lip-sync analyzer returned unknown mouth shape byte {value}"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouthCue {
    pub start: Time,
    pub end: Time,
    pub shape: MouthShape,
}

pub fn analyze(samples: &[i16]) -> Result<Vec<MouthCue>, String> {
    let analyzer = if let Some(analyzer) = ANALYZER.get() {
        analyzer
    } else {
        let analyzer = shrimply_rhubarb_lip_sync::Analyzer::new()?;
        let _ = ANALYZER.set(analyzer);
        ANALYZER.get().expect("analyzer was initialized")
    };
    let cues = analyzer.analyze(samples)?;
    Ok(cues
        .into_iter()
        .map(|cue| MouthCue {
            start: Time::from_fraction(cue.start_centiseconds, 100),
            end: Time::from_fraction(cue.end_centiseconds, 100),
            shape: match cue.shape {
                shrimply_rhubarb_lip_sync::MouthShape::A => MouthShape::A,
                shrimply_rhubarb_lip_sync::MouthShape::B => MouthShape::B,
                shrimply_rhubarb_lip_sync::MouthShape::C => MouthShape::C,
                shrimply_rhubarb_lip_sync::MouthShape::D => MouthShape::D,
                shrimply_rhubarb_lip_sync::MouthShape::E => MouthShape::E,
                shrimply_rhubarb_lip_sync::MouthShape::F => MouthShape::F,
                shrimply_rhubarb_lip_sync::MouthShape::G => MouthShape::G,
                shrimply_rhubarb_lip_sync::MouthShape::H => MouthShape::H,
                shrimply_rhubarb_lip_sync::MouthShape::X => MouthShape::X,
            },
        })
        .collect())
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
