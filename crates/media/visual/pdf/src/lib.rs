use cairo::{Context, Format, ImageSurface};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, sync_channel};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageSize {
    pub width: u32,
    pub height: u32,
}

pub struct RenderedPage {
    pub size: PageSize,
    pub rgba: Vec<u8>,
}

pub struct PreparedDocument {
    commands: Sender<Command>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    RenderPage(
        u32,
        std::sync::mpsc::SyncSender<Result<RenderedPage, String>>,
    ),
    Shutdown,
}

impl PreparedDocument {
    pub fn new(bytes: Vec<u8>) -> Result<Self, String> {
        let (commands, receiver) = std::sync::mpsc::channel();
        let (initialized, initialization) = sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("shrimply-pdf-document".to_string())
            .spawn(move || document_worker(bytes, receiver, initialized))
            .map_err(|error| format!("start PDF document worker: {error}"))?;
        initialization
            .recv()
            .map_err(|_| "PDF document worker stopped during initialization".to_string())??;
        Ok(Self {
            commands,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn render_page(&self, page_index: u32) -> Result<RenderedPage, String> {
        let (sender, result) = sync_channel(1);
        self.commands
            .send(Command::RenderPage(page_index, sender))
            .map_err(|_| "PDF document worker stopped before rendering".to_string())?;
        result
            .recv()
            .map_err(|_| "PDF document worker stopped while rendering".to_string())?
    }
}

impl Drop for PreparedDocument {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self
            .worker
            .lock()
            .expect("PDF document worker mutex poisoned")
            .take()
        {
            let _ = worker.join();
        }
    }
}

pub fn page_sizes(bytes: Vec<u8>) -> Result<Vec<PageSize>, String> {
    let document = document(bytes)?;
    let page_count = usize::try_from(document.n_pages())
        .map_err(|_| "PDF reported a negative page count".to_string())?;
    if page_count == 0 {
        return Err("PDF contains no pages".to_string());
    }
    (0..page_count)
        .map(|index| {
            let page = document
                .page(i32::try_from(index).expect("Poppler page index must fit i32"))
                .ok_or_else(|| format!("PDF page {} does not exist", index + 1))?;
            page_size(page.size())
        })
        .collect()
}

pub fn render_page(bytes: Vec<u8>, page_index: u32) -> Result<RenderedPage, String> {
    let document = document(bytes)?;
    render_document_page(&document, page_index)
}

fn render_document_page(
    document: &poppler::Document,
    page_index: u32,
) -> Result<RenderedPage, String> {
    let page = document
        .page(i32::try_from(page_index).map_err(|_| "PDF page index is too large")?)
        .ok_or_else(|| format!("PDF page {} does not exist", page_index.saturating_add(1)))?;
    let size = page_size(page.size())?;
    let width = i32::try_from(size.width).map_err(|_| "PDF page width is too large")?;
    let height = i32::try_from(size.height).map_err(|_| "PDF page height is too large")?;
    let surface = ImageSurface::create(Format::ARgb32, width, height)
        .map_err(|error| format!("create PDF page surface: {error}"))?;
    let context = Context::new(&surface)
        .map_err(|error| format!("create PDF page drawing context: {error}"))?;
    context.set_source_rgb(1.0, 1.0, 1.0);
    context
        .paint()
        .map_err(|error| format!("clear PDF page surface: {error}"))?;
    page.render(&context);
    context
        .status()
        .map_err(|error| format!("render PDF page: {error}"))?;
    drop(context);

    let stride = usize::try_from(surface.stride()).map_err(|_| "invalid PDF surface stride")?;
    let data = surface
        .take_data()
        .map_err(|error| format!("read rendered PDF page: {error}"))?;
    let row_bytes = usize::try_from(size.width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or("PDF page row size overflow")?;
    let capacity = row_bytes
        .checked_mul(usize::try_from(size.height).expect("page height must fit usize"))
        .ok_or("PDF page pixel size overflow")?;
    let mut rgba = Vec::with_capacity(capacity);
    for row in data.chunks(stride).take(size.height as usize) {
        for pixel in row[..row_bytes].as_chunks::<4>().0 {
            #[cfg(target_endian = "little")]
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            #[cfg(target_endian = "big")]
            rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
        }
    }
    Ok(RenderedPage { size, rgba })
}

fn document_worker(
    bytes: Vec<u8>,
    commands: Receiver<Command>,
    initialized: std::sync::mpsc::SyncSender<Result<(), String>>,
) {
    let document = match document(bytes) {
        Ok(document) => {
            if initialized.send(Ok(())).is_err() {
                return;
            }
            document
        }
        Err(error) => {
            let _ = initialized.send(Err(error));
            return;
        }
    };
    while let Ok(command) = commands.recv() {
        match command {
            Command::RenderPage(page, result) => {
                let _ = result.send(render_document_page(&document, page));
            }
            Command::Shutdown => break,
        }
    }
}

fn document(bytes: Vec<u8>) -> Result<poppler::Document, String> {
    poppler::Document::from_bytes(&glib::Bytes::from_owned(bytes), None)
        .map_err(|error| format!("could not open PDF: {error}"))
}

fn page_size((width, height): (f64, f64)) -> Result<PageSize, String> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("PDF page has invalid dimensions".to_string());
    }
    if width.ceil() > f64::from(i32::MAX) || height.ceil() > f64::from(i32::MAX) {
        return Err("PDF page dimensions are too large".to_string());
    }
    Ok(PageSize {
        width: width.ceil() as u32,
        height: height.ceil() as u32,
    })
}
