use super::*;

#[cfg(target_os = "macos")]
const FRAME_RENDER_DECODER_LIMIT: usize = 4;

pub(super) async fn analyze_transparent_fill(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    request: AnalyzeTransparentFillRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let address = shrimply_editing_api::query::model_item_address(&request.address)?;
    let modifier_id = Uuid::parse_str(&request.modifier_id)
        .map_err(|error| format!("invalid modifier_id {:?}: {error}", request.modifier_id))?;
    let mut project = live.borrow().clone();
    let original = content_revision(&project)?;
    let fill = project
        .video_item_mut(&address)
        .ok_or_else(|| "Transparent Fill analysis requires a video clip address".to_string())?
        .modifiers
        .iter_mut()
        .find(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| format!("modifier {modifier_id} does not exist on the addressed clip"))
        .and_then(|modifier| match &mut modifier.effect {
            ModifierEffect::Raster(effect) => match &mut **effect {
                RasterModifierEffect::TransparentFill(fill) => Ok(fill),
                _ => Err(format!("modifier {modifier_id} is not Transparent Fill")),
            },
            _ => Err(format!("modifier {modifier_id} is not Transparent Fill")),
        })?;
    if fill.points.is_empty() {
        return Err("add at least one transparent fill point before analyzing".to_string());
    }
    fill.analysis_generation = fill.analysis_generation.wrapping_add(1).max(1);
    let generation = fill.analysis_generation;
    let run_id = transparent_fill_analysis::analyze(project.clone(), &address, modifier_id)?;

    loop {
        if canceled.load(Ordering::Acquire) {
            transparent_fill_jobs::cancel(run_id);
            return Err("editing API client canceled Transparent Fill analysis".to_string());
        }
        let status = transparent_fill_jobs::status_for_run(run_id);
        match status {
            TransparentFillStatus::Running { .. } => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            TransparentFillStatus::Complete => break,
            TransparentFillStatus::Failed(error) => {
                return Err(format!("Transparent Fill analysis failed: {error}"));
            }
            TransparentFillStatus::Cancelled => {
                return Err("Transparent Fill analysis was canceled".to_string());
            }
            TransparentFillStatus::Missing => {
                transparent_fill_jobs::cancel(run_id);
                return Err(
                    "Transparent Fill inputs changed while analysis was running; retry analysis"
                        .to_string(),
                );
            }
        }
    }

    if canceled.load(Ordering::Acquire) {
        return Err("analysis was canceled before commit".into());
    }
    if content_revision(&live.borrow())? != original {
        return Err("project changed while analysis ran; retry analysis".into());
    }
    shrimply_project_document::project::commit_edit_saved(
        &project,
        "API analyze Transparent Fill",
    )?;
    *live.borrow_mut() = project;
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(AnalyzeTransparentFillResponse {
        address: request.address,
        modifier_id: request.modifier_id,
        analysis_generation: generation,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize Transparent Fill analysis result: {error}"))
}

pub(super) async fn view_frame(
    live: &Rc<RefCell<Project>>,
    frame: u64,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let project = live.borrow().clone();
    let position = time_from_frame(frame, project.fps)
        .ok_or_else(|| "frame exceeds the supported exact range".to_string())?;
    let fps = project.fps;
    #[cfg(not(target_os = "macos"))]
    let canvas = project.canvas_size;
    let (sender, receiver) = async_channel::bounded(1);
    let render_canceled = canceled.clone();
    thread::Builder::new()
        .name("shrimply-editing-frame".to_string())
        .spawn(move || {
            #[cfg(not(target_os = "macos"))]
            let result = shrimply_visual_cuda::compositor::render_project_rgba(
                &project,
                position,
                &render_canceled,
            )
            .map(|frame| frame.pixels);
            #[cfg(target_os = "macos")]
            let result = shrimply_preview_render_metal::render_png_cancelable(
                &project,
                position,
                FRAME_RENDER_DECODER_LIMIT,
                &render_canceled,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start editing API frame renderer: {error}"))?;

    let pixels = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled the frame render".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("editing API frame renderer stopped without a result".to_string());
            }
        }
    };
    #[cfg(not(target_os = "macos"))]
    let png = {
        let info = skia_safe::ImageInfo::new(
            (
                i32::try_from(canvas.width).map_err(|_| "canvas width is too large")?,
                i32::try_from(canvas.height).map_err(|_| "canvas height is too large")?,
            ),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let image = skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(&pixels),
            canvas.width as usize * std::mem::size_of::<u32>(),
        )
        .ok_or("could not construct frame image")?;
        let png = image
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .ok_or("could not encode frame PNG")?;
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(png.as_bytes())
    };
    #[cfg(target_os = "macos")]
    let png = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(pixels)
    };
    serde_json::to_value(ViewFrameResponse {
        frame: shrimply_editing_api::query::frame_time(frame, fps)?,
        png,
    })
    .map_err(|error| format!("could not serialize rendered frame: {error}"))
}
