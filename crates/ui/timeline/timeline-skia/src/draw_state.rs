use super::*;

pub struct TimelineSoftwareCursor {
    pub position: Vec2,
    pub cursor: shrimply_components_skia::cursor::SoftwareCursor,
}

pub use shrimply_timeline_paint_skia::{TrackButtonId, TrackLabelAction};

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

pub use shrimply_timeline_paint_skia::{NaturalEndMarker, PreviewTimeMode, TimedItemBox};

pub struct TimelineDraw<'a> {
    pub painter: &'a TimelinePainter,
    pub waveforms: &'a WaveformMap,
    pub timeline_x: f64,
    pub timeline_width: f64,
    pub waveform_chunks_per_second: u32,
    pub view: TimelineViewState,
    pub animation_seconds: f64,
}

pub use shrimply_timeline_paint_skia::TrackControlDraw;

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
