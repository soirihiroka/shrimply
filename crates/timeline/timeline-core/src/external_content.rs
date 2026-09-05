use super::*;
#[derive(Clone)]
pub struct TextPreview {
    pub text: String,
    pub kind: TrackKind,
    pub track_index: usize,
    pub start: Time,
    pub end: Time,
}
