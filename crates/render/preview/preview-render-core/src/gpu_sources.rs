use crate::gpu::{Buffer, ImportedFrame, Renderer};
use shrimply_preview_render_core::{DecodedImage, rgba_pixels};
use shrimply_visual_frame::{GpuFrame, VideoFrame};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Sources {
    images: HashMap<u32, Buffer>,
    videos: HashMap<usize, ImportedFrame>,
    used_images: HashSet<u32>,
    used_videos: HashSet<usize>,
}

impl Sources {
    pub(super) fn begin_frame(&mut self) {
        self.used_images.clear();
        self.used_videos.clear();
    }

    pub(super) fn retain_used(&mut self) {
        self.images.retain(|key, _| self.used_images.contains(key));
        self.videos.retain(|key, _| self.used_videos.contains(key));
    }

    pub(super) fn image(
        &mut self,
        renderer: &Renderer,
        image: &skia_safe::Image,
    ) -> Result<Buffer, String> {
        let key = image.unique_id();
        self.used_images.insert(key);
        if let Some(buffer) = self.images.get(&key) {
            return Ok(buffer.clone());
        }
        let buffer = renderer.upload(&rgba_pixels(image)?)?;
        self.images.insert(key, buffer.clone());
        Ok(buffer)
    }

    pub(super) fn video(
        &mut self,
        renderer: &mut Renderer,
        video: &Arc<VideoFrame>,
    ) -> Result<Buffer, String> {
        let key = Arc::as_ptr(video) as usize;
        self.used_videos.insert(key);
        if let Some(imported) = self.videos.get(&key) {
            return Ok(imported.output.clone());
        }
        let imported = ImportedFrame::import(renderer, video.clone())?;
        let buffer = imported.output.clone();
        self.videos.insert(key, imported);
        Ok(buffer)
    }

    pub(super) fn decoded(
        &mut self,
        renderer: &mut Renderer,
        image: &DecodedImage,
    ) -> Result<Buffer, String> {
        match image {
            DecodedImage::Raster(image) => self.image(renderer, image),
            DecodedImage::Video(video) => self.video(renderer, video),
        }
    }
}
