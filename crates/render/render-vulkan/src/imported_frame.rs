use crate::{Buffer, Renderer, Submission};
use shrimply_visual_frame::{GpuFrame, VideoFrame};
use std::sync::Arc;

pub struct ImportedFrame {
    // Wait for GPU readers before releasing the native decoder surface.
    pub submission: Submission,
    pub output: Buffer,
    source: Arc<VideoFrame>,
}

impl GpuFrame for ImportedFrame {
    type Context = Renderer;

    fn import(renderer: &mut Renderer, source: Arc<VideoFrame>) -> Result<Self, String> {
        let (output, submission) = renderer.import_video(&source)?;
        Ok(Self {
            source,
            output,
            submission,
        })
    }

    fn source(&self) -> &Arc<VideoFrame> {
        &self.source
    }
}
