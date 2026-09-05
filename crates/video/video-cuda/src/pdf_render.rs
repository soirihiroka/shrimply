use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_gpu_memory::{ResourceKey, global as gpu_memory};
use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use uuid::Uuid;

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{
    VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest, VisualSourceCache,
};

pub struct PdfRenderSession {
    file: Asset,
    snapshot: AssetSnapshot,
    page: u32,
    width: u32,
    height: u32,
    page_pending: Option<Receiver<Result<VisualFrame, String>>>,
}

impl PdfRenderSession {
    pub fn new(item: &VideoItem) -> Result<Self, String> {
        let VideoItemContent::Pdf(pdf) = &item.content else {
            return Err("PDF renderer received a non-PDF item".to_string());
        };
        Ok(Self {
            file: item.file.clone(),
            snapshot: item.file.snapshot()?,
            page: pdf.page,
            width: item.source_width.max(1),
            height: item.source_height.max(1),
            page_pending: None,
        })
    }

    fn document_key(&self) -> ResourceKey {
        let mut discriminator = Vec::new();
        discriminator.extend_from_slice(b"pdf-document\0");
        discriminator.extend_from_slice(self.snapshot.cache_key().as_bytes());
        ResourceKey::new(self.snapshot.path().to_path_buf(), discriminator)
    }

    fn page_key(&self) -> ResourceKey {
        let mut discriminator = Vec::new();
        discriminator.extend_from_slice(b"pdf-page-gpu\0");
        discriminator.extend_from_slice(self.snapshot.cache_key().as_bytes());
        discriminator.extend_from_slice(&self.page.to_le_bytes());
        ResourceKey::new(self.snapshot.path().to_path_buf(), discriminator)
    }

    fn request_document(&self) {
        let key = self.document_key();
        if !gpu_memory().begin_resource_load(key.clone()) {
            return;
        }
        let file = self.file.clone();
        let snapshot = self.snapshot.clone();
        rayon::spawn(move || {
            let result = snapshot.read().and_then(|bytes| {
                let source_bytes = u64::try_from(bytes.len())
                    .map_err(|_| "PDF source size exceeds u64".to_string())?;
                let document = shrimply_pdf::PreparedDocument::new(bytes)?;
                snapshot.ensure_current()?;
                Ok((source_bytes, document))
            });
            let (bytes, result) = match result {
                Ok((bytes, document)) => (bytes, Ok(document)),
                Err(error) => (0, Err(error)),
            };
            if let Err(error) = gpu_memory().finish_resource_load(key, bytes, result) {
                tracing::error!(file = %file.display(), %error, "could not finish preparing PDF source");
            }
        });
        shrimply_benchmarking::increment("PDF source residency / Miss");
    }

    fn request_page(&mut self) -> Result<(), String> {
        if self.page_pending.is_some() || gpu_memory().contains_resource(&self.page_key()) {
            return Ok(());
        }
        let Some(document) =
            gpu_memory().get_resource::<shrimply_pdf::PreparedDocument>(&self.document_key())?
        else {
            self.request_document();
            return Ok(());
        };
        let page = self.page;
        let (sender, result) = sync_channel(1);
        self.page_pending = Some(result);
        rayon::spawn(move || {
            let result = document.render_page(page).and_then(|rendered| {
                VisualFrame::from_rgba_bytes(
                    rendered.size.width,
                    rendered.size.height,
                    rendered.rgba,
                )
            });
            let _ = sender.send(result);
        });
        Ok(())
    }

    fn rendered_page(&mut self) -> Result<Option<VisualFrame>, String> {
        let Some(pending) = &self.page_pending else {
            return Ok(None);
        };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err("PDF page renderer stopped unexpectedly".to_string())
            }
        };
        self.page_pending = None;
        result.map(Some)
    }
}

impl VisualElement for PdfRenderSession {
    fn matches(&self, item: &VideoItem, _canvas_size: CanvasSize) -> bool {
        let VideoItemContent::Pdf(pdf) = &item.content else {
            return false;
        };
        self.file == item.file
            && self.snapshot.is_current()
            && self.page == pdf.page
            && self.width == item.source_width.max(1)
            && self.height == item.source_height.max(1)
    }

    fn prepare(
        &mut self,
        _request: VisualPrepareRequest<'_>,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<(), String> {
        self.request_page()
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        if let Some(frame) = gpu_memory().get_resource::<VisualFrame>(&self.page_key())? {
            shrimply_benchmarking::increment("PDF GPU residency / Hit");
            compositor.prepare_host_backed_frame(&frame, "cached PDF page preview")?;
            return Ok(VisualRender::Ready(Visual::Raster(
                RasterVisual::materialized(
                    GpuFrame::Rgba(Rc::new((*frame).clone())),
                    request.state,
                ),
            )));
        }
        if let Some(frame) = self.rendered_page()? {
            shrimply_benchmarking::increment("PDF GPU residency / Miss");
            let mut frame = compositor.upload_frame(&frame)?;
            if let Some(retained) =
                compositor.retain_host_backed_frame(&frame, "PDF page preview cache")?
            {
                gpu_memory().insert_resource(self.page_key(), 0, retained.clone())?;
                frame = retained;
            }
            compositor.prepare_host_backed_frame(&frame, "PDF page preview")?;
            return Ok(VisualRender::Ready(Visual::Raster(
                RasterVisual::materialized(GpuFrame::Rgba(Rc::new(frame)), request.state),
            )));
        }
        shrimply_benchmarking::increment("PDF GPU residency / Miss");
        self.request_page()?;
        Ok(VisualRender::Loading(CanvasSize {
            width: self.width,
            height: self.height,
        }))
    }
}
