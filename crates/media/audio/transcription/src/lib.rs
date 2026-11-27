use hashbrown::HashMap;

use shrimply_audio_engine::streaming::{AudioRenderSession, AudioSourceKey, mix_project_range};
use shrimply_math_core::Time;
use shrimply_project_document::project::Project;

const CHANNELS: usize = 2;
pub const SAMPLE_RATE: u32 = 16_000;

pub struct PreparedTranscriptionChunk {
    pub start: Time,
    pub end: Time,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct TranscribedSegment {
    pub start: Time,
    pub end: Time,
    pub text: String,
}

pub fn prepare_transcription_chunks(
    project: &Project,
    ranges: &[(Time, Time)],
) -> Result<Vec<PreparedTranscriptionChunk>, String> {
    if ranges.is_empty() {
        return Err("Selected audio has no duration".to_string());
    }
    if ranges.iter().any(|(start, end)| end <= start) {
        return Err("Selected audio has no duration".to_string());
    }

    let mut chunks = Vec::new();
    for &(start, end) in ranges {
        let samples = mix_range_mono(project, start, end);
        if samples.iter().all(|sample| sample.abs() <= f32::EPSILON) {
            continue;
        }
        chunks.push(PreparedTranscriptionChunk {
            start,
            end,
            samples,
        });
    }
    if chunks.is_empty() {
        return Err("Selected audio decoded to silence".to_string());
    }
    Ok(chunks)
}

pub fn sanitize_transcribed_segments(
    segments: &mut Vec<TranscribedSegment>,
    frame_step: Time,
) -> usize {
    for segment in segments.iter_mut() {
        segment.start = segment.start.snapped(frame_step);
        segment.end = segment.end.snapped(frame_step);
    }
    segments.retain(|segment| !segment.text.trim().is_empty() && segment.end > segment.start);

    let mut overlap_count = 0;
    loop {
        segments.sort_by_key(|segment| (segment.start, segment.end));
        let Some(index) = segments
            .windows(2)
            .position(|pair| pair[0].end > pair[1].start)
        else {
            break;
        };
        let overlap_start = segments[index].start.max(segments[index + 1].start);
        let overlap_end = segments[index].end.min(segments[index + 1].end);
        let boundary =
            shrimply_math_media::time_midpoint(overlap_start, overlap_end).snapped(frame_step);
        segments[index].end = boundary;
        segments[index + 1].start = boundary;
        overlap_count += 1;
        segments.retain(|segment| segment.end > segment.start);
    }
    overlap_count
}

fn mix_range_mono(project: &Project, start: Time, end: Time) -> Vec<f32> {
    let start_frame = start.as_sample_frame(SAMPLE_RATE);
    let end_frame = end.as_sample_frame(SAMPLE_RATE);
    let frame_count = end_frame.saturating_sub(start_frame) as usize;
    let mut sessions: HashMap<AudioSourceKey, AudioRenderSession> = HashMap::new();
    let stereo = mix_project_range(
        project,
        &mut sessions,
        start_frame,
        frame_count,
        SAMPLE_RATE,
    );
    stereo
        .as_chunks::<CHANNELS>()
        .0
        .iter()
        .map(|frame| {
            let sample = frame[0] * 0.5 + frame[1] * 0.5;
            if sample.is_finite() { sample } else { 0.0 }
        })
        .collect()
}
