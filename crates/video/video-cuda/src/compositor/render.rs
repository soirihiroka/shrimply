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
    cache_item: Option<&ItemAddress>,
    snap_cache_item: bool,
    excluded_item_id: Option<Uuid>,
    decode_control: Option<&DecodeControl>,
) -> RenderedFrame {
    let _measurement = shrimply_benchmarking::measure("Video / Render frame");
    compositor.set_render_control(None);
    if decode_control.is_some_and(DecodeControl::superseded) {
        return RenderedFrame {
            frame: None,
            audio_analysis: audio_analysis.clone(),
            loading: false,
            loading_placeholder: false,
            clear: false,
            errors: Vec::new(),
            manim_durations: Vec::new(),
            manim_parameters: Vec::new(),
            manim_statuses: Vec::new(),
            superseded: true,
        };
    }
    let mut errors = Vec::new();
    let mut manim_statuses = Vec::new();
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
                        shrimply_project::project::VideoItemContent::Obj(_)
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

    active_items.extend(shrimply_video_core::sequence::active_tracks(
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
            manim_durations: Vec::new(),
            manim_parameters: Vec::new(),
            manim_statuses,
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
        mask_layers: HashMap::new(),
        alpha_mask_layers: HashMap::new(),
        render_stack: Vec::new(),
        sequence_stack: Vec::new(),
        sequence_path: Vec::new(),
        manim_durations: Vec::new(),
        manim_parameters: Vec::new(),
        decode_control,
        superseded: false,
        clip_transition: None,
        cache_item: cache_item.cloned(),
        snap_cache_item,
        excluded_item_id,
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
        if let Some(transition) = active.clip_transition.filter(|transition| {
            transition.definition.kind == VisualClipTransitionKind::Morph
                && transition.role == ClipTransitionRole::Outgoing
        }) && let Some(incoming) = active_items[active_index + 1..].iter().find(|candidate| {
            candidate.track_id == active.track_id
                && candidate
                    .clip_transition
                    .is_some_and(|candidate_transition| {
                        candidate_transition.definition.kind == VisualClipTransitionKind::Morph
                            && candidate_transition.role == ClipTransitionRole::Incoming
                            && candidate_transition.progress == transition.progress
                    })
        }) {
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
            for item in [active.item, incoming.item] {
                if matches!(
                    item.content,
                    shrimply_project::project::VideoItemContent::Manim(_)
                ) {
                    manim_statuses.push((item.id, manim_source_revision(item), None));
                }
            }
            if renderer.loading || renderer.superseded {
                break;
            }
            continue;
        }
        let held_item = shrimply_video_core::clip_transition::held_item(
            active.item,
            position,
            active.clip_transition.is_some(),
        );
        let item = held_item.as_ref();
        let cached_item = match crate::modifier_cache::effective_item(item, project.canvas_size) {
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
            shrimply_project::project::VideoItemContent::Media => "Video item / Media",
            shrimply_project::project::VideoItemContent::Image => "Video item / Image",
            shrimply_project::project::VideoItemContent::Gif => "Video item / GIF",
            shrimply_project::project::VideoItemContent::Svg => "Video item / SVG",
            shrimply_project::project::VideoItemContent::Pdf(_) => "Video item / PDF",
            shrimply_project::project::VideoItemContent::Manim(_) => "Video item / Manim",
            shrimply_project::project::VideoItemContent::Blender(_) => "Video item / Blender",
            shrimply_project::project::VideoItemContent::LayeredImage(_) => {
                "Video item / Layered image"
            }
            shrimply_project::project::VideoItemContent::Text(_) => "Video item / Text",
            shrimply_project::project::VideoItemContent::Shape(_) => "Video item / Shape",
            shrimply_project::project::VideoItemContent::Paint(_) => "Video item / Paint",
            shrimply_project::project::VideoItemContent::Background(_) => "Video item / Background",
            shrimply_project::project::VideoItemContent::Obj(_) => "Video item / OBJ",
            shrimply_project::project::VideoItemContent::Gaussian(_) => "Video item / Gaussian",
            shrimply_project::project::VideoItemContent::FoldedSequence(_) => {
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
            let _measurement = shrimply_benchmarking::measure(item_measurement);
            renderer.render_item(
                active.track_index,
                active.track_id,
                item,
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
        let source_revision = match &item.content {
            shrimply_project::project::VideoItemContent::Manim(_) => {
                Some(manim_source_revision(item))
            }
            _ => None,
        };
        match rendered {
            Ok(Some(layer)) => {
                if let Some(source_revision) = source_revision {
                    manim_statuses.push((active.item.id, source_revision, None));
                }
                if item_ids.is_some_and(|ids| ids.contains(&active.item.id)) {
                    selected_layers.push(layer.clone());
                }
                layers.push(layer);
                if let Some((color, opacity)) = active
                    .clip_transition
                    .and_then(shrimply_video_core::clip_transition::color_layer)
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
            Ok(None) => {
                if let Some(source_revision) = source_revision {
                    manim_statuses.push((active.item.id, source_revision, None));
                }
            }
            Err(error) => {
                abort_render_if_superseded!(renderer.decode_control, {
                    renderer.superseded = true;
                    break;
                });
                let error = format!("Could not render visual item {}: {error}", item.id);
                if let Some(source_revision) = source_revision {
                    manim_statuses.push((active.item.id, source_revision, Some(error.clone())));
                }
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
            let _measurement = shrimply_benchmarking::measure("Video / Final composition");
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
            Ok(frame) => Some(frame),
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
        manim_durations: renderer.manim_durations,
        manim_parameters: renderer.manim_parameters,
        manim_statuses,
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
    mask_layers: HashMap<Uuid, Rc<crate::gpu::VisualFrame>>,
    alpha_mask_layers: HashMap<(Vec<Uuid>, Uuid, u32), Rc<crate::gpu::VisualFrame>>,
    render_stack: Vec<Uuid>,
    pub(super) sequence_stack: Vec<Uuid>,
    pub(super) sequence_path: Vec<Uuid>,
    manim_durations: Vec<(Uuid, u64, Time)>,
    manim_parameters: Vec<(
        Uuid,
        u64,
        String,
        Vec<shrimply_project::project::ManimParameter>,
        bool,
    )>,
    pub(super) decode_control: Option<&'a DecodeControl>,
    superseded: bool,
    pub(super) clip_transition: Option<ActiveClipTransition>,
    pub(super) cache_item: Option<ItemAddress>,
    pub(super) snap_cache_item: bool,
    excluded_item_id: Option<Uuid>,
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
            shrimply_project::project::VideoItemContent::Obj(_)
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
        let Some((sequence, position)) = shrimply_video_core::sequence::resolve(
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
                shrimply_benchmarking::measure("Folded sequence / Resolve active items");
            shrimply_video_core::sequence::active_tracks(&sequence.video_tracks, position, None)
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
                        shrimply_project::project::VideoItemContent::Obj(_)
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
        self.sequence_stack.push(reference.sequence_id);
        self.sequence_path.push(item.id);
        let outer_position = self.position;
        let outer_clip_transition = self.clip_transition;
        self.position = position;
        let mut layers = Vec::with_capacity(active.len());
        let mut error = None;
        let children_measurement =
            shrimply_benchmarking::measure("Folded sequence / Render children");
        let mut morphed_items = HashSet::new();
        for (active_index, (track_index, track_id, child, transition, previous)) in
            active.iter().enumerate()
        {
            abort_render_if_superseded!(self.decode_control, break);
            if morphed_items.contains(&child.id) {
                continue;
            }
            if let Some(transition) = transition.filter(|transition| {
                transition.definition.kind == VisualClipTransitionKind::Morph
                    && transition.role == ClipTransitionRole::Outgoing
            }) && let Some((_, _, incoming, _, _)) = active[active_index + 1..].iter().find(
                |(_, candidate_track_id, _, candidate_transition, _)| {
                    *candidate_track_id == *track_id
                        && candidate_transition.is_some_and(|candidate_transition| {
                            candidate_transition.definition.kind == VisualClipTransitionKind::Morph
                                && candidate_transition.role == ClipTransitionRole::Incoming
                                && candidate_transition.progress == transition.progress
                        })
                },
            ) {
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
            let child = shrimply_video_core::clip_transition::held_item(
                child,
                position,
                transition.is_some(),
            );
            let cached_child =
                match crate::modifier_cache::effective_item(&child, self.project.canvas_size) {
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
                *track_index,
                *track_id,
                child,
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
                        transition.and_then(shrimply_video_core::clip_transition::color_layer)
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
        let _measurement = shrimply_benchmarking::measure("Folded sequence / Flatten");
        let layer =
            self.compositor
                .render_layers_to_rgba(self.project.canvas_size, &layers, Some(0))?;
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), state),
        )))
    }

    fn render_item(
        &mut self,
        track_index: usize,
        track_id: Uuid,
        item: &VideoItem,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<crate::layer::VideoLayer>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let Some((visual, render_canvas)) = self.render_item_visual(
            track_index,
            track_id,
            item,
            routes,
            ignore_visibility,
            transmission_background,
        )?
        else {
            return Ok(None);
        };
        let _measurement = shrimply_benchmarking::measure("Video item / Build layer");
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
        track_index: usize,
        track_id: Uuid,
        item: &VideoItem,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<(Visual, shrimply_project::project::CanvasSize)>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        if self.render_stack.contains(&item.id) {
            return Err(format!("cyclic mask reference involving item {}", item.id));
        }
        self.render_stack.push(item.id);
        let result = self.render_item_inner(
            track_index,
            track_id,
            item,
            routes,
            ignore_visibility,
            transmission_background,
        );
        self.render_stack.pop();
        result
    }

    fn render_item_inner(
        &mut self,
        track_index: usize,
        track_id: Uuid,
        item: &VideoItem,
        routes: VideoDecodeRoutes,
        ignore_visibility: bool,
        transmission_background: Option<&crate::gpu::VisualFrame>,
    ) -> Result<Option<(Visual, shrimply_project::project::CanvasSize)>, String> {
        let cached_item = crate::modifier_cache::effective_item(item, self.project.canvas_size)?;
        let item = cached_item.as_ref().unwrap_or(item);
        let cache_item = self.cache_item.as_ref().is_some_and(|address| {
            address.sequence_path() == self.sequence_path
                && address.track_id() == track_id
                && address.item_id() == item.id
        });
        let cache_host = self.cache_item.as_ref().is_some_and(|address| {
            address
                .sequence_path()
                .get(self.sequence_path.len())
                .is_some_and(|host_id| *host_id == item.id)
        });
        let cache_branch = cache_item || cache_host;
        let content_position = if cache_item && self.snap_cache_item {
            crate::modifiers::transparent_fill::snapped_transparent_fill_position(
                self.project,
                item,
                self.position,
            )
        } else {
            crate::modifiers::transparent_fill::render_position(self.project, item, self.position)
        };
        let property_measurement =
            shrimply_benchmarking::measure("Video item / Resolve properties");
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
            shrimply_project::project::VideoItemContent::Obj(_)
                | shrimply_project::project::VideoItemContent::Gaussian(_)
        );
        let motion_blur_transforms = self.motion_blur_transforms(item, transform, scene_3d);
        let render_canvas = if cache_host {
            self.project.canvas_size
        } else {
            shrimply_video_core::generated::render_canvas(
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
        let sampling = shrimply_video_core::generated::sampling(
            sampling,
            self.mode.accuracy().content_accurate(),
        );
        drop(property_measurement);

        let audio_analysis = self.audio_analysis.clone();
        let request = VisualRenderRequest {
            project: self.project,
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
            accuracy: self.mode.accuracy(),
            transmission_background,
            decode_control: self.decode_control,
        };
        let sequence_reference = match &item.content {
            shrimply_project::project::VideoItemContent::FoldedSequence(reference) => {
                Some(*reference)
            }
            _ => None,
        };
        let key = match &item.content {
            shrimply_project::project::VideoItemContent::Manim(_) => VisualElementKey::Manim {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: item.id,
                width: render_canvas.width,
                height: render_canvas.height,
            },
            _ => VisualElementKey::Item {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: item.id,
                media_track_id: item.track_id,
                plane: VideoPlane::Color,
            },
        };
        if sequence_reference.is_none() {
            let _measurement =
                shrimply_benchmarking::measure("Video item / Create or reuse renderer");
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

        let (rendered, source_duration, parameters) = if let Some(reference) = sequence_reference {
            (
                self.render_folded_sequence(item, reference, request.state)?,
                None,
                None,
            )
        } else {
            let _measurement = shrimply_benchmarking::measure("Video item / Draw source");
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
            (
                rendered,
                element.take_source_duration(),
                element.take_manim_parameters(),
            )
        };
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        if let (Some(duration), shrimply_project::project::VideoItemContent::Manim(_)) =
            (source_duration, &item.content)
        {
            self.manim_durations
                .push((item.id, manim_source_revision(item), duration));
        }
        if let (
            Some((parameters, render_is_current)),
            shrimply_project::project::VideoItemContent::Manim(manim),
        ) = (parameters, &item.content)
        {
            self.manim_parameters.push((
                item.id,
                manim_source_revision(item),
                manim.scene.clone(),
                parameters,
                render_is_current,
            ));
        }
        let mut visual = match rendered {
            VisualRender::Ready(visual) => visual,
            VisualRender::Loading(_) => {
                if !matches!(
                    item.content,
                    shrimply_project::project::VideoItemContent::Manim(_)
                ) {
                    tracing::debug!(
                        item = %item.id,
                        content = ?item.content,
                        position = %self.position.as_label(),
                        ?request.accuracy,
                        "visual item is still loading",
                    );
                }
                self.loading = true;
                return Ok(None);
            }
            VisualRender::LoadingPlaceholder(visual) => {
                if !matches!(
                    item.content,
                    shrimply_project::project::VideoItemContent::Manim(_)
                ) {
                    tracing::debug!(
                        item = %item.id,
                        content = ?item.content,
                        position = %self.position.as_label(),
                        ?request.accuracy,
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
            shrimply_benchmarking::measure("Video item / Apply modifiers and masks");
        if !cache_host {
            if let Some(alpha_mask_video) = item.alpha_mask_video {
                let mask =
                    self.alpha_mask_source(track_index, track_id, item, alpha_mask_video, routes)?;
                if self.loading && !self.loading_placeholder {
                    return Ok(None);
                }
                visual = crate::alpha_mask::apply(visual, mask)?;
            }

            for (modifier_index, modifier) in item.modifiers.iter().enumerate() {
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                if !modifier.enabled {
                    continue;
                }
                let mask_source = match &modifier.effect {
                    shrimply_video_modifiers::ModifierEffect::Raster(effect) => match &**effect {
                        shrimply_video_modifiers::RasterModifierEffect::Mask(mask) => {
                            self.mask_source(mask.item_id)?
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if self.loading && !self.loading_placeholder {
                    return Ok(None);
                }
                let alpha_mask = modifier
                    .alpha_mask
                    .as_ref()
                    .filter(|mask| mask.enabled)
                    .map(|mask| {
                        resolve_shape_alpha_mask(mask, &evaluation, &mut self.cache.expressions)
                    });
                let analysis_cache_key = if matches!(
                    &modifier.effect,
                    ModifierEffect::Raster(effect)
                        if matches!(&**effect, RasterModifierEffect::TransparentFill(_))
                ) {
                    let address = ItemAddress::Video {
                        sequence_path: self.sequence_path.clone(),
                        track_id,
                        item_id: item.id,
                    };
                    let prompt_signature = match &modifier.effect {
                        ModifierEffect::Raster(effect) => match &**effect {
                            RasterModifierEffect::TransparentFill(fill) => fill.prompt_signature(),
                            _ => unreachable!("checked transparent fill modifier"),
                        },
                        _ => unreachable!("checked raster modifier"),
                    };
                    let key = (address.clone(), modifier.id, prompt_signature);
                    if let Some(cache_key) = self.cache.transparent_fill_keys.get(&key) {
                        Some(cache_key.clone())
                    } else {
                        let source_index = self
                            .project
                            .video_item(&address)
                            .and_then(|source| {
                                source
                                    .modifiers
                                    .iter()
                                    .position(|source| source.id == modifier.id)
                            })
                            .ok_or("transparent fill modifier source no longer exists")?;
                        let render_project =
                            crate::modifiers::transparent_fill::render_input_project(
                                self.project,
                                &address,
                                source_index,
                            )?;
                        let cache_key = crate::modifiers::transparent_fill::analysis_cache_key(
                            &render_project,
                            &address,
                            modifier.id,
                            prompt_signature,
                        );
                        self.cache
                            .transparent_fill_keys
                            .insert(key, cache_key.clone());
                        Some(cache_key)
                    }
                } else {
                    None
                };
                let mut context = VisualModifierContext::new(
                    self.project,
                    item,
                    content_position,
                    modifier.id,
                    modifier_index,
                    &evaluation,
                    &mut self.cache.expressions,
                );
                context.accuracy = self.mode.accuracy();
                context.require_complete_assets =
                    matches!(self.mode, RenderMode::ExportContentAccurate { .. });
                context.mask_source = mask_source;
                context.analysis_cache_key = analysis_cache_key;
                let masked = alpha_mask.is_some();
                if let Some(mask) = alpha_mask {
                    visual.begin_alpha_mask(mask);
                }
                visual = crate::modifiers::apply(&modifier.effect, visual, &mut context)?;
                if masked {
                    visual.end_alpha_mask();
                }
            }
        }
        if !cache_branch && let Some(samples) = motion_blur_transforms {
            visual.push_motion_blur(transform.composed(), samples);
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
            let outer_transition = self.clip_transition;
            let volume_revision = self.sessions.volume_revision;
            let outer_audio = self.audio_analysis.clone();
            let created: Result<Option<CachedMorph>, String> = (|| {
                self.clip_transition = None;
                let (source_position, target_position) =
                    shrimply_math_media::clip_transition_bounds(outgoing.end, duration);
                self.position = source_position;
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
                    track_index,
                    track_id,
                    outgoing,
                    VideoDecodeRoutes::default(),
                    false,
                    source_background.as_deref(),
                )?;
                abort_render_if_superseded!(self.decode_control, return Ok(None));
                self.position = target_position;
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
                    track_index,
                    track_id,
                    incoming,
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
        render_canvas: shrimply_project::project::CanvasSize,
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
                let forward = flow
                    .forward
                    .iter()
                    .map(|flow| [flow.x, flow.y])
                    .collect::<Vec<_>>();
                let backward = flow
                    .backward
                    .iter()
                    .map(|flow| [flow.x, flow.y])
                    .collect::<Vec<_>>();
                let source_offsets =
                    shrimply_math_geometry::optical_flow_source_offsets(&forward, progress);
                let target_offsets =
                    shrimply_math_geometry::optical_flow_source_offsets(&backward, 1.0 - progress);
                let grid_width = u32::try_from(flow.width)
                    .map_err(|_| "Morph optical-flow grid width is too large")?;
                let grid_height = u32::try_from(flow.height)
                    .map_err(|_| "Morph optical-flow grid height is too large")?;
                let source = Rc::new(self.compositor.render_mesh_flow(
                    source,
                    grid_width,
                    grid_height,
                    &source_offsets,
                )?);
                let target = Rc::new(self.compositor.render_mesh_flow(
                    target,
                    grid_width,
                    grid_height,
                    &target_offsets,
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
                target_compositing.opacity *= progress;
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
        current: shrimply_project::project::ResolvedTransform,
        scene_3d: bool,
    ) -> Option<Vec<shrimply_math_geometry::ComposedTransform2D>> {
        if scene_3d || !item.motion_blur.enabled {
            return None;
        }
        let project = self.project;
        let volume_revision = self.sessions.volume_revision;
        shrimply_video_core::motion_blur::sample_transforms(
            shrimply_video_core::motion_blur::Request {
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
        let mut alpha_item = item.clone();
        alpha_item.track_id = media_track_id;
        alpha_item.alpha_mask_video = None;
        alpha_item.stabilize_video = false;
        let render_canvas = shrimply_project::project::CanvasSize {
            width: if alpha_item.source_width > 0 {
                alpha_item.source_width
            } else {
                self.project.canvas_size.width.max(1)
            },
            height: if alpha_item.source_height > 0 {
                alpha_item.source_height
            } else {
                self.project.canvas_size.height.max(1)
            },
        };
        let key = match &alpha_item.content {
            shrimply_project::project::VideoItemContent::Manim(_) => VisualElementKey::Manim {
                sequence_path: self.sequence_path.clone(),
                track_id,
                item_id: alpha_item.id,
                width: render_canvas.width,
                height: render_canvas.height,
            },
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
        item_id: Option<Uuid>,
    ) -> Result<Option<Rc<crate::gpu::VisualFrame>>, String> {
        abort_render_if_superseded!(self.decode_control, return Ok(None));
        let Some(item_id) = item_id else {
            return Ok(None);
        };
        if let Some(layer) = self.mask_layers.get(&item_id) {
            return Ok(Some(layer.clone()));
        }
        let Some((track_index, track_id, item, previous)) = self
            .project
            .video_tracks
            .iter()
            .enumerate()
            .find_map(|(track_index, track)| {
                if !track.enabled {
                    return None;
                }
                track
                    .items
                    .iter()
                    .find(|item| {
                        item.id == item_id
                            && self.position >= item.start
                            && self.position < item.end
                    })
                    .map(|item| {
                        (
                            track_index,
                            track.id,
                            item.clone(),
                            preload::predecessor(&track.items, item.id).cloned(),
                        )
                    })
            })
        else {
            return Ok(None);
        };
        let cached_item = crate::modifier_cache::effective_item(&item, self.project.canvas_size)?;
        let item = cached_item.as_ref().unwrap_or(&item);
        let previous = cached_item.is_none().then_some(previous.as_ref()).flatten();
        let routes = self.decode_routes(track_id, previous, item);
        let Some(layer) = self.render_item(track_index, track_id, item, routes, true, None)? else {
            return Ok(None);
        };
        let layer = self
            .compositor
            .render_layer_to_rgba(self.project.canvas_size, &layer)?;
        self.mask_layers.insert(item_id, layer.clone());
        Ok(Some(layer))
    }
}

use shrimply_video_core::generated::transition as generated_transition;

use shrimply_video_core::transition::active_visual_transition;

fn apply_visual_clip_transition(
    visual: &mut Visual,
    transition: ActiveClipTransition,
    render_canvas: shrimply_project::project::CanvasSize,
) {
    let spatial = shrimply_video_core::clip_transition::spatial(transition, render_canvas);
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
    let spatial = shrimply_video_core::transition::spatial(transition, visible, center);
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
