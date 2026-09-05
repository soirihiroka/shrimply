use super::*;

pub struct TimelineSoftwareCursor {
    pub position: Vec2,
    pub cursor: shrimply_skia_adw_core::cursor::SoftwareCursor,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TrackLabelAction {
    Select,
    Toggle,
    Add,
    AudioRecord,
    VideoRecord,
}

pub type TrackButtonId = (TrackKey, TrackLabelAction);

pub struct LiveRecordingDraw {
    pub key: TrackKey,
    pub item: AudioItem,
    pub waveform: waveform::Waveform,
}

#[derive(Clone, Copy)]
pub struct LiveVideoRecordingDraw {
    pub key: TrackKey,
    pub start: Time,
    pub end: Time,
}

#[derive(Clone, Copy)]
pub enum WaveformState<'a> {
    Loading,
    Loaded(Option<&'a waveform::Waveform>),
}

#[derive(Clone, Copy)]
pub struct NaturalEndMarker {
    pub start: Time,
    pub end: Time,
    pub position: Option<Time>,
    pub repeat_interval: Option<Time>,
    pub real_start: Option<Time>,
    pub real_end: Option<Time>,
}

#[derive(Clone, Copy)]
pub struct TimedItemBox {
    pub marker: NaturalEndMarker,
    pub bounds: Rect,
    pub fill: Color,
    pub timeline_x: f64,
    pub view: TimelineViewState,
    pub selected: bool,
    pub selected_border_color: Color,
}

#[derive(Clone, Copy)]
pub enum PreviewTimeMode {
    Move,
    Resize,
}

pub struct TimelineDraw<'a> {
    pub painter: &'a TimelinePainter,
    pub waveforms: &'a WaveformMap,
    pub timeline_x: f64,
    pub timeline_width: f64,
    pub waveform_chunks_per_second: u32,
    pub view: TimelineViewState,
    pub animation_seconds: f64,
}

pub struct TrackControlDraw<'a> {
    pub animation_active: &'a mut bool,
    pub buttons: &'a mut HashMap<TrackButtonId, shrimply_skia_adw_core::button::Button>,
    pub active_audio_recording_key: Option<TrackKey>,
    pub active_video_recording_key: Option<TrackKey>,
}

#[derive(Clone)]
pub struct TimelineImportPreview {
    pub source: crate::project::Asset,
    pub duration: Time,
    pub visual_kind: Option<import::VisualMediaKind>,
    pub preview: import::ImportPreview,
    pub y: f64,
}

#[derive(Clone)]
pub struct TimelineCut {
    pub key: crate::project::ItemAddress,
    pub time: Time,
    pub keys: Vec<crate::project::ItemAddress>,
}
