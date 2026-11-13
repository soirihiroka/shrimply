use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_project_frame(
    project: &Project,
    position: Time,
    sessions: &mut RenderSessions,
    cache: &mut RenderCache,
    compositor: &mut CudaVideoCompositor,
    mode: RenderMode,
    audio_analysis: &FrameAudioAnalysis,
    item_ids: Option<&[Uuid]>,
    capture_track: Option<&shrimply_project_document::project::TrackAddress>,
    cache_item: Option<&ItemAddress>,
    snap_cache_item: bool,
    excluded_item_id: Option<Uuid>,
    decode_control: Option<&DecodeControl>,
) -> RenderedFrame {
    let _measurement = shrimply_profiling::measure("Video / Render frame");
    sessions.active_elements.clear();
    compositor.set_render_control(None);
    if decode_control.is_some_and(DecodeControl::superseded) {
        return RenderedFrame {
            frame: None,
            audio_analysis: audio_analysis.clone(),
            loading: false,
            loading_placeholder: false,
            clear: false,
            errors: Vec::new(),
            manim_updates: Vec::new(),
            superseded: true,
        };
    }
    let mut errors = Vec::new();
    let manim_updates = Vec::new();
    let mut active_items = Vec::new();
    let preload_during_playback = mode.accuracy().continuous_playback();
    let selected_needs_background = item_ids.is_some_and(|item_ids| {
        project
            .video_tracks
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| {
                item_ids.contains(&item.id)
                    && matches!(
                        item.content,
                        shrimply_project_document::project::VideoItemContent::Obj(_)
                    )
            })
    });
    let active_filter = if selected_needs_background {
        None
    } else {
        item_ids
    };

    if preload_during_playback {
        preload::upcoming_images(project, sessions, position, mode.accuracy());
    }

    active_items.extend(shrimply_visual_core::sequence::active_tracks(
        &project.video_tracks,
        position,
        active_filter,
    ));

    if selected_needs_background && let Some(item_ids) = item_ids {
        let selected_end = active_items
            .iter()
            .rposition(|active| item_ids.contains(&active.item.id))
            .map_or(0, |index| index + 1);
        active_items.truncate(selected_end);
    }
    active_items.retain(|active| Some(active.item.id) != excluded_item_id);
    if let Some(shrimply_project_document::project::TrackAddress::Video {
        sequence_path,
        track_id,
    }) = capture_track
    {
        if let Some(root_item_id) = sequence_path.first() {
            active_items.retain(|active| active.item.id == *root_item_id);
        } else {
            active_items.retain(|active| active.track_id == *track_id);
        }
    }
    if let Some(cache_item_id) = cache_item.filter(|address| address.sequence_path().is_empty()) {
        for active in &mut active_items {
            if active.track_id == cache_item_id.track_id()
                && active.item.id == cache_item_id.item_id()
            {
                active.clip_transition = None;
            }
        }
    }

    if active_items.is_empty() {
        if preload_during_playback {
            preload::upcoming_videos(project, sessions, position, mode.accuracy());
        }
        return RenderedFrame {
            frame: None,
            audio_analysis: audio_analysis.clone(),
            loading: false,
            loading_placeholder: false,
            clear: true,
            errors,
            manim_updates,
            superseded: false,
        };
    }

    compositor.set_render_control(decode_control.cloned());
    let mut renderer = FrameItemRenderer {
        project,
        position,
        sessions,
        cache,
        compositor,
        mode,
        audio_analysis: audio_analysis.clone(),
        loading: false,
        loading_placeholder: false,
        mask_layers: Vec::new(),
        alpha_mask_layers: HashMap::new(),
        render_stack: Vec::new(),
        sequence_stack: Vec::new(),
        sequence_path: Vec::new(),
        scope_positions: vec![position],
        manim_updates: Vec::new(),
        decode_control,
        superseded: false,
        clip_transition: None,
        cache_item: cache_item.cloned(),
        capture_track: capture_track.cloned(),
        snap_cache_item,
        excluded_item_id,
        sanim_sources: Default::default(),
    };
    if mode.prepare_active_sources() {
        renderer.preload_active_sources(&active_items);
    }
    if preload_during_playback {
        abort_render_if_superseded!(renderer.decode_control, renderer.superseded = true);
        if !renderer.superseded {
            preload::upcoming_videos(project, renderer.sessions, position, mode.accuracy());
        }
    }
    let mut layers = Vec::with_capacity(active_items.len());
    let mut selected_layers = Vec::new();
    let mut morphed_items = HashSet::new();
    for (active_index, active) in active_items.iter().enumerate() {
        abort_render_if_superseded!(renderer.decode_control, {
            renderer.superseded = true;
            break;
        });
        if morphed_items.contains(&active.item.id) {
            continue;
        }
        if let Some(endpoint) =
            shrimply_visual_core::sequence::morph_endpoint(&active_items, active_index)
            && active
                .clip_transition
                .is_some_and(|transition| transition.role == ClipTransitionRole::Outgoing)
        {
            let transition = active
                .clip_transition
                .expect("Morph endpoint must have a transition");
            let incoming = &active_items[endpoint.peer_index];
            morphed_items.insert(incoming.item.id);
            match renderer.render_morph_pair(
                active.track_index,
                active.track_id,
                active.item,
                incoming.item,
                transition.definition.duration,
                transition.progress,
                &layers,
            ) {
                Ok(morph_layers) => {
                    if item_ids.is_some_and(|ids| {
                        ids.contains(&active.item.id) || ids.contains(&incoming.item.id)
                    }) {
                        selected_layers.extend(morph_layers.iter().cloned());
                    }
                    layers.extend(morph_layers);
                }
                Err(error) => {
                    abort_render_if_superseded!(renderer.decode_control, {
                        renderer.superseded = true;
                        break;
                    });
                    errors.push(format!(
                        "Could not render Morph transition from {} to {}: {error}",
                        active.item.id, incoming.item.id
                    ));
                }
            }
            if renderer.loading || renderer.superseded {
                break;
            }
            continue;
        }
        let held_item = shrimply_visual_core::clip_transition::held_item(
            active.item,
            position,
            active.clip_transition.is_some(),
        );
        let item = held_item.as_ref();
        let address = ItemAddress::Video {
            sequence_path: Vec::new(),
            track_id: active.track_id,
            item_id: item.id,
        };
        let cached_item = match shrimply_visual_core::modifier_cache::effective_item(
            &address,
            item,
            project.canvas_size,
        ) {
            Ok(item) => item,
            Err(error) => {
                errors.push(format!(
                    "Could not load visual cache for item {}: {error}",
                    item.id
                ));
                continue;
            }
        };
        let item = cached_item.as_ref().unwrap_or(item);
        let previous = cached_item.is_none().then_some(active.previous).flatten();
        let routes = renderer.decode_routes(active.track_id, previous, item);
        renderer.clip_transition = active.clip_transition;
        let item_measurement = match &item.content {
            shrimply_project_document::project::VideoItemContent::Media => "Video item / Media",
            shrimply_project_document::project::VideoItemContent::Image => "Video item / Image",
            shrimply_project_document::project::VideoItemContent::Gif => "Video item / GIF",
            shrimply_project_document::project::VideoItemContent::Svg => "Video item / SVG",
            shrimply_project_document::project::VideoItemContent::Pdf(_) => "Video item / PDF",
            shrimply_project_document::project::VideoItemContent::Manim(_) => "Video item / Manim",
            shrimply_project_document::project::VideoItemContent::Sanim(_) => "Video item / Sanim",
            shrimply_project_document::project::VideoItemContent::Blender(_) => {
                "Video item / Blender"
            }
            shrimply_project_document::project::VideoItemContent::LayeredImage(_) => {
                "Video item / Layered image"
            }
            shrimply_project_document::project::VideoItemContent::Text(_) => "Video item / Text",
            shrimply_project_document::project::VideoItemContent::Shape(_) => "Video item / Shape",
            shrimply_project_document::project::VideoItemContent::Paint(_) => "Video item / Paint",
            shrimply_project_document::project::VideoItemContent::Background(_) => {
                "Video item / Background"
            }
            shrimply_project_document::project::VideoItemContent::Obj(_) => "Video item / OBJ",
            shrimply_project_document::project::VideoItemContent::Gaussian(_) => {
                "Video item / Gaussian"
            }
            shrimply_project_document::project::VideoItemContent::FoldedSequence(_) => {
                "Video item / Folded sequence"
            }
        };
        let transmission_background = match renderer.render_scene_background(item, &layers) {
            Ok(background) => background,
            Err(error) => {
                abort_render_if_superseded!(renderer.decode_control, {
                    renderer.superseded = true;
                    break;
                });
                errors.push(format!(
                    "Could not render visual item {} transmission background: {error}",
                    item.id
                ));
                None
            }
        };
        let rendered = {
            let _measurement = shrimply_profiling::measure(item_measurement);
            renderer.render_item(
                (active.track_index, active.track_id),
                item,
                cached_item.as_ref().map(|_| held_item.as_ref()),
                routes,
                cache_item.is_some_and(|address| {
                    address
                        .sequence_path()
                        .first()
                        .copied()
                        .unwrap_or_else(|| address.item_id())
                        == item.id
                }),
                transmission_background.as_deref(),
            )
        };
        match rendered {
            Ok(Some(layer)) => {
                if item_ids.is_some_and(|ids| ids.contains(&active.item.id)) {
                    selected_layers.push(layer.clone());
                }
                layers.push(layer);
                if let Some((color, opacity)) = active
                    .clip_transition
                    .and_then(shrimply_visual_core::clip_transition::color_layer)
                {
                    match solid_video_layer(
                        renderer.compositor,
                        project.canvas_size,
                        color,
                        opacity,
                    ) {
                        Ok(layer) => {
                            if item_ids.is_some_and(|ids| ids.contains(&active.item.id)) {
                                selected_layers.push(layer.clone());
                            }
                            layers.push(layer);
                        }
                        Err(error) => {
                            abort_render_if_superseded!(renderer.decode_control, {
                                renderer.superseded = true;
                                break;
                            });
                            errors.push(error);
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                abort_render_if_superseded!(renderer.decode_control, {
                    renderer.superseded = true;
                    break;
                });
                let error = format!("Could not render visual item {}: {error}", item.id);
                errors.push(error);
            }
        }
        if renderer.loading {
            break;
        }
        abort_render_if_superseded!(renderer.decode_control, {
            renderer.superseded = true;
            break;
        });
    }

    let mut loading = renderer.loading;
    if audio_analysis.mouth.pending()
        && matches!(mode, RenderMode::Preview { accuracy } if accuracy.content_accurate())
    {
        loading = true;
    }
    errors.extend(audio_analysis.mouth.failures());
    if loading && matches!(mode, RenderMode::ExportContentAccurate { .. }) {
        errors.push(EXPORT_ASSETS_LOADING.to_string());
    }
    let output_layers = if item_ids.is_some() {
        &selected_layers
    } else {
        &layers
    };
    abort_render_if_superseded!(renderer.decode_control, renderer.superseded = true);
    let frame = if renderer.superseded
        || (loading && !renderer.loading_placeholder)
        || output_layers.is_empty()
    {
        None
    } else {
        let result = {
            let _measurement = shrimply_profiling::measure("Video / Final composition");
            match mode {
                RenderMode::Preview { .. } => renderer
                    .compositor
                    .render(project.canvas_size, output_layers),
                RenderMode::ExportContentAccurate {
                    background_alpha, ..
                } => renderer.compositor.render_export(
                    project.canvas_size,
                    output_layers,
                    background_alpha,
                ),
            }
        };
        match result {
            Ok(mut frame) => {
                frame.sanim_sources = Arc::new(std::mem::take(&mut renderer.sanim_sources));
                Some(frame)
            }
            Err(error) => {
                abort_render_if_superseded!(renderer.decode_control, renderer.superseded = true);
                if !renderer.superseded {
                    errors.push(error);
                }
                None
            }
        }
    };

    // A render failure must not turn into a successful empty frame. The preview
    // keeps its last completed frame while the source/compositor recovers. An
    // empty layer set without an error is the only active-item case that
    // intentionally clears the canvas; the no-active-items case returned above.
    let clear = frame.is_none()
        && !loading
        && !renderer.superseded
        && errors.is_empty()
        && output_layers.is_empty();
    let superseded = renderer.superseded;
    renderer.compositor.set_render_control(None);
    RenderedFrame {
        frame,
        audio_analysis: audio_analysis.clone(),
        loading,
        loading_placeholder: renderer.loading_placeholder,
        clear,
        errors,
        manim_updates: renderer
            .manim_updates
            .into_iter()
            .chain(manim_updates)
            .collect(),
        superseded,
    }
}

pub(super) struct FrameItemRenderer<'a> {
    pub(super) project: &'a Project,
    pub(super) position: Time,
    pub(super) sessions: &'a mut RenderSessions,
    cache: &'a mut RenderCache,
    compositor: &'a mut CudaVideoCompositor,
    pub(super) mode: RenderMode,
    audio_analysis: FrameAudioAnalysis,
    loading: bool,
    loading_placeholder: bool,
    mask_layers: Vec<(
        shrimply_visual_core::raster_modifiers::ExternalDependency,
        Rc<crate::gpu::VisualFrame>,
    )>,
    alpha_mask_layers: HashMap<(Vec<Uuid>, Uuid, u32), Rc<crate::gpu::VisualFrame>>,
    render_stack: Vec<ItemAddress>,
    pub(super) sequence_stack: Vec<Uuid>,
    pub(super) sequence_path: Vec<Uuid>,
    scope_positions: Vec<Time>,
    manim_updates: Vec<shrimply_manim_state::Update>,
    pub(super) decode_control: Option<&'a DecodeControl>,
    superseded: bool,
    pub(super) clip_transition: Option<ActiveClipTransition>,
    pub(super) cache_item: Option<ItemAddress>,
    capture_track: Option<shrimply_project_document::project::TrackAddress>,
    pub(super) snap_cache_item: bool,
    excluded_item_id: Option<Uuid>,
    sanim_sources: shrimply_visual_core::sanim::inspection::FrameSources,
}

impl FrameItemRenderer<'_> {
    fn render_scene_background(
        &mut self,
        item: &VideoItem,
        layers: &[crate::layer::VideoLayer],
    ) -> Result<Option<Rc<crate::gpu::VisualFrame>>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        if !matches!(
            item.content,
            shrimply_project_document::project::VideoItemContent::Obj(_)
        ) || layers.is_empty()
        {
            return Ok(None);
        }
        let background_alpha = match self.mode {
            RenderMode::Preview { .. } => None,
            RenderMode::ExportContentAccurate {
                background_alpha, ..
            } => Some(background_alpha),
        };
        self.compositor
            .render_layers_to_rgba(self.project.canvas_size, layers, background_alpha)
            .map(Some)
    }

    fn render_folded_sequence(
        &mut self,
        item: &VideoItem,
        reference: SequenceReference,
        state: VisualState,
    ) -> Result<VisualRender, String> {
        let Some((sequence, position)) = shrimply_visual_core::sequence::resolve(
            self.project,
            item,
            reference,
            self.position,
            &self.sequence_stack,
        )?
        else {
            return Ok(VisualRender::Empty);
        };
        let mut active = {
            let _measurement =
                shrimply_profiling::measure("Folded sequence / Resolve active items");
            shrimply_visual_core::sequence::active_tracks(&sequence.video_tracks, position, None)
                .into_iter()
                .filter(|active| Some(active.item.id) != self.excluded_item_id)
                .map(|active| {
                    (
                        active.track_index,
                        active.track_id,
                        active.item.clone(),
                        active.clip_transition,
                        active.previous.cloned(),
                    )
                })
                .collect::<Vec<_>>()
        };
        if let Some(address) = self.cache_item.as_ref()
            && address
                .sequence_path()
                .get(self.sequence_path.len())
                .is_some_and(|host_id| *host_id == item.id)
        {
            let child_depth = self.sequence_path.len() + 1;
            let child_id = address
                .sequence_path()
                .get(child_depth)
                .copied()
                .unwrap_or_else(|| address.item_id());
            let final_child = child_depth == address.sequence_path().len();
            if final_child
                && self.project.video_item(address).is_some_and(|item| {
                    matches!(
                        item.content,
                        shrimply_project_document::project::VideoItemContent::Obj(_)
                    )
                })
            {
                let selected_end = active
                    .iter()
                    .rposition(|(_, track_id, child, _, _)| {
                        *track_id == address.track_id() && child.id == child_id
                    })
                    .map_or(0, |index| index + 1);
                active.truncate(selected_end);
            } else {
                active.retain(|(_, track_id, child, _, _)| {
                    child.id == child_id && (!final_child || *track_id == address.track_id())
                });
            }
            if final_child {
                for (_, track_id, child, transition, _) in &mut active {
                    if *track_id == address.track_id() && child.id == address.item_id() {
                        *transition = None;
                    }
                }
            }
        }
        if let Some(shrimply_project_document::project::TrackAddress::Video {
            sequence_path,
            track_id,
        }) = self.capture_track.as_ref()
            && sequence_path.get(self.sequence_path.len()) == Some(&item.id)
        {
            let child_depth = self.sequence_path.len() + 1;
            if let Some(child_id) = sequence_path.get(child_depth) {
                active.retain(|(_, _, child, _, _)| child.id == *child_id);
            } else {
                active.retain(|(_, child_track_id, _, _, _)| child_track_id == track_id);
            }
        }
        self.sequence_stack.push(reference.sequence_id);
        self.sequence_path.push(item.id);
        self.scope_positions.push(position);
        let outer_position = self.position;
        let outer_clip_transition = self.clip_transition;
        self.position = position;
        let mut layers = Vec::with_capacity(active.len());
        let mut error = None;
        let children_measurement = shrimply_profiling::measure("Folded sequence / Render children");
        let mut morphed_items = HashSet::new();
        let morph_candidates = active
            .iter()
            .map(|(track_index, track_id, child, transition, previous)| {
                shrimply_visual_core::sequence::ActiveVideoItem {
                    track_index: *track_index,
                    track_id: *track_id,
                    item: child,
                    clip_transition: *transition,
                    previous: previous.as_ref(),
                }
            })
            .collect::<Vec<_>>();
        for active_index in 0..active.len() {
            let (track_index, track_id, child, transition, previous) = &active[active_index];
            abort_render_if_superseded!(self.decode_control, break);
            if morphed_items.contains(&child.id) {
                continue;
            }
            if let Some(endpoint) =
                shrimply_visual_core::sequence::morph_endpoint(&morph_candidates, active_index)
                && transition
                    .is_some_and(|transition| transition.role == ClipTransitionRole::Outgoing)
            {
                let transition = transition.expect("Morph endpoint must have a transition");
                let incoming = &active[endpoint.peer_index].2;
                morphed_items.insert(incoming.id);
                match self.render_morph_pair(
                    *track_index,
                    *track_id,
                    child,
                    incoming,
                    transition.definition.duration,
                    transition.progress,
                    &layers,
                ) {
                    Ok(morph_layers) => layers.extend(morph_layers),
                    Err(value) => {
                        error = Some(value);
                        break;
                    }
                }
                if self.superseded {
                    break;
                }
                continue;
            }
            let child = shrimply_visual_core::clip_transition::held_item(
                child,
                position,
                transition.is_some(),
            );
            let motion_blur_source = child.as_ref();
            let address = ItemAddress::Video {
                sequence_path: self.sequence_path.clone(),
                track_id: *track_id,
                item_id: child.id,
            };
            let cached_child = match shrimply_visual_core::modifier_cache::effective_item(
                &address,
                &child,
                self.project.canvas_size,
            ) {
                Ok(item) => item,
                Err(value) => {
                    error = Some(value);
                    break;
                }
            };
            let child = cached_child.as_ref().unwrap_or(&child);
            self.clip_transition = *transition;
            let previous = cached_child
                .is_none()
                .then_some(previous.as_ref())
                .flatten();
            let routes = self.decode_routes(*track_id, previous, child);
            let transmission_background = match self.render_scene_background(child, &layers) {
                Ok(background) => background,
                Err(value) => {
                    error = Some(value);
                    break;
                }
            };
            let cache_child = self.cache_item.as_ref().is_some_and(|address| {
                address.sequence_path() == self.sequence_path
                    && address.track_id() == *track_id
                    && address.item_id() == child.id
            });
            let cache_path_child = self.cache_item.as_ref().is_some_and(|address| {
                address
                    .sequence_path()
                    .get(self.sequence_path.len())
                    .copied()
                    .unwrap_or_else(|| address.item_id())
                    == child.id
            });
            match self.render_item(
                (*track_index, *track_id),
                child,
                cached_child.as_ref().map(|_| motion_blur_source),
                routes,
                cache_path_child,
                transmission_background.as_deref(),
            ) {
                Ok(Some(layer)) => {
                    if cache_child {
                        layers.clear();
                    }
                    layers.push(layer);
                    if let Some((color, opacity)) =
                        transition.and_then(shrimply_visual_core::clip_transition::color_layer)
                    {
                        match solid_video_layer(
                            self.compositor,
                            self.project.canvas_size,
                            color,
                            opacity,
                        ) {
                            Ok(layer) => layers.push(layer),
                            Err(value) => {
                                error = Some(value);
                                break;
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(value) => {
                    error = Some(value);
                    break;
                }
            }
            if self.superseded {
                break;
            }
        }
        drop(children_measurement);
        self.clip_transition = outer_clip_transition;
        self.position = outer_position;
        self.scope_positions.pop();
        self.sequence_path.pop();
        self.sequence_stack.pop();
        if let Some(error) = error {
            return Err(error);
        }
        if self.superseded {
            return Ok(VisualRender::Superseded);
        }
        if layers.is_empty() {
            return Ok(VisualRender::Empty);
        }
        let _measurement = shrimply_profiling::measure("Folded sequence / Flatten");
        let layer =
            self.compositor
                .render_layers_to_rgba(self.project.canvas_size, &layers, Some(0))?;
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), state),
        )))
    }

    fn render_item(
        &mut self,
        (track_index, track_id): (usize, Uuid),
        item: &VideoItem,
        motion_blur_source: Option<&VideoItem>,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<crate::layer::VideoLayer>, String> {
        let result = self.render_item_result(
            (track_index, track_id),
            item,
            motion_blur_source,
            routes,
            ignore_visibility,
            transmission_background,
        );
        if matches!(item.content, VideoItemContent::Manim(_)) {
            let element = self
                .sessions
                .elements
                .iter_mut()
                .find_map(|(key, element)| {
                    matches!(
                        key,
                        VisualElementKey::Manim {
                            sequence_path,
                            track_id: source_track_id,
                            item_id,
                            ..
                        } if sequence_path == &self.sequence_path
                            && *source_track_id == track_id
                            && *item_id == item.id
                    )
                    .then_some(element)
                });
            if let Some(element) = element {
                self.manim_updates.extend(element.take_manim_updates());
                if let Some(status) =
                    element.manim_status(result.as_ref().err().map(ToString::to_string))
                {
                    self.manim_updates.push(status);
                }
            } else if let (Err(error), VideoItemContent::Manim(manim)) = (&result, &item.content) {
                self.manim_updates.push(
                    shrimply_manim_wgpu::SourceIdentity {
                        item_id: item.id,
                        source_revision: item
                            .file
                            .snapshot()
                            .map_or(0, |snapshot| snapshot.revision()),
                        scene: manim.scene.clone(),
                        input_parameters: manim.parameters.clone(),
                    }
                    .error(Some(error.clone())),
                );
            }
        }
        result
    }

    fn render_item_result(
        &mut self,
        (track_index, track_id): (usize, Uuid),
        item: &VideoItem,
        motion_blur_source: Option<&VideoItem>,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<crate::layer::VideoLayer>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let Some((visual, render_canvas)) = self.render_item_visual(
            (track_index, track_id),
            item,
            motion_blur_source,
            routes,
            ignore_visibility,
            transmission_background,
        )?
        else {
            return Ok(None);
        };
        let _measurement = shrimply_profiling::measure("Video item / Build layer");
        visual
            .into_layer(
                self.compositor,
                render_canvas,
                (&self.sequence_path, track_id, item.id),
                &mut self.sessions.sources,
            )
            .map(Some)
    }

    fn render_item_visual(
        &mut self,
        (track_index, track_id): (usize, Uuid),
        item: &VideoItem,
        motion_blur_source: Option<&VideoItem>,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<(Visual, shrimply_project_document::project::CanvasSize)>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let address = ItemAddress::Video {
            sequence_path: self.sequence_path.clone(),
            track_id,
            item_id: item.id,
        };
        if self.render_stack.contains(&address) {
            return Err(format!("cyclic mask reference involving item {}", item.id));
        }
        self.render_stack.push(address);
        let result = self.render_item_inner(
            (track_index, track_id),
            item,
            motion_blur_source,
            routes,
            ignore_visibility,
            transmission_background,
        );
        self.render_stack.pop();
        result
    }

    fn render_item_inner(
        &mut self,
        (track_index, track_id): (usize, Uuid),
        item: &VideoItem,
        motion_blur_source: Option<&VideoItem>,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<(Visual, shrimply_project_document::project::CanvasSize)>, String> {
        let address = ItemAddress::Video {
            sequence_path: self.sequence_path.clone(),
            track_id,
            item_id: item.id,
        };
        let cached_item = shrimply_visual_core::modifier_cache::effective_item(
            &address,
            item,
            self.project.canvas_size,
        )?;
        let motion_blur_source = motion_blur_source
            .or_else(|| cached_item.as_ref().map(|_| item))
            .unwrap_or(item);
        let item = cached_item.as_ref().unwrap_or(item);
        use shrimply_visual_core::modifier_input::{CaptureBranch, capture_branch};
        let capture_address = ItemAddress::Video {
            sequence_path: self.sequence_path.clone(),
            track_id,
            item_id: item.id,
        };
        let branch = self
            .cache_item
            .as_ref()
            .map_or(CaptureBranch::Normal, |target| {
                capture_branch(target, &capture_address)
            });
        let cache_item = branch == CaptureBranch::Item;
        let cache_host = branch == CaptureBranch::Host;
        let cache_branch = branch != CaptureBranch::Normal;
        let content_position = if cache_item && self.snap_cache_item {
            crate::modifiers::transparent_fill::snapped_transparent_fill_position(
                self.project,
                item,
                self.position,
            )
        } else {
            crate::modifiers::transparent_fill::render_position(self.project, item, self.position)
        };
        let property_measurement = shrimply_profiling::measure("Video item / Resolve properties");
        let evaluation = VisualEvaluation::for_item_with_audio(
            self.project,
            item,
            self.position,
            &self.audio_analysis,
        );
        if !ignore_visibility
            && !resolve_bool(&item.visibility, &evaluation, &mut self.cache.expressions)
        {
            return Ok(None);
        }
        item.modifier_output_kind()
            .map_err(|error| format!("invalid modifier chain: {error}"))?;
        let transform = resolve_item_transform_with_audio(
            self.project,
            item,
            self.position,
            &self.audio_analysis,
            &mut self.cache.expressions,
        );
        let compositing = if cache_branch {
            ResolvedCompositing {
                opacity: 1.0,
                blend_mode: LayerBlendMode::Normal,
            }
        } else {
            ResolvedCompositing {
                opacity: resolve_scalar(
                    &item.compositing.opacity,
                    &evaluation,
                    &mut self.cache.expressions,
                )
                .clamp(0.0, 1.0),
                blend_mode: item
                    .compositing
                    .blend_mode
                    .value_at(evaluation.local_time()),
            }
        };
        let scene_3d = matches!(
            &item.content,
            shrimply_project_document::project::VideoItemContent::Obj(_)
                | shrimply_project_document::project::VideoItemContent::Gaussian(_)
        );
        let motion_blur_scene_3d = matches!(
            &motion_blur_source.content,
            shrimply_project_document::project::VideoItemContent::Obj(_)
                | shrimply_project_document::project::VideoItemContent::Gaussian(_)
        );
        let motion_blur_transform = if std::ptr::eq(motion_blur_source, item) {
            transform
        } else {
            resolve_item_transform_with_audio(
                self.project,
                motion_blur_source,
                self.position,
                &self.audio_analysis,
                &mut self.cache.expressions,
            )
        };
        let motion_blur_transforms = self.motion_blur_transforms(
            motion_blur_source,
            motion_blur_transform,
            motion_blur_scene_3d,
        );
        let render_canvas = if cache_host {
            self.project.canvas_size
        } else {
            shrimply_visual_core::generated::render_canvas(
                item,
                self.project.canvas_size,
                &evaluation,
                &mut self.cache.expressions,
            )
        };
        let sampling = resolve(
            &item.sample_method,
            &evaluation,
            &mut self.cache.expressions,
        );
        let sampling = shrimply_visual_core::generated::sampling(
            sampling,
            self.mode.accuracy().content_accurate(),
        );
        drop(property_measurement);

        let audio_analysis = self.audio_analysis.clone();
        let accuracy = self.mode.accuracy();
        let request = VisualRenderRequest {
            project: self.project,
            sequence_path: &self.sequence_path,
            item,
            position: content_position,
            audio_analysis: &audio_analysis,
            state: VisualState {
                transform: if scene_3d || cache_host {
                    shrimply_math_geometry::ComposedTransform2D::IDENTITY
                } else {
                    transform.composed()
                },
                bounds: Default::default(),
                sampling,
                skia_drawing_strategy: item.skia_drawing_strategy,
                compositing,
            },
            render_canvas,
            generated_transition: (!cache_branch)
                .then(|| generated_transition(item, self.position, scene_3d))
                .flatten(),
            accuracy,
            transmission_background,
            decode_control: self.decode_control,
        };
        let sequence_reference = match &item.content {
            shrimply_project_document::project::VideoItemContent::FoldedSequence(reference) => {
                Some(*reference)
            }
            _ => None,
        };
        let key = match &item.content {
            shrimply_project_document::project::VideoItemContent::Manim(_) => {
                VisualElementKey::Manim {
                    sequence_path: self.sequence_path.clone(),
                    track_id,
                    item_id: item.id,
                    width: render_canvas.width,
                    height: render_canvas.height,
                }
            }
            _ => VisualElementKey::Item {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: item.id,
                media_track_id: item.track_id,
                plane: VideoPlane::Color,
            },
        };
        if sequence_reference.is_none() {
            let _measurement = shrimply_profiling::measure("Video item / Create or reuse renderer");
            if self
                .sessions
                .elements
                .get(&key)
                .is_none_or(|element| !element.matches(item, render_canvas))
            {
                self.sessions.remove_manim_replacement(&key);
                let element = self.sessions.create_element(
                    &self.sequence_path,
                    track_id,
                    item,
                    render_canvas,
                    routes.route(VideoPlane::Color),
                )?;
                self.sessions.elements.insert(key.clone(), element);
            }
        }

        let rendered = if let Some(reference) = sequence_reference {
            self.render_folded_sequence(item, reference, request.state)?
        } else {
            let _measurement = shrimply_profiling::measure("Video item / Draw source");
            self.sessions.active_elements.insert(key.clone());
            let element = self
                .sessions
                .elements
                .get_mut(&key)
                .expect("visual element was just created");
            let rendered = element.draw(
                request,
                self.compositor,
                track_id,
                &mut self.sessions.sources,
            )?;
            if let Some(source) = element.sanim_source() {
                self.sanim_sources.insert(address.clone(), source);
            }
            rendered
        };
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let mut visual = match rendered {
            VisualRender::Ready(visual) => visual,
            VisualRender::Loading(_) => {
                if !matches!(
                    item.content,
                    shrimply_project_document::project::VideoItemContent::Manim(_)
                ) {
                    tracing::debug!(
                        item = %item.id,
                        content = ?item.content,
                        position = %self.position.as_label(),
                        ?accuracy,
                        "visual item is still loading",
                    );
                }
                self.loading = true;
                return Ok(None);
            }
            VisualRender::LoadingPlaceholder(visual) => {
                if !matches!(
                    item.content,
                    shrimply_project_document::project::VideoItemContent::Manim(_)
                ) {
                    tracing::debug!(
                        item = %item.id,
                        content = ?item.content,
                        position = %self.position.as_label(),
                        ?accuracy,
                        "visual item is showing a loading placeholder",
                    );
                }
                self.loading = true;
                self.loading_placeholder = true;
                visual
            }
            VisualRender::Empty => return Ok(None),
            VisualRender::Superseded => {
                self.superseded = true;
                return Ok(None);
            }
        };
        let modifier_measurement =
            shrimply_profiling::measure("Video item / Apply modifiers and masks");
        if !cache_host {
            let address = ItemAddress::Video {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: item.id,
            };
            if let Some(alpha_mask_video) = item.alpha_mask_video {
                let mask =
                    self.alpha_mask_source(track_index, track_id, item, alpha_mask_video, routes)?;
                if self.loading && !self.loading_placeholder {
                    return Ok(None);
                }
                visual = crate::alpha_mask::apply(visual, mask)?;
            }

            let modifier_plan = shrimply_visual_core::raster_modifiers::plan(item)?;
            for modifier_index in modifier_plan.source {
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                let modifier = &item.modifiers[modifier_index];
                let alpha_mask = modifier
                    .alpha_mask
                    .as_ref()
                    .filter(|mask| mask.enabled)
                    .map(|mask| {
                        resolve_shape_alpha_mask(mask, &evaluation, &mut self.cache.expressions)
                    });
                let mut context =
                    VisualModifierContext::new(item, &evaluation, &mut self.cache.expressions);
                context.accuracy = self.mode.accuracy();
                let masked = alpha_mask.is_some();
                if let Some(mask) = alpha_mask {
                    visual.begin_alpha_mask(mask);
                }
                visual = crate::modifiers::apply_source(&modifier.effect, visual, &mut context)?;
                if masked {
                    visual.end_alpha_mask();
                }
            }
            for modifier_index in modifier_plan.raster {
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                let modifier = shrimply_visual_core::raster_modifiers::modifier(
                    shrimply_visual_core::raster_modifiers::ModifierRequest {
                        project: self.project,
                        address: &address,
                        item,
                        position: self.position,
                        scope_positions: &self.scope_positions,
                        modifier_index,
                        require_complete_assets: matches!(
                            self.mode,
                            RenderMode::ExportContentAccurate { .. }
                        ),
                    },
                    &evaluation,
                    &mut self.cache.expressions,
                    self.mode.accuracy().content_accurate(),
                )?
                .ok_or("shared raster modifier did not resolve an operation")?;
                let mask_source = match &modifier.operation {
                    shrimply_visual_core::raster_modifiers::Operation::Mask(mask) => {
                        self.mask_source(mask.source.as_ref())?
                    }
                    _ => None,
                };
                if self.loading && !self.loading_placeholder {
                    return Ok(None);
                }
                let masked = modifier.alpha_mask.is_some();
                if let Some(mask) = modifier.alpha_mask {
                    visual.begin_alpha_mask(mask);
                }
                visual = crate::modifiers::apply_resolved(modifier.operation, visual, mask_source)?;
                if masked {
                    visual.end_alpha_mask();
                }
            }
        }
        if !cache_branch && let Some(samples) = motion_blur_transforms {
            visual.push_motion_blur(motion_blur_transform.composed(), samples);
        }
        if !scene_3d && !cache_branch {
            apply_visual_transition(&mut visual, item, self.position, transform.position);
        }
        if !cache_branch && let Some(transition) = self.clip_transition {
            apply_visual_clip_transition(&mut visual, transition, render_canvas);
        }
        if render_canvas != self.project.canvas_size {
            visual = visual.rasterize(item.skia_drawing_strategy, sampling);
            visual.push_transform(shrimply_math_geometry::ComposedTransform2D {
                matrix: glam::Mat3::from_scale(glam::Vec2::new(
                    self.project.canvas_size.width.max(1) as f32
                        / render_canvas.width.max(1) as f32,
                    self.project.canvas_size.height.max(1) as f32
                        / render_canvas.height.max(1) as f32,
                )),
            });
        }
        if !cache_branch
            && let Some(mask) = item
                .compositing
                .alpha_mask
                .as_ref()
                .filter(|mask| mask.enabled)
        {
            visual.push_alpha_mask(resolve_shape_alpha_mask(
                mask,
                &evaluation,
                &mut self.cache.expressions,
            ));
        }
        drop(modifier_measurement);
        Ok(Some((visual, render_canvas)))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_morph_pair(
        &mut self,
        track_index: usize,
        track_id: Uuid,
        outgoing: &VideoItem,
        incoming: &VideoItem,
        duration: Time,
        progress: f32,
        lower_layers: &[VideoLayer],
    ) -> Result<Vec<VideoLayer>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(Vec::new()));
        let content = serde_json::to_vec(&(outgoing, incoming))
            .map_err(|error| format!("serialize Morph transition endpoints: {error}"))?;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let key = MorphCacheKey {
            sequence_path: self.sequence_path.clone(),
            track_id,
            outgoing_id: outgoing.id,
            incoming_id: incoming.id,
            width: self.project.canvas_size.width,
            height: self.project.canvas_size.height,
            content_hash: hasher.finish(),
        };
        let cached = if let Some(cached) = self.cache.morphs.get(&key) {
            Rc::clone(cached)
        } else {
            let outer_position = self.position;
            let outer_scope_position = *self
                .scope_positions
                .last()
                .expect("frame renderer has an active sequence scope");
            let outer_transition = self.clip_transition;
            let volume_revision = self.sessions.volume_revision;
            let outer_audio = self.audio_analysis.clone();
            let created: Result<Option<CachedMorph>, String> = (|| {
                self.clip_transition = None;
                let (source_position, target_position) =
                    shrimply_math_media::clip_transition_bounds(outgoing.end, duration);
                self.position = source_position;
                *self
                    .scope_positions
                    .last_mut()
                    .expect("frame renderer has an active sequence scope") = source_position;
                self.audio_analysis = FrameAudioAnalysis {
                    volume: self.sessions.volume.sample(
                        self.project,
                        source_position,
                        volume_revision,
                    ),
                    mouth: self.sessions.mouth.sample(
                        self.project,
                        source_position,
                        volume_revision,
                    ),
                };
                let source_background = self.render_scene_background(outgoing, lower_layers)?;
                let source = self.render_item_visual(
                    (track_index, track_id),
                    outgoing,
                    None,
                    VideoDecodeRoutes::default(),
                    false,
                    source_background.as_deref(),
                )?;
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                self.position = target_position;
                *self
                    .scope_positions
                    .last_mut()
                    .expect("frame renderer has an active sequence scope") = target_position;
                self.audio_analysis = FrameAudioAnalysis {
                    volume: self.sessions.volume.sample(
                        self.project,
                        target_position,
                        volume_revision,
                    ),
                    mouth: self.sessions.mouth.sample(
                        self.project,
                        target_position,
                        volume_revision,
                    ),
                };
                let target_background = self.render_scene_background(incoming, lower_layers)?;
                let target = self.render_item_visual(
                    (track_index, track_id),
                    incoming,
                    None,
                    VideoDecodeRoutes::default(),
                    false,
                    target_background.as_deref(),
                )?;
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                let (Some((source, source_canvas)), Some((target, target_canvas))) =
                    (source, target)
                else {
                    return Ok(None);
                };
                if let (Some(source_vector), Some(target_vector)) =
                    (source.morph_input(), target.morph_input())
                {
                    return Ok(Some(CachedMorph::Vector {
                        morph: Rc::new(crate::vector_morph::PreparedVectorMorph::new(
                            source_vector.scene,
                            target_vector.scene,
                        )),
                        source_state: source_vector.state,
                        target_state: target_vector.state,
                    }));
                }
                let (source, source_compositing) =
                    self.materialize_morph_endpoint(source, source_canvas, track_id, outgoing.id)?;
                let (target, target_compositing) =
                    self.materialize_morph_endpoint(target, target_canvas, track_id, incoming.id)?;
                let flow = self.compositor.estimate_optical_flow(&source, &target)?;
                Ok(Some(CachedMorph::OpticalFlow {
                    source,
                    target,
                    flow,
                    source_compositing,
                    target_compositing,
                    source_strategy: outgoing.skia_drawing_strategy,
                    target_strategy: incoming.skia_drawing_strategy,
                }))
            })();
            self.position = outer_position;
            *self
                .scope_positions
                .last_mut()
                .expect("frame renderer has an active sequence scope") = outer_scope_position;
            self.clip_transition = outer_transition;
            self.audio_analysis = outer_audio;
            let Some(created) = created? else {
                return Ok(Vec::new());
            };
            let cached = Rc::new(created);
            self.cache.morphs.insert(key, Rc::clone(&cached));
            cached
        };
        self.render_cached_morph(&cached, progress)
    }

    fn materialize_morph_endpoint(
        &mut self,
        visual: Visual,
        render_canvas: shrimply_project_document::project::CanvasSize,
        track_id: Uuid,
        item_id: Uuid,
    ) -> Result<(Rc<crate::gpu::VisualFrame>, ResolvedCompositing), String> {
        let mut layer = visual.into_layer(
            self.compositor,
            render_canvas,
            (&self.sequence_path, track_id, item_id),
            &mut self.sessions.sources,
        )?;
        let compositing = match &mut layer {
            VideoLayer::Nv12 { compositing, .. } | VideoLayer::Rgba { compositing, .. } => {
                let original = *compositing;
                *compositing = ResolvedCompositing {
                    opacity: 1.0,
                    blend_mode: LayerBlendMode::Normal,
                };
                original
            }
        };
        self.compositor
            .render_layer_to_rgba(self.project.canvas_size, &layer)
            .map(|frame| (frame, compositing))
    }

    fn render_cached_morph(
        &mut self,
        cached: &CachedMorph,
        progress: f32,
    ) -> Result<Vec<VideoLayer>, String> {
        match cached {
            CachedMorph::Vector {
                morph,
                source_state,
                target_state,
            } => {
                let presentation = morph.presentation(
                    progress,
                    source_state.compositing.opacity,
                    target_state.compositing.opacity,
                );
                let frame = morph.frame(progress);
                let frame = self.compositor.render_vector_morph(
                    &frame,
                    presentation.scene,
                    if presentation.target_side {
                        target_state.skia_drawing_strategy
                    } else {
                        source_state.skia_drawing_strategy
                    },
                )?;
                let mut state = if presentation.target_side {
                    target_state.baked()
                } else {
                    source_state.baked()
                };
                state.compositing.opacity = presentation.opacity;
                Ok(vec![crate::layer::frame_layer(
                    GpuFrame::Rgba(frame),
                    state,
                )])
            }
            CachedMorph::OpticalFlow {
                source,
                target,
                flow,
                source_compositing,
                target_compositing,
                source_strategy,
                target_strategy,
            } => {
                let presentation = flow.presentation(progress);
                let source = Rc::new(self.compositor.render_mesh_flow(
                    source,
                    presentation.grid_size.x,
                    presentation.grid_size.y,
                    &presentation.source_offsets,
                )?);
                let target = Rc::new(self.compositor.render_mesh_flow(
                    target,
                    presentation.grid_size.x,
                    presentation.grid_size.y,
                    &presentation.target_offsets,
                )?);
                let state = |compositing, drawing_strategy| VisualState {
                    transform: shrimply_math_geometry::ComposedTransform2D::IDENTITY,
                    bounds: Default::default(),
                    sampling: VideoSampleMethod::Bilinear,
                    skia_drawing_strategy: drawing_strategy,
                    compositing,
                };
                let source_compositing = *source_compositing;
                let mut target_compositing = *target_compositing;
                target_compositing.opacity *= presentation.target_opacity;
                Ok(vec![
                    crate::layer::frame_layer(
                        GpuFrame::Rgba(source),
                        state(source_compositing, *source_strategy),
                    ),
                    crate::layer::frame_layer(
                        GpuFrame::Rgba(target),
                        state(target_compositing, *target_strategy),
                    ),
                ])
            }
        }
    }

    fn motion_blur_transforms(
        &mut self,
        item: &VideoItem,
        current: shrimply_project_document::project::ResolvedTransform,
        scene_3d: bool,
    ) -> Option<Vec<shrimply_math_geometry::ComposedTransform2D>> {
        if scene_3d || !item.motion_blur.enabled {
            return None;
        }
        let project = self.project;
        let volume_revision = self.sessions.volume_revision;
        shrimply_visual_core::motion_blur::sample_transforms(
            shrimply_visual_core::motion_blur::Request {
                project,
                item,
                position: self.position,
                current: current.composed(),
                content_accurate: self.mode.accuracy().content_accurate(),
            },
            &mut self.cache.expressions,
            |position| FrameAudioAnalysis {
                volume: self
                    .sessions
                    .volume
                    .sample(project, position, volume_revision),
                mouth: self
                    .sessions
                    .mouth
                    .sample(project, position, volume_revision),
            },
        )
    }

    fn alpha_mask_source(
        &mut self,
        _track_index: usize,
        track_id: Uuid,
        item: &VideoItem,
        media_track_id: u32,
        routes: VideoDecodeRoutes,
    ) -> Result<Option<Rc<crate::gpu::VisualFrame>>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let cache_key = (self.sequence_path.clone(), item.id, media_track_id);
        if let Some(layer) = self.alpha_mask_layers.get(&cache_key) {
            return Ok(Some(layer.clone()));
        }
        let (alpha_item, render_canvas) = shrimply_visual_core::alpha_mask::video_source(
            item,
            media_track_id,
            self.project.canvas_size,
        );
        let key = match &alpha_item.content {
            shrimply_project_document::project::VideoItemContent::Manim(_) => {
                VisualElementKey::Manim {
                    sequence_path: self.sequence_path.clone(),
                    track_id,
                    item_id: alpha_item.id,
                    width: render_canvas.width,
                    height: render_canvas.height,
                }
            }
            _ => VisualElementKey::Item {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: alpha_item.id,
                media_track_id: alpha_item.track_id,
                plane: VideoPlane::Alpha,
            },
        };
        if self
            .sessions
            .elements
            .get(&key)
            .is_none_or(|element| !element.matches(&alpha_item, render_canvas))
        {
            self.sessions.remove_manim_replacement(&key);
            let element = self.sessions.create_element(
                &self.sequence_path,
                track_id,
                &alpha_item,
                render_canvas,
                routes.route(VideoPlane::Alpha),
            )?;
            self.sessions.elements.insert(key.clone(), element);
        }
        let evaluation = VisualEvaluation::for_item_with_audio(
            self.project,
            &alpha_item,
            self.position,
            &self.audio_analysis,
        );
        let sampling = resolve(
            &alpha_item.sample_method,
            &evaluation,
            &mut self.cache.expressions,
        );
        let sampling = if self.mode.accuracy().content_accurate() {
            sampling
        } else {
            if matches!(sampling, VideoSampleMethod::Nearest) {
                VideoSampleMethod::Nearest
            } else {
                VideoSampleMethod::Bilinear
            }
        };
        let request = VisualRenderRequest {
            project: self.project,
            sequence_path: &self.sequence_path,
            item: &alpha_item,
            position: self.position,
            audio_analysis: &self.audio_analysis,
            state: VisualState {
                transform: shrimply_math_geometry::ComposedTransform2D::IDENTITY,
                bounds: Default::default(),
                sampling,
                skia_drawing_strategy: alpha_item.skia_drawing_strategy,
                compositing: ResolvedCompositing {
                    opacity: 1.0,
                    blend_mode: LayerBlendMode::Normal,
                },
            },
            render_canvas,
            generated_transition: None,
            accuracy: self.mode.accuracy(),
            transmission_background: None,
            decode_control: self.decode_control,
        };
        let rendered = self
            .sessions
            .elements
            .get_mut(&key)
            .expect("alpha mask element was just created")
            .draw(
                request,
                self.compositor,
                track_id,
                &mut self.sessions.sources,
            )?;
        let visual = match rendered {
            VisualRender::Ready(visual) => visual,
            VisualRender::Loading(_) => {
                tracing::debug!(
                    item = %alpha_item.id,
                    position = %self.position.as_label(),
                    ?request.accuracy,
                    "alpha-mask visual item is still loading",
                );
                self.loading = true;
                return Ok(None);
            }
            VisualRender::LoadingPlaceholder(visual) => {
                self.loading = true;
                self.loading_placeholder = true;
                visual
            }
            VisualRender::Empty => return Ok(None),
            VisualRender::Superseded => {
                self.superseded = true;
                return Ok(None);
            }
        };
        let layer = visual.into_layer(
            self.compositor,
            render_canvas,
            (&self.sequence_path, track_id, alpha_item.id),
            &mut self.sessions.sources,
        )?;
        let layer = self
            .compositor
            .render_layer_to_rgba(render_canvas, &layer)
            .map_err(|error| {
                format!(
                    "materialize alpha-mask plane {} for item {}: {error}",
                    alpha_item.track_id, item.id,
                )
            })?;
        self.alpha_mask_layers.insert(cache_key, layer.clone());
        Ok(Some(layer))
    }

    fn mask_source(
        &mut self,
        source: Option<&shrimply_visual_core::raster_modifiers::ExternalDependency>,
    ) -> Result<Option<Rc<crate::gpu::VisualFrame>>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let Some(source) = source else {
            return Ok(None);
        };
        if let Some((_, layer)) = self
            .mask_layers
            .iter()
            .find(|(dependency, _)| dependency == source)
        {
            return Ok(Some(layer.clone()));
        }
        let address = &source.address;
        if address.sequence_path() != self.sequence_path {
            return Err("mask source resolved outside the active sequence".to_string());
        }
        if source.scope_positions != self.scope_positions {
            return Err("mask source sequence timing diverged from its owner".to_string());
        }
        let tracks = self
            .project
            .video_tracks_for_path(address.sequence_path())
            .ok_or("mask source sequence no longer exists")?;
        let Some(active) = shrimply_visual_core::sequence::active_tracks(
            tracks,
            source.position(),
            Some(std::slice::from_ref(&address.item_id())),
        )
        .into_iter()
        .find(|active| {
            active.track_id == address.track_id() && active.item.id == address.item_id()
        }) else {
            return Ok(None);
        };
        let track_index = active.track_index;
        let track_id = active.track_id;
        let source_transition = active.clip_transition;
        let previous = active.previous.cloned();
        let item = shrimply_visual_core::clip_transition::held_item(
            active.item,
            source.position(),
            source_transition.is_some(),
        )
        .into_owned();
        let cached_item = shrimply_visual_core::modifier_cache::effective_item(
            address,
            &item,
            self.project.canvas_size,
        )?;
        let motion_blur_source = &item;
        let item = cached_item.as_ref().unwrap_or(&item);
        let previous = cached_item.is_none().then_some(previous.as_ref()).flatten();
        let routes = self.decode_routes(track_id, previous, item);
        let outer_transition = self.clip_transition;
        self.clip_transition = source_transition;
        let rendered = self.render_item(
            (track_index, track_id),
            item,
            cached_item.as_ref().map(|_| motion_blur_source),
            routes,
            true,
            None,
        );
        self.clip_transition = outer_transition;
        let Some(layer) = rendered? else {
            return Ok(None);
        };
        let layer = self
            .compositor
            .render_layer_to_rgba(self.project.canvas_size, &layer)?;
        self.mask_layers.push((source.clone(), layer.clone()));
        Ok(Some(layer))
    }
}

use shrimply_visual_core::generated::transition as generated_transition;

use shrimply_visual_core::transition::active_visual_transition;

fn apply_visual_clip_transition(
    visual: &mut Visual,
    transition: ActiveClipTransition,
    render_canvas: shrimply_project_document::project::CanvasSize,
) {
    let spatial = shrimply_visual_core::clip_transition::spatial(transition, render_canvas);
    if spatial.opacity != 1.0 {
        visual.multiply_opacity(spatial.opacity);
    }
    if spatial.transform != glam::Mat3::IDENTITY {
        visual.push_transform(shrimply_math_geometry::ComposedTransform2D {
            matrix: spatial.transform,
        });
    }
    if transition.role == ClipTransitionRole::Incoming {
        crate::visual_transition::apply_clip_mask(
            visual,
            &transition.definition,
            transition.progress,
        );
    }
}

fn apply_visual_transition(
    visual: &mut Visual,
    item: &VideoItem,
    position: Time,
    center: glam::Vec2,
) {
    let Some((_, transition, visible, _)) = active_visual_transition(item, position) else {
        return;
    };
    let spatial = shrimply_visual_core::transition::spatial(transition, visible, center);
    if spatial.opacity != 1.0 {
        visual.multiply_opacity(spatial.opacity);
    }
    if spatial.transform != glam::Mat3::IDENTITY {
        visual.push_transform(shrimply_math_geometry::ComposedTransform2D {
            matrix: spatial.transform,
        });
    }
    crate::visual_transition::apply(visual, transition, visible, center);
}
