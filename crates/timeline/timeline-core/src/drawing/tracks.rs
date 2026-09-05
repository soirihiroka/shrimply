use super::*;

mod audio;
mod caption;
mod video;

#[derive(Clone, Copy)]
pub struct TrackDrawInput<'a> {
    painter: &'a TimelinePainter,
    pub project: &'a Project,
    selected_items: &'a [ItemKey],
    pub selected_nested_items: &'a [crate::project::ItemAddress],
    pub folded_drag: Option<&'a folded_sequence::FoldedDrag>,
    pub selected_tracks: &'a [crate::project::TrackAddress],
    dragged_group: Option<&'a DraggedGroup>,
    resize_drag: Option<&'a ResizeDrag>,
    pub transition_drag: Option<&'a TransitionDrag>,
    pub clip_transition_drag: Option<&'a ClipTransitionDrag>,
    pub focused_transition: Option<&'a (crate::project::ItemAddress, TransitionSide)>,
    live_recording: Option<&'a LiveRecordingDraw>,
    live_video_recording: Option<LiveVideoRecordingDraw>,
    virtual_tracks: &'a [(TrackKind, usize)],
    view: TimelineViewState,
    timeline_x: f64,
    timeline_width: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_tracks(
    painter: &TimelinePainter,
    project: &Project,
    waveforms: &WaveformMap,
    selected_items: &[ItemKey],
    selected_nested_items: &[crate::project::ItemAddress],
    folded_drag: Option<&folded_sequence::FoldedDrag>,
    selected_tracks: &[crate::project::TrackAddress],
    dragged_group: Option<&DraggedGroup>,
    resize_drag: Option<&ResizeDrag>,
    transition_drag: Option<&TransitionDrag>,
    clip_transition_drag: Option<&ClipTransitionDrag>,
    focused_transition: Option<&(crate::project::ItemAddress, TransitionSide)>,
    live_recording: Option<&LiveRecordingDraw>,
    live_video_recording: Option<LiveVideoRecordingDraw>,
    virtual_tracks: &[(TrackKind, usize)],
    view: TimelineViewState,
    timeline_x: f64,
    timeline_width: f64,
    content_height: f64,
    animation_seconds: f64,
    waveform_chunks_per_second: u32,
) {
    let draw = TimelineDraw {
        painter,
        waveforms,
        timeline_x,
        timeline_width,
        waveform_chunks_per_second,
        view,
        animation_seconds,
    };
    let input = TrackDrawInput {
        painter,
        project,
        selected_items,
        selected_nested_items,
        folded_drag,
        selected_tracks,
        dragged_group,
        resize_drag,
        transition_drag,
        clip_transition_drag,
        focused_transition,
        live_recording,
        live_video_recording,
        virtual_tracks,
        view,
        timeline_x,
        timeline_width,
    };
    let (first_visible_row, last_visible_row) = visible_row_range(view, content_height);
    caption::draw(input, first_visible_row, last_visible_row);
    video::draw(
        &draw,
        input,
        content_height,
        first_visible_row,
        last_visible_row,
    );
    audio::draw(
        &draw,
        input,
        content_height,
        first_visible_row,
        last_visible_row,
    );
}
