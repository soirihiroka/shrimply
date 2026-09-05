use shrimply_project::project::{Time, VideoItem};
pub use shrimply_video_decoder::{
    DEFAULT_VIDEO_DECODER_POOL_SIZE, DecodeControl, VideoDecoderOwner, VideoDecoderPool,
    VideoPlane, is_decoder_startup_pressure, take_decoder_pressure,
};
use shrimply_video_decoder::{
    DecodeOutcome, DecodeRequest, DecodedVisual, PendingDecode, VideoDecoderHandle,
};

use crate::gpu::CudaVideoCompositor;
use crate::visual_source::{
    CompositeAccuracy, VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest,
};

pub struct VideoElement {
    decoder: VideoDecoderHandle,
    handoff_from: Option<VideoDecoderOwner>,
    prepared: Option<PreparedVideoFrame>,
}

struct PreparedVideoFrame {
    timeline_position: Time,
    source_position: Time,
    mode: MediaDecodeMode,
    generation: Option<u64>,
    decode: PendingDecode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaDecodeMode {
    BestEffort,
    Accurate,
    Continuous,
    LocalScrub,
}

impl MediaDecodeMode {
    const fn for_accuracy(accuracy: CompositeAccuracy) -> Self {
        if accuracy.continuous_playback() {
            Self::Continuous
        } else if accuracy.local_scrub() {
            Self::LocalScrub
        } else if accuracy.time_accurate() {
            Self::Accurate
        } else {
            Self::BestEffort
        }
    }

    const fn time_accurate(self) -> bool {
        !matches!(self, Self::BestEffort)
    }

    const fn realtime(self) -> bool {
        matches!(self, Self::Continuous | Self::LocalScrub)
    }

    const fn continuous(self) -> bool {
        matches!(self, Self::Continuous)
    }

    fn request(self, position: Time) -> DecodeRequest {
        match self {
            Self::BestEffort => DecodeRequest::best_effort(position),
            Self::Accurate => DecodeRequest::accurate(position),
            Self::Continuous => DecodeRequest::continuous(position),
            Self::LocalScrub => DecodeRequest::local_scrub(position),
        }
    }
}

fn decodable_source_position(item: &VideoItem, position: Time, frame_duration: Time) -> Time {
    shrimply_math_core::decodable_source_position(position, item.source_duration, frame_duration)
}

impl VideoElement {
    pub(crate) fn new(
        decoder: VideoDecoderHandle,
        handoff_from: Option<VideoDecoderOwner>,
    ) -> Self {
        Self {
            decoder,
            handoff_from,
            prepared: None,
        }
    }
}

impl VisualElement for VideoElement {
    fn matches(
        &self,
        item: &VideoItem,
        _canvas_size: shrimply_project::project::CanvasSize,
    ) -> bool {
        matches!(
            &item.content,
            shrimply_project::project::VideoItemContent::Media
        ) && self.decoder.matches(item)
    }

    fn prepare(
        &mut self,
        request: VisualPrepareRequest<'_>,
        _track_id: uuid::Uuid,
        _cache: &mut crate::visual_source::VisualSourceCache,
    ) -> Result<(), String> {
        let Some(source_position) =
            shrimply_project::project::video_source_time_at(request.item, request.position)
        else {
            self.prepared = None;
            return Ok(());
        };
        let source_position =
            decodable_source_position(request.item, source_position, self.decoder.frame_duration());
        let mode = MediaDecodeMode::for_accuracy(request.accuracy);
        let foreground = !request.prefetch;
        if foreground {
            self.decoder.touch_foreground();
        }
        let current = self.decoder.current();
        if usable_cached_frame(
            current.as_ref(),
            source_position,
            self.decoder.frame_duration(),
            mode,
        )
        .is_some()
        {
            self.prepared = None;
            return Ok(());
        }
        let handoff_from = foreground.then(|| self.handoff_from.clone()).flatten();
        let decode_request = mode
            .request(source_position)
            .handoff_from(handoff_from)
            .control(
                request
                    .decode_control
                    .filter(|_| !mode.continuous())
                    .cloned(),
            );
        if mode.realtime() {
            self.prepared = None;
            let submitted = self.decoder.try_latest(decode_request, foreground)?;
            shrimply_benchmarking::increment(if submitted {
                "Video decode / Prepared requests submitted"
            } else {
                "Video decode / Prepared requests busy"
            });
            return Ok(());
        }
        let generation = request.decode_control.map(DecodeControl::generation);
        if self.prepared.as_ref().is_some_and(|prepared| {
            prepared.timeline_position == request.position
                && prepared.source_position == source_position
                && prepared.mode == mode
                && prepared.generation == generation
        }) {
            return Ok(());
        }
        self.prepared = None;
        let decode = if foreground && mode.time_accurate() {
            Some(self.decoder.request(decode_request)?)
        } else {
            self.decoder.try_request(decode_request, foreground)?
        };
        shrimply_benchmarking::increment(if decode.is_some() {
            "Video decode / Prepared requests submitted"
        } else {
            "Video decode / Prepared requests busy"
        });
        self.prepared = decode.map(|decode| PreparedVideoFrame {
            timeline_position: request.position,
            source_position,
            mode,
            generation,
            decode,
        });
        Ok(())
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        _compositor: &mut CudaVideoCompositor,
        _track_id: uuid::Uuid,
        _cache: &mut crate::visual_source::VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let Some(source_position) =
            shrimply_project::project::video_source_time_at(request.item, request.position)
        else {
            return Ok(VisualRender::Empty);
        };
        let source_position =
            decodable_source_position(request.item, source_position, self.decoder.frame_duration());
        let mode = MediaDecodeMode::for_accuracy(request.accuracy);
        self.decoder.touch_foreground();
        let current = self.decoder.current();
        if let Some(frame) = usable_cached_frame(
            current.as_ref(),
            source_position,
            self.decoder.frame_duration(),
            mode,
        ) {
            self.prepared = None;
            shrimply_benchmarking::increment("Temporal decoder state / Current hit");
            return Ok(stabilized_frame(frame, request.item, request.state));
        }
        shrimply_benchmarking::increment("Temporal decoder state / Current miss");
        if mode.realtime() {
            self.prepared = None;
            let _ = self.decoder.try_latest(
                mode.request(source_position)
                    .handoff_from(self.handoff_from.clone())
                    .control(
                        request
                            .decode_control
                            .filter(|_| !mode.continuous())
                            .cloned(),
                    ),
                true,
            )?;
            shrimply_benchmarking::increment("Temporal decoder state / Dropped target");
            return current.map_or_else(
                || Ok(VisualRender::Loading(self.decoder.frame_size())),
                |frame| Ok(stabilized_frame(frame, request.item, request.state)),
            );
        }
        let generation = request.decode_control.map(DecodeControl::generation);
        let prepared = self.prepared.take().filter(|prepared| {
            prepared.timeline_position == request.position
                && prepared.source_position == source_position
                && prepared.mode == mode
                && prepared.generation == generation
        });
        let outcome = if let Some(prepared) = prepared {
            shrimply_benchmarking::increment("Video decode / Prepared request consumed");
            prepared.decode.receive()?
        } else {
            self.decoder
                .request(
                    mode.request(source_position)
                        .handoff_from(self.handoff_from.clone())
                        .control(request.decode_control.cloned()),
                )?
                .receive()?
        };
        let frame = match outcome {
            DecodeOutcome::Frame(frame) => frame,
            DecodeOutcome::Superseded(_) => {
                if !request
                    .decode_control
                    .is_some_and(DecodeControl::superseded)
                {
                    return Err(format!(
                        "video decoder silently superseded the current request: item={}, file={}, timeline_position={}, source_position={}, mode={mode:?}",
                        request.item.id,
                        request.item.file.display(),
                        request.position.as_label(),
                        source_position.as_label(),
                    ));
                }
                return Ok(VisualRender::Superseded);
            }
        };
        if request
            .decode_control
            .is_some_and(DecodeControl::superseded)
        {
            return Ok(VisualRender::Superseded);
        }
        match frame {
            Some(frame) => Ok(stabilized_frame(frame, request.item, request.state)),
            None => Ok(VisualRender::Loading(self.decoder.frame_size())),
        }
    }
}

fn usable_cached_frame(
    cached: Option<&DecodedVisual>,
    position: Time,
    frame_duration: Time,
    mode: MediaDecodeMode,
) -> Option<DecodedVisual> {
    if mode.time_accurate() {
        cached.filter(|frame| frame.0 == position).cloned()
    } else {
        cached
            .filter(|frame| {
                frame.0 <= position && position.saturating_sub(frame.0) < frame_duration
            })
            .cloned()
    }
}

fn stabilized_frame(
    frame: DecodedVisual,
    item: &VideoItem,
    state: crate::layer::VisualState,
) -> VisualRender {
    let (source_position, frame) = frame;
    let mut visual =
        crate::layer::RasterVisual::materialized(crate::layer::GpuFrame::Nv12(frame), state);
    if let Some(warp) = crate::video_stabilization::source_warp(item, source_position) {
        match warp {
            crate::video_stabilization::StabilizationWarp::Affine(source_transform) => {
                visual.push_preserving_pixel(crate::modifiers::stabilization_warp(source_transform))
            }
            crate::video_stabilization::StabilizationWarp::Mesh {
                grid_width,
                grid_height,
                source_offsets,
            } => visual.push_mesh_flow(grid_width, grid_height, source_offsets),
        }
    }
    VisualRender::Ready(crate::layer::Visual::Raster(visual))
}
