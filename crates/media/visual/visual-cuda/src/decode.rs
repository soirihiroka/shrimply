use shrimply_visual_frame::{GpuFrame, cuda::ImportedFrame};

use shrimply_project_document::project::{Time, VideoItem};
pub use shrimply_video_decoder::{
    DEFAULT_VIDEO_DECODER_POOL_SIZE, DecodeControl, VideoDecoderOwner, VideoDecoderPool,
    VideoPlane, is_decoder_startup_pressure, take_decoder_pressure,
};
use shrimply_video_decoder::{
    DecodeMode, DecodeOutcome, DecodeRequest, DecodedVisual, PendingDecode, VideoDecoderHandle,
};

use crate::gpu::CudaVideoCompositor;
use crate::visual_source::{
    CompositeAccuracy, VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest,
};

pub struct VideoElement {
    decoder: VideoDecoderHandle,
    handoff_from: Option<VideoDecoderOwner>,
    prepared: Option<PreparedVideoFrame>,
    imported: Option<ImportedFrame>,
}

struct PreparedVideoFrame {
    timeline_position: Time,
    source_position: Time,
    mode: DecodeMode,
    generation: Option<u64>,
    decode: PendingDecode,
}

fn decode_mode(accuracy: CompositeAccuracy) -> DecodeMode {
    if accuracy.continuous_playback() {
        DecodeMode::Continuous
    } else if accuracy.local_scrub() {
        DecodeMode::LocalScrub
    } else if accuracy.time_accurate() {
        DecodeMode::Accurate
    } else {
        DecodeMode::BestEffort
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
            imported: None,
        }
    }
}

impl VisualElement for VideoElement {
    fn matches(
        &self,
        item: &VideoItem,
        _canvas_size: shrimply_project_document::project::CanvasSize,
    ) -> bool {
        matches!(
            &item.content,
            shrimply_project_document::project::VideoItemContent::Media
        ) && self.decoder.matches(item)
    }

    fn prepare(
        &mut self,
        request: VisualPrepareRequest<'_>,
        _track_id: uuid::Uuid,
        _cache: &mut crate::visual_source::VisualSourceCache,
    ) -> Result<(), String> {
        let Some(source_position) = shrimply_project_document::project::video_source_time_at(
            request.item,
            request.position,
        ) else {
            self.prepared = None;
            return Ok(());
        };
        let source_position =
            decodable_source_position(request.item, source_position, self.decoder.frame_duration());
        let mode = decode_mode(request.accuracy);
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
        let decode_request = DecodeRequest::new(source_position, mode)
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
            shrimply_profiling::increment(if submitted {
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
        shrimply_profiling::increment(if decode.is_some() {
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
        compositor: &mut CudaVideoCompositor,
        _track_id: uuid::Uuid,
        _cache: &mut crate::visual_source::VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let Some(source_position) = shrimply_project_document::project::video_source_time_at(
            request.item,
            request.position,
        ) else {
            return Ok(VisualRender::Empty);
        };
        let source_position =
            decodable_source_position(request.item, source_position, self.decoder.frame_duration());
        let mode = decode_mode(request.accuracy);
        self.decoder.touch_foreground();
        let current = self.decoder.current();
        if let Some(frame) = usable_cached_frame(
            current.as_ref(),
            source_position,
            self.decoder.frame_duration(),
            mode,
        ) {
            self.prepared = None;
            shrimply_profiling::increment("Temporal decoder state / Current hit");
            return self.stabilized_frame(frame, &request, compositor);
        }
        shrimply_profiling::increment("Temporal decoder state / Current miss");
        if mode.realtime() {
            self.prepared = None;
            let _ = self.decoder.try_latest(
                DecodeRequest::new(source_position, mode)
                    .handoff_from(self.handoff_from.clone())
                    .control(
                        request
                            .decode_control
                            .filter(|_| !mode.continuous())
                            .cloned(),
                    ),
                true,
            )?;
            shrimply_profiling::increment("Temporal decoder state / Dropped target");
            return match current {
                Some(frame) => self.stabilized_frame(frame, &request, compositor),
                None => Ok(VisualRender::Loading(self.decoder.frame_size())),
            };
        }
        let generation = request.decode_control.map(DecodeControl::generation);
        let prepared = self.prepared.take().filter(|prepared| {
            prepared.timeline_position == request.position
                && prepared.source_position == source_position
                && prepared.mode == mode
                && prepared.generation == generation
        });
        let outcome = if let Some(prepared) = prepared {
            shrimply_profiling::increment("Video decode / Prepared request consumed");
            prepared.decode.receive()?
        } else {
            self.decoder
                .request(
                    DecodeRequest::new(source_position, mode)
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
            Some(frame) => self.stabilized_frame(frame, &request, compositor),
            None => Ok(VisualRender::Loading(self.decoder.frame_size())),
        }
    }
}

fn usable_cached_frame(
    cached: Option<&DecodedVisual>,
    position: Time,
    frame_duration: Time,
    mode: DecodeMode,
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

impl VideoElement {
    fn stabilized_frame(
        &mut self,
        frame: DecodedVisual,
        request: &VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<VisualRender, String> {
        let item = request.item;
        let state = request.state;
        let (source_position, source) = frame;
        if self
            .imported
            .as_ref()
            .is_none_or(|imported| !std::sync::Arc::ptr_eq(imported.source(), &source))
        {
            self.imported = Some(ImportedFrame::import(&mut compositor.context, source)?);
        }
        let frame = self
            .imported
            .as_ref()
            .expect("imported CUDA video frame")
            .frame
            .clone();
        let mut visual =
            crate::layer::RasterVisual::materialized(crate::layer::GpuFrame::Nv12(frame), state);
        if let Some(warp) = crate::video_stabilization::source_warp(item, source_position) {
            match warp {
                crate::video_stabilization::StabilizationWarp::Affine(source_transform) => visual
                    .push_preserving_pixel(crate::modifiers::stabilization_warp(source_transform)),
                crate::video_stabilization::StabilizationWarp::Mesh {
                    grid_width,
                    grid_height,
                    source_offsets,
                } => visual.push_mesh_flow(grid_width, grid_height, source_offsets),
            }
        }
        Ok(VisualRender::Ready(crate::layer::Visual::Raster(visual)))
    }
}
