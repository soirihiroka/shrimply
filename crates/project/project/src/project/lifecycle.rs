use super::*;

pub enum ProjectValidationOutcome {
    Valid(Project),
    FrameGridRepair(Project),
}

pub fn from_json_file(path: impl AsRef<Path>) -> Result<Project, String> {
    match from_json_file_with_frame_grid_repair(path)? {
        ProjectValidationOutcome::Valid(project) => Ok(project),
        ProjectValidationOutcome::FrameGridRepair(_) => {
            Err("project is not aligned to the project frame grid".to_string())
        }
    }
}

pub fn from_json_file_with_frame_grid_repair(
    path: impl AsRef<Path>,
) -> Result<ProjectValidationOutcome, String> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let project = conversion::from_json(&contents)
        .map_err(|error| format!("could not load {}: {error}", path.display()))?;
    let outcome = validate_with_frame_grid_repair(project)
        .map_err(|error| format!("could not load {}: {error}", path.display()))?;
    Ok(prepare_validation_outcome(
        outcome,
        path.parent().unwrap_or_else(|| Path::new(".")),
    ))
}

pub fn from_json_value(value: serde_json::Value) -> Result<Project, String> {
    match from_json_value_with_frame_grid_repair(value)? {
        ProjectValidationOutcome::Valid(project) => Ok(project),
        ProjectValidationOutcome::FrameGridRepair(_) => {
            Err("imported project is not aligned to the project frame grid".to_string())
        }
    }
}

pub fn from_json_value_with_frame_grid_repair(
    value: serde_json::Value,
) -> Result<ProjectValidationOutcome, String> {
    let project: Project = serde_json::from_value(value)
        .map_err(|error| format!("could not decode imported project: {error}"))?;
    let outcome = validate_with_frame_grid_repair(project)
        .map_err(|error| format!("could not validate imported project: {error}"))?;
    Ok(prepare_imported_outcome(outcome))
}

pub(super) fn validate_with_frame_grid_repair(
    project: Project,
) -> Result<ProjectValidationOutcome, String> {
    project.validate_without_frame_alignment()?;
    if validate_project_frame_alignment(&project).is_ok() {
        return Ok(ProjectValidationOutcome::Valid(project));
    }
    let mut repaired = project;
    if !repaired.repair_frame_grid() {
        return Err("project is not aligned to the project frame grid".to_string());
    }
    repaired.validate()?;
    Ok(ProjectValidationOutcome::FrameGridRepair(repaired))
}

pub(super) fn prepare_validation_outcome(
    outcome: ProjectValidationOutcome,
    directory: &Path,
) -> ProjectValidationOutcome {
    match outcome {
        ProjectValidationOutcome::Valid(mut project) => {
            prepare_loaded_project(&mut project, directory);
            ProjectValidationOutcome::Valid(project)
        }
        ProjectValidationOutcome::FrameGridRepair(mut project) => {
            prepare_loaded_project(&mut project, directory);
            ProjectValidationOutcome::FrameGridRepair(project)
        }
    }
}

fn prepare_loaded_project(project: &mut Project, directory: &Path) {
    project.migrate_gaussian_items();
    project.resolve_media_paths(directory);
    project.ensure_ids();
    project.normalize_media_relative_transforms();
}

fn prepare_imported_outcome(outcome: ProjectValidationOutcome) -> ProjectValidationOutcome {
    let prepare = |project: &mut Project| {
        project.ensure_ids();
        project.normalize_media_relative_transforms();
    };
    match outcome {
        ProjectValidationOutcome::Valid(mut project) => {
            prepare(&mut project);
            ProjectValidationOutcome::Valid(project)
        }
        ProjectValidationOutcome::FrameGridRepair(mut project) => {
            prepare(&mut project);
            ProjectValidationOutcome::FrameGridRepair(project)
        }
    }
}

impl Project {
    pub fn frame_step(&self) -> Time {
        shrimply_math_core::time_from_frame(1, self.fps)
            .expect("project frame rate must be positive")
    }

    pub fn repair_frame_grid(&mut self) -> bool {
        let frame_step = self.frame_step();
        let mut changed = false;
        for track in &mut self.caption_tracks {
            let mut previous_end = None;
            for item in &mut track.items {
                changed |= repair_frame_aligned_times(
                    &mut item.start,
                    &mut item.end,
                    &mut previous_end,
                    self.fps,
                    frame_step,
                );
            }
        }
        for track in self.video_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.video_tracks),
        ) {
            let mut previous_end = None;
            for item in &mut track.items {
                changed |= repair_frame_aligned_times(
                    &mut item.start,
                    &mut item.end,
                    &mut previous_end,
                    self.fps,
                    frame_step,
                );
            }
        }
        for track in self.audio_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.audio_tracks),
        ) {
            let mut previous_end = None;
            for item in &mut track.items {
                changed |= repair_frame_aligned_times(
                    &mut item.start,
                    &mut item.end,
                    &mut previous_end,
                    self.fps,
                    frame_step,
                );
            }
        }
        changed
    }

    pub fn has_timeline_items(&self) -> bool {
        self.caption_tracks
            .iter()
            .any(|track| !track.items.is_empty())
            || self
                .video_tracks
                .iter()
                .any(|track| !track.items.is_empty())
            || self
                .audio_tracks
                .iter()
                .any(|track| !track.items.is_empty())
            || self.folded_sequences.iter().any(|sequence| {
                sequence
                    .video_tracks
                    .iter()
                    .any(|track| !track.items.is_empty())
                    || sequence
                        .audio_tracks
                        .iter()
                        .any(|track| !track.items.is_empty())
            })
    }

    pub fn normalize_clip_transitions(&mut self) {
        for track in &mut self.video_tracks {
            normalize_visual_clip_transitions(&mut track.items);
        }
        for track in &mut self.audio_tracks {
            normalize_audio_clip_transitions(&mut track.items);
        }
        for sequence in &mut self.folded_sequences {
            for track in &mut sequence.video_tracks {
                normalize_visual_clip_transitions(&mut track.items);
            }
            for track in &mut sequence.audio_tracks {
                normalize_audio_clip_transitions(&mut track.items);
            }
        }
    }

    pub fn folded_sequence(&self, id: Uuid) -> Option<&FoldedSequence> {
        self.folded_sequences
            .iter()
            .find(|sequence| sequence.id == id)
    }

    pub fn folded_sequence_mut(&mut self, id: Uuid) -> Option<&mut FoldedSequence> {
        self.folded_sequences
            .iter_mut()
            .find(|sequence| sequence.id == id)
    }

    pub(super) fn can_insert_sequence_reference(
        &self,
        sequence_id: Uuid,
        parent_sequence_id: Option<Uuid>,
    ) -> bool {
        let Some(parent_sequence_id) = parent_sequence_id else {
            return true;
        };
        let mut pending = vec![sequence_id];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if id == parent_sequence_id {
                return false;
            }
            if !visited.insert(id) {
                continue;
            }
            let Some(sequence) = self.folded_sequence(id) else {
                return false;
            };
            pending.extend(
                sequence_references(&sequence.video_tracks, &sequence.audio_tracks)
                    .map(|reference| reference.sequence_id),
            );
        }
        true
    }

    pub fn prune_folded_sequences(&mut self) {
        let mut pending =
            sequence_reference_items(&self.video_tracks, &self.audio_tracks).collect::<Vec<_>>();
        let mut reachable_sequences = HashSet::new();
        let mut reachable_items = HashSet::new();
        while let Some((item_id, reference)) = pending.pop() {
            reachable_items.insert(item_id);
            if !reachable_sequences.insert(reference.sequence_id) {
                continue;
            }
            if let Some(sequence) = self.folded_sequence(reference.sequence_id) {
                pending.extend(sequence_reference_items(
                    &sequence.video_tracks,
                    &sequence.audio_tracks,
                ));
            }
        }
        self.folded_sequences
            .retain(|sequence| reachable_sequences.contains(&sequence.id));
        self.expanded_sequence_paths.retain(|path| {
            !path.is_empty() && path.iter().all(|item| reachable_items.contains(item))
        });
    }

    pub(super) fn migrate_gaussian_items(&mut self) {
        for item in self
            .video_tracks
            .iter_mut()
            .chain(
                self.folded_sequences
                    .iter_mut()
                    .flat_map(|sequence| &mut sequence.video_tracks),
            )
            .flat_map(|track| track.items.iter_mut())
        {
            let ply = item
                .file
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ply"));
            if !ply || !matches!(item.content, VideoItemContent::Obj(_)) {
                continue;
            }
            let VideoItemContent::Obj(scene) = std::mem::take(&mut item.content) else {
                unreachable!();
            };
            item.content = VideoItemContent::Gaussian(Box::new(shrimply_3dgs::GaussianScene {
                model: scene.model,
                camera: shrimply_3dgs::Camera3d {
                    source: scene.camera.source,
                    projection: match scene.camera.projection {
                        shrimply_scene_3d::CameraProjection::Perspective => {
                            shrimply_3dgs::CameraProjection::Perspective
                        }
                        shrimply_scene_3d::CameraProjection::Orthographic => {
                            shrimply_3dgs::CameraProjection::Orthographic
                        }
                        shrimply_scene_3d::CameraProjection::Equirectangular => {
                            shrimply_3dgs::CameraProjection::Equirectangular
                        }
                        shrimply_scene_3d::CameraProjection::Cylindrical => {
                            shrimply_3dgs::CameraProjection::Cylindrical
                        }
                        shrimply_scene_3d::CameraProjection::Fisheye => {
                            shrimply_3dgs::CameraProjection::Fisheye
                        }
                    },
                    position: scene.camera.position,
                    rotation_degrees: scene.camera.rotation_degrees,
                    vertical_fov_degrees: scene.camera.vertical_fov_degrees,
                    orthographic_height: scene.camera.orthographic_height,
                    focus_distance: scene.camera.focus_distance,
                    f_stop: scene.camera.f_stop,
                    exposure_ev: scene.camera.exposure_ev,
                },
            }));
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_without_frame_alignment()?;
        validate_project_frame_alignment(self)
    }

    fn validate_without_frame_alignment(&self) -> Result<(), String> {
        if self.format_version != PROJECT_FORMAT_VERSION {
            return Err(format!(
                "unsupported project format {}; expected {}",
                self.format_version, PROJECT_FORMAT_VERSION,
            ));
        }
        for (track_index, track) in self.video_tracks.iter().enumerate() {
            for (item_index, item) in track.items.iter().enumerate() {
                item.modifier_output_kind().map_err(|error| {
                    format!(
                        "invalid modifier chain at visual track {}, item {}: {error}",
                        track_index + 1,
                        item_index + 1,
                    )
                })?;
                validate_visual_alpha_masks(item).map_err(|error| {
                    format!(
                        "invalid alpha mask at visual track {}, item {}: {error}",
                        track_index + 1,
                        item_index + 1,
                    )
                })?;
                validate_visual_transitions(item).map_err(|error| {
                    format!(
                        "invalid transition at visual track {}, item {}: {error}",
                        track_index + 1,
                        item_index + 1,
                    )
                })?;
                paint::validate_item(item).map_err(|error| {
                    format!(
                        "invalid paint at visual track {}, item {}: {error}",
                        track_index + 1,
                        item_index + 1,
                    )
                })?;
            }
        }
        for (track_index, track) in self.audio_tracks.iter().enumerate() {
            for (item_index, item) in track.items.iter().enumerate() {
                validate_transition_durations(
                    item.end.saturating_sub(item.start),
                    item.transitions.intro.as_ref().map(|value| value.duration),
                    item.transitions.outro.as_ref().map(|value| value.duration),
                )
                .map_err(|error| {
                    format!(
                        "invalid transition at audio track {}, item {}: {error}",
                        track_index + 1,
                        item_index + 1,
                    )
                })?;
            }
        }
        for (track_index, track) in self.caption_tracks.iter().enumerate() {
            validate_track_times(
                "caption",
                track_index,
                track.items.iter().map(|item| (item.start, item.end)),
            )?;
        }
        for (track_index, track) in self.video_tracks.iter().enumerate() {
            validate_track_times(
                "visual",
                track_index,
                track.items.iter().map(|item| (item.start, item.end)),
            )?;
            validate_visual_clip_transitions(track_index, &track.items)?;
        }
        for (track_index, track) in self.audio_tracks.iter().enumerate() {
            validate_track_times(
                "audio",
                track_index,
                track.items.iter().map(|item| (item.start, item.end)),
            )?;
            validate_audio_clip_transitions(track_index, &track.items)?;
        }
        let mut sequence_ids = HashSet::new();
        for sequence in &self.folded_sequences {
            if !sequence_ids.insert(sequence.id) {
                return Err(format!("duplicate folded sequence id {}", sequence.id));
            }
            validate_folded_sequence(sequence)?;
        }
        validate_sequence_references(self)?;
        Ok(())
    }

    pub fn resolve_media_paths(&mut self, directory: &Path) {
        self.visit_assets_mut(|asset| {
            if !asset.as_os_str().is_empty() {
                *asset = asset.resolve_against(directory);
            }
        });
    }

    #[cfg(feature = "editor")]
    pub(super) fn make_asset_paths_portable(&mut self, directory: &Path) {
        self.visit_assets_mut(|asset| {
            if asset.path().is_relative() || asset.as_os_str().is_empty() {
                return;
            }
            if let Ok(relative) = asset.path().strip_prefix(directory)
                && !relative.as_os_str().is_empty()
            {
                *asset = Asset::new(relative);
            }
        });
    }

    pub fn ensure_ids(&mut self) {
        let mut seen_track_ids = HashSet::new();
        for track in &mut self.caption_tracks {
            ensure_unique_id(&mut track.id, &mut seen_track_ids);
        }
        for track in &mut self.video_tracks {
            ensure_unique_id(&mut track.id, &mut seen_track_ids);
        }
        for track in &mut self.audio_tracks {
            ensure_unique_id(&mut track.id, &mut seen_track_ids);
        }
        for sequence in &mut self.folded_sequences {
            for track in &mut sequence.video_tracks {
                ensure_unique_id(&mut track.id, &mut seen_track_ids);
            }
            for track in &mut sequence.audio_tracks {
                ensure_unique_id(&mut track.id, &mut seen_track_ids);
            }
        }

        let mut seen_item_ids = HashSet::new();
        let mut seen_inspectable_ids = HashSet::new();
        for track in &mut self.caption_tracks {
            for item in &mut track.items {
                ensure_unique_id(&mut item.id, &mut seen_item_ids);
            }
        }
        for track in self.video_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.video_tracks),
        ) {
            for item in &mut track.items {
                ensure_unique_id(&mut item.id, &mut seen_item_ids);
                ensure_video_property_ids(item, &mut seen_inspectable_ids);
            }
        }
        for track in self.audio_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.audio_tracks),
        ) {
            for item in &mut track.items {
                ensure_unique_id(&mut item.id, &mut seen_item_ids);
                ensure_audio_property_ids(item, &mut seen_inspectable_ids);
            }
        }
    }

    pub fn regenerate_video_property_ids(item: &mut VideoItem) {
        let mut seen = HashSet::new();
        ensure_video_property_ids(&mut item.clone(), &mut seen);
        ensure_video_property_ids(item, &mut seen);
    }

    pub fn regenerate_audio_property_ids(item: &mut AudioItem) {
        let mut seen = HashSet::new();
        ensure_audio_property_ids(&mut item.clone(), &mut seen);
        ensure_audio_property_ids(item, &mut seen);
    }

    pub fn normalize_media_relative_transforms(&mut self) {
        let canvas = Vec2::new(
            self.canvas_size.width.max(1) as f32,
            self.canvas_size.height.max(1) as f32,
        );
        for track in self.video_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.video_tracks),
        ) {
            for item in &mut track.items {
                if !item.is_media() {
                    continue;
                }
                let Some(media) = video_item_media_size(item, canvas) else {
                    continue;
                };
                if item.source_width == 0 || item.source_height == 0 {
                    let size = media_size_components(media);
                    item.source_width = size.x;
                    item.source_height = size.y;
                }

                if let Some(mode) = media_transform_normalization(&item.transform, canvas, media) {
                    normalize_transform_to_media(&mut item.transform, mode, canvas, media);
                }
                if let Some(transform) = &mut item.default_transform
                    && let Some(mode) = media_transform_normalization(transform, canvas, media)
                {
                    normalize_transform_to_media(transform, mode, canvas, media);
                }
            }
        }
    }

    pub fn duration(&self) -> Time {
        self.video_tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.end))
            .chain(
                self.audio_tracks
                    .iter()
                    .flat_map(|track| track.items.iter().map(|item| item.end)),
            )
            .chain(
                self.caption_tracks
                    .iter()
                    .flat_map(|track| track.items.iter().map(|item| item.end)),
            )
            .max()
            .unwrap_or(Time::ZERO)
    }

    pub fn first_video_item(&self) -> Option<&VideoItem> {
        self.video_tracks
            .iter()
            .find_map(|track| track.items.iter().find(|item| item.is_media()))
    }

    pub fn video_item_by_id(&self, id: Uuid) -> Option<&VideoItem> {
        self.video_tracks
            .iter()
            .chain(
                self.folded_sequences
                    .iter()
                    .flat_map(|sequence| &sequence.video_tracks),
            )
            .flat_map(|track| &track.items)
            .find(|item| item.id == id)
    }

    pub fn video_item_by_id_mut(&mut self, id: Uuid) -> Option<&mut VideoItem> {
        self.video_tracks
            .iter_mut()
            .chain(
                self.folded_sequences
                    .iter_mut()
                    .flat_map(|sequence| &mut sequence.video_tracks),
            )
            .flat_map(|track| &mut track.items)
            .find(|item| item.id == id)
    }

    pub fn first_audio_item(&self) -> Option<&AudioItem> {
        self.audio_tracks.iter().find_map(|track| {
            track.items.iter().find(|item| {
                matches!(&item.source, AudioSource::Media | AudioSource::Tts(_))
                    && !item.file.as_os_str().is_empty()
            })
        })
    }

    pub fn source_file(&self) -> Option<&Path> {
        self.first_video_item()
            .map(|item| item.file.path())
            .or_else(|| self.first_audio_item().map(|item| item.file.path()))
    }

    pub fn source_asset(&self) -> Option<&Asset> {
        self.first_video_item()
            .map(|item| &item.file)
            .or_else(|| self.first_audio_item().map(|item| &item.file))
    }

    pub fn assets(&self) -> Vec<&Asset> {
        let mut assets = Vec::new();
        self.visit_assets(|asset| {
            if !asset.as_os_str().is_empty() {
                assets.push(asset);
            }
        });
        assets
    }

    pub fn watch_assets(&self) -> Result<(), String> {
        for asset in self.assets() {
            asset.watch()?;
        }
        Ok(())
    }

    pub fn uses_video_asset(&self, path: &Path) -> bool {
        let mut used = false;
        self.visit_video_assets(|asset| used |= asset.path() == path);
        used
    }

    pub fn uses_audio_asset(&self, path: &Path) -> bool {
        let mut used = false;
        self.visit_audio_assets(|asset| used |= asset.path() == path);
        used
    }

    fn visit_assets<'a>(&'a self, mut visit: impl FnMut(&'a Asset)) {
        self.visit_video_assets(&mut visit);
        self.visit_audio_assets(visit);
    }

    fn visit_video_assets<'a>(&'a self, mut visit: impl FnMut(&'a Asset)) {
        for track in self.video_tracks.iter().chain(
            self.folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.video_tracks),
        ) {
            for item in &track.items {
                if item.uses_file_asset() {
                    visit(&item.file);
                }
                if let Some(settings) = &item.video_generation {
                    for value in settings.inputs.values() {
                        if let shrimply_video_generation::VideoGenerationValue::Media { items } =
                            value
                        {
                            for media in items {
                                visit(&media.value);
                            }
                        }
                    }
                }
                if let VideoItemContent::Paint(paint) = &item.content {
                    for texture in paint
                        .palette
                        .iter()
                        .filter_map(|entry| entry.texture.as_ref())
                    {
                        visit(&texture.image_path);
                    }
                }
                if let VideoItemContent::Obj(scene) = &item.content
                    && let Some(environment) = &scene.environment.file
                {
                    visit(environment);
                }
                for modifier in &item.modifiers {
                    let ModifierEffect::Scene3d(effect) = &modifier.effect else {
                        continue;
                    };
                    let shrimply_video_modifiers::scene_3d::Scene3dModifierEffect::Object(object) =
                        &**effect
                    else {
                        continue;
                    };
                    if let Some(file) = &object.file {
                        visit(file);
                    }
                }
            }
        }
    }

    fn visit_audio_assets<'a>(&'a self, mut visit: impl FnMut(&'a Asset)) {
        for track in self.audio_tracks.iter().chain(
            self.folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.audio_tracks),
        ) {
            for item in &track.items {
                if item.uses_file_asset() {
                    visit(&item.file);
                }
                if let AudioSource::Tts(settings) = &item.source {
                    for value in settings.inputs.values() {
                        if let shrimply_tts::TtsValue::Audio { value } = value {
                            visit(value);
                        }
                    }
                }
            }
        }
    }

    fn visit_assets_mut(&mut self, mut visit: impl FnMut(&mut Asset)) {
        for track in self.video_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.video_tracks),
        ) {
            for item in &mut track.items {
                if item.uses_file_asset() {
                    visit(&mut item.file);
                }
                if let Some(settings) = &mut item.video_generation {
                    for value in settings.inputs.values_mut() {
                        if let shrimply_video_generation::VideoGenerationValue::Media { items } =
                            value
                        {
                            for media in items {
                                visit(&mut media.value);
                            }
                        }
                    }
                }
                if let VideoItemContent::Paint(paint) = &mut item.content {
                    for texture in paint
                        .palette
                        .iter_mut()
                        .filter_map(|entry| entry.texture.as_mut())
                    {
                        visit(&mut texture.image_path);
                    }
                }
                if let VideoItemContent::Obj(scene) = &mut item.content
                    && let Some(environment) = &mut scene.environment.file
                {
                    visit(environment);
                }
                for modifier in &mut item.modifiers {
                    let ModifierEffect::Scene3d(effect) = &mut modifier.effect else {
                        continue;
                    };
                    let shrimply_video_modifiers::scene_3d::Scene3dModifierEffect::Object(object) =
                        &mut **effect
                    else {
                        continue;
                    };
                    if let Some(file) = &mut object.file {
                        visit(file);
                    }
                }
            }
        }
        for track in self.audio_tracks.iter_mut().chain(
            self.folded_sequences
                .iter_mut()
                .flat_map(|sequence| &mut sequence.audio_tracks),
        ) {
            for item in &mut track.items {
                if item.uses_file_asset() {
                    visit(&mut item.file);
                }
                if let AudioSource::Tts(settings) = &mut item.source {
                    for value in settings.inputs.values_mut() {
                        if let shrimply_tts::TtsValue::Audio { value } = value {
                            visit(value);
                        }
                    }
                }
            }
        }
    }
}

pub fn validate_visual_transitions(item: &VisualItem) -> Result<(), String> {
    validate_transition_durations(
        item.end.saturating_sub(item.start),
        item.transitions.intro.as_ref().map(|value| value.duration),
        item.transitions.outro.as_ref().map(|value| value.duration),
    )?;
    if matches!(
        item.content,
        VisualSource::Obj(_) | VisualSource::Gaussian(_)
    ) && (item.transitions.intro.is_some() || item.transitions.outro.is_some())
    {
        return Err("2D visual transitions are not supported for 3D scene items".into());
    }
    for (side, transition) in [
        (TransitionSide::Intro, item.transitions.intro.as_ref()),
        (TransitionSide::Outro, item.transitions.outro.as_ref()),
    ]
    .into_iter()
    .filter_map(|(side, transition)| transition.map(|transition| (side, transition)))
    {
        if !transition.slide_rotation_degrees.is_finite()
            || !transition.slide_distance.is_finite()
            || transition.slide_distance < 0.0
        {
            return Err("slide parameters must be finite and distance must be nonnegative".into());
        }
        if !transition.effect_amount.is_finite()
            || !transition.effect_detail.is_finite()
            || !transition.effect_angle_degrees.is_finite()
            || !transition.effect_softness.is_finite()
            || transition.effect_amount < 0.0
            || transition.effect_detail < 0.0
            || transition.effect_softness < 0.0
        {
            return Err("transition effect parameters must be finite and nonnegative".into());
        }
        if !transition.iris_center.is_finite()
            || !(0.0..=1.0).contains(&transition.iris_center.x)
            || !(0.0..=1.0).contains(&transition.iris_center.y)
        {
            return Err("iris center must be between 0 and 1".into());
        }
        if !transition.drawing_stroke_overlap.is_finite()
            || !(-1.0..=1.0).contains(&transition.drawing_stroke_overlap)
            || !transition.drawing_stroke_length_weight.is_finite()
            || !(0.0..=1.0).contains(&transition.drawing_stroke_length_weight)
        {
            return Err(
                "drawing transition overlap must be between -1 and 1 and length weight must be between 0 and 1"
                    .into(),
            );
        }
        if transition.effect_evolve_seed && transition.effect_seed_frequency == 0 {
            return Err(
                "transition seed frequency must be positive when evolution is enabled".into(),
            );
        }
        if matches!(
            (side, transition.kind),
            (TransitionSide::Intro, VisualTransitionKind::Diffusion)
                | (
                    TransitionSide::Outro,
                    VisualTransitionKind::ReverseDiffusion
                )
        ) {
            return Err("Diffusion is only supported for outros and Reverse Diffusion is only supported for intros".into());
        }
        if matches!(transition.kind, VisualTransitionKind::Morph)
            && !matches!(item.content, VisualSource::Text(_))
        {
            return Err("Morph transitions are only supported for text".into());
        }
        if transition.kind == VisualTransitionKind::Drawing
            && !matches!(item.content, VisualSource::Paint(_))
        {
            return Err("Drawing transitions are only supported for paint items".into());
        }
        if matches!(
            transition.kind,
            VisualTransitionKind::Write
                | VisualTransitionKind::Create
                | VisualTransitionKind::FacetAssembly
                | VisualTransitionKind::Coalesce
                | VisualTransitionKind::ContourCurrent
                | VisualTransitionKind::SoftRefraction
                | VisualTransitionKind::MorphologicalResolve
                | VisualTransitionKind::LivingFill
                | VisualTransitionKind::Diffusion
                | VisualTransitionKind::ReverseDiffusion
        ) && !item.supports_vector_transitions()
        {
            return Err("Vector transitions are only supported for SVGs, text, and shapes".into());
        }
    }
    Ok(())
}

fn validate_transition_durations(
    item_duration: Time,
    intro: Option<Time>,
    outro: Option<Time>,
) -> Result<(), String> {
    if intro.is_some_and(|duration| duration < Time::ZERO)
        || outro.is_some_and(|duration| duration < Time::ZERO)
    {
        return Err("transition duration must be nonnegative".into());
    }
    let intro = intro.unwrap_or(Time::ZERO);
    let outro = outro.unwrap_or(Time::ZERO);
    if intro.saturating_add(outro) > item_duration {
        return Err("intro and outro durations overlap".into());
    }
    Ok(())
}

fn validate_track_times(
    kind: &str,
    track_index: usize,
    items: impl Iterator<Item = (Time, Time)>,
) -> Result<(), String> {
    let mut previous_end = None;
    for (item_index, (start, end)) in items.enumerate() {
        if start >= end {
            return Err(format!(
                "{kind} track {}, item {} has a non-positive duration",
                track_index + 1,
                item_index + 1,
            ));
        }
        if previous_end.is_some_and(|previous_end| previous_end > start) {
            return Err(format!(
                "{kind} track {}, item {} overlaps the preceding item",
                track_index + 1,
                item_index + 1,
            ));
        }
        previous_end = Some(end);
    }
    Ok(())
}

fn repair_frame_aligned_times(
    start: &mut Time,
    end: &mut Time,
    previous_end: &mut Option<Time>,
    fps: Fraction,
    frame_step: Time,
) -> bool {
    let original_start = *start;
    let original_end = *end;
    let duration_frames = original_end
        .saturating_sub(original_start)
        .as_frame_ceil(fps)
        .max(1);
    let duration = shrimply_math_core::time_from_frame(duration_frames, fps)
        .expect("validated project duration must fit the frame grid");
    *start = original_start.snapped(frame_step);
    if let Some(previous_end) = previous_end {
        *start = (*start).max(*previous_end);
    }
    *end = original_end
        .snapped(frame_step)
        .max(start.saturating_add(duration));
    *previous_end = Some(*end);
    *start != original_start || *end != original_end
}

fn validate_project_frame_alignment(project: &Project) -> Result<(), String> {
    let frame_step = project.frame_step();
    for (track_index, track) in project.caption_tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            validate_frame_aligned_times(
                "caption",
                track_index,
                item_index,
                item.start,
                item.end,
                frame_step,
            )?;
        }
    }

    for (track_index, track) in project
        .video_tracks
        .iter()
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.video_tracks),
        )
        .enumerate()
    {
        for (item_index, item) in track.items.iter().enumerate() {
            validate_frame_aligned_times(
                "visual",
                track_index,
                item_index,
                item.start,
                item.end,
                frame_step,
            )?;
        }
    }
    for (track_index, track) in project
        .audio_tracks
        .iter()
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.audio_tracks),
        )
        .enumerate()
    {
        for (item_index, item) in track.items.iter().enumerate() {
            validate_frame_aligned_times(
                "audio",
                track_index,
                item_index,
                item.start,
                item.end,
                frame_step,
            )?;
        }
    }
    Ok(())
}

fn validate_frame_aligned_times(
    kind: &str,
    track_index: usize,
    item_index: usize,
    start: Time,
    end: Time,
    frame_step: Time,
) -> Result<(), String> {
    if start != start.snapped(frame_step) || end != end.snapped(frame_step) {
        return Err(format!(
            "{kind} track {}, item {} is not aligned to the project frame grid",
            track_index + 1,
            item_index + 1,
        ));
    }
    Ok(())
}

fn validate_visual_clip_transitions(
    track_index: usize,
    items: &[VisualItem],
) -> Result<(), String> {
    for (item_index, pair) in items.windows(2).enumerate() {
        let outgoing = &pair[0];
        let incoming = &pair[1];
        let Some(transition) = outgoing.transitions.to_next.as_ref() else {
            continue;
        };
        validate_clip_transition(
            "visual",
            track_index,
            item_index,
            outgoing.start,
            outgoing.end,
            outgoing
                .transitions
                .intro
                .as_ref()
                .map(|value| value.duration),
            outgoing.transitions.outro.is_some(),
            incoming.id,
            incoming.start,
            incoming.end,
            incoming.transitions.intro.is_some(),
            incoming
                .transitions
                .outro
                .as_ref()
                .map(|value| value.duration),
            transition.target_item_id,
            transition.duration,
        )?;
        validate_visual_clip_transition_config(transition)?;
    }
    if items
        .last()
        .is_some_and(|item| item.transitions.to_next.is_some())
    {
        return Err(format!(
            "visual track {}, last item has a transition to a missing next item",
            track_index + 1
        ));
    }
    Ok(())
}

fn validate_visual_clip_transition_config(transition: &VisualClipTransition) -> Result<(), String> {
    if matches!(
        transition.kind,
        VisualClipTransitionKind::Wipe
            | VisualClipTransitionKind::ClockWipe
            | VisualClipTransitionKind::Slide
            | VisualClipTransitionKind::Push
    ) && !transition.direction_degrees.is_finite()
    {
        return Err("visual clip transition direction must be finite".into());
    }
    let maximum_softness = match transition.kind {
        VisualClipTransitionKind::ClockWipe => Some(MAX_VISUAL_CLIP_TRANSITION_CLOCK_SOFTNESS),
        VisualClipTransitionKind::Wipe | VisualClipTransitionKind::Iris => {
            Some(MAX_VISUAL_CLIP_TRANSITION_SOFTNESS)
        }
        _ => None,
    };
    if maximum_softness.is_some_and(|maximum| {
        !transition.softness.is_finite() || !(0.0..=maximum).contains(&transition.softness)
    }) {
        return Err("visual clip transition softness is out of range".into());
    }
    if matches!(
        transition.kind,
        VisualClipTransitionKind::Iris
            | VisualClipTransitionKind::ClockWipe
            | VisualClipTransitionKind::Zoom
    ) && (!transition.center.is_finite()
        || !(0.0..=1.0).contains(&transition.center.x)
        || !(0.0..=1.0).contains(&transition.center.y))
    {
        return Err("visual clip transition center must be between 0 and 1".into());
    }
    if transition.kind == VisualClipTransitionKind::FadeThroughColor
        && transition.fade_color.a != u8::MAX
    {
        return Err("fade-through color must be opaque".into());
    }
    if transition.kind == VisualClipTransitionKind::Dissolve
        && !(1..=MAX_VISUAL_CLIP_TRANSITION_DISSOLVE_GRAIN_SIZE)
            .contains(&transition.dissolve_grain_size)
    {
        return Err("visual clip transition grain size is out of range".into());
    }
    if transition.kind == VisualClipTransitionKind::Zoom
        && (!transition.zoom_start_scale.is_finite()
            || !(0.0..=MAX_VISUAL_CLIP_TRANSITION_ZOOM_SCALE)
                .contains(&transition.zoom_start_scale))
    {
        return Err("visual clip transition zoom scale is out of range".into());
    }
    Ok(())
}

fn normalize_visual_clip_transitions(items: &mut [VisualItem]) {
    for index in 0..items.len() {
        let valid = items[index]
            .transitions
            .to_next
            .as_ref()
            .zip(items.get(index + 1))
            .filter(|(transition, incoming)| {
                transition.target_item_id == incoming.id
                    && items[index].end == incoming.start
                    && items[index].transitions.outro.is_none()
                    && incoming.transitions.intro.is_none()
            })
            .map(|(_, incoming)| {
                shrimply_math_media::maximum_clip_transition_duration(
                    items[index].end.saturating_sub(items[index].start),
                    incoming.end.saturating_sub(incoming.start),
                    items[index]
                        .transitions
                        .intro
                        .as_ref()
                        .map(|transition| transition.duration),
                    incoming
                        .transitions
                        .outro
                        .as_ref()
                        .map(|transition| transition.duration),
                )
            });
        match valid {
            Some(maximum) => {
                let transition = items[index]
                    .transitions
                    .to_next
                    .as_mut()
                    .expect("validated transition");
                transition.duration = transition.duration.min(maximum);
                if transition.duration <= Time::ZERO {
                    items[index].transitions.to_next = None;
                }
            }
            None => {
                items[index].transitions.to_next = None;
            }
        }
    }
}

fn normalize_audio_clip_transitions(items: &mut [AudioItem]) {
    for index in 0..items.len() {
        let valid = items[index]
            .transitions
            .to_next
            .as_ref()
            .zip(items.get(index + 1))
            .filter(|(transition, incoming)| {
                transition.target_item_id == incoming.id
                    && items[index].end == incoming.start
                    && items[index].transitions.outro.is_none()
                    && incoming.transitions.intro.is_none()
            })
            .map(|(_, incoming)| {
                shrimply_math_media::maximum_clip_transition_duration(
                    items[index].end.saturating_sub(items[index].start),
                    incoming.end.saturating_sub(incoming.start),
                    items[index]
                        .transitions
                        .intro
                        .as_ref()
                        .map(|transition| transition.duration),
                    incoming
                        .transitions
                        .outro
                        .as_ref()
                        .map(|transition| transition.duration),
                )
            });
        match valid {
            Some(maximum) => {
                let transition = items[index]
                    .transitions
                    .to_next
                    .as_mut()
                    .expect("validated transition");
                transition.duration = transition.duration.min(maximum);
                if transition.duration <= Time::ZERO {
                    items[index].transitions.to_next = None;
                }
            }
            None => {
                items[index].transitions.to_next = None;
            }
        }
    }
}

fn validate_audio_clip_transitions(track_index: usize, items: &[AudioItem]) -> Result<(), String> {
    for (item_index, pair) in items.windows(2).enumerate() {
        let outgoing = &pair[0];
        let incoming = &pair[1];
        let Some(transition) = outgoing.transitions.to_next.as_ref() else {
            continue;
        };
        validate_clip_transition(
            "audio",
            track_index,
            item_index,
            outgoing.start,
            outgoing.end,
            outgoing
                .transitions
                .intro
                .as_ref()
                .map(|value| value.duration),
            outgoing.transitions.outro.is_some(),
            incoming.id,
            incoming.start,
            incoming.end,
            incoming.transitions.intro.is_some(),
            incoming
                .transitions
                .outro
                .as_ref()
                .map(|value| value.duration),
            transition.target_item_id,
            transition.duration,
        )?;
    }
    if items
        .last()
        .is_some_and(|item| item.transitions.to_next.is_some())
    {
        return Err(format!(
            "audio track {}, last item has a transition to a missing next item",
            track_index + 1
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_clip_transition(
    kind: &str,
    track_index: usize,
    item_index: usize,
    outgoing_start: Time,
    outgoing_end: Time,
    outgoing_intro: Option<Time>,
    outgoing_outro: bool,
    incoming_id: Uuid,
    incoming_start: Time,
    incoming_end: Time,
    incoming_intro: bool,
    incoming_outro: Option<Time>,
    target_item_id: Uuid,
    duration: Time,
) -> Result<(), String> {
    let label = format!("{kind} track {}, item {}", track_index + 1, item_index + 1);
    if target_item_id != incoming_id || outgoing_end != incoming_start {
        return Err(format!("{label} has a transition to a non-adjacent item"));
    }
    if outgoing_outro || incoming_intro {
        return Err(format!(
            "{label} has an edit-point transition overlapping an outro or intro"
        ));
    }
    if duration <= Time::ZERO {
        return Err(format!("{label} has a non-positive edit-point transition"));
    }
    let outgoing_duration = outgoing_end.saturating_sub(outgoing_start);
    let incoming_duration = incoming_end.saturating_sub(incoming_start);
    let shortest = outgoing_duration.min(incoming_duration);
    if duration.seconds * Fraction::from(2_u8) > shortest.seconds {
        return Err(format!(
            "{label} edit-point transition exceeds half the shorter item"
        ));
    }
    let doubled_outgoing = outgoing_duration.seconds * Fraction::from(2_u8);
    let doubled_incoming = incoming_duration.seconds * Fraction::from(2_u8);
    if outgoing_intro.is_some_and(|intro| {
        intro.seconds * Fraction::from(2_u8) + duration.seconds > doubled_outgoing
    }) || incoming_outro.is_some_and(|outro| {
        outro.seconds * Fraction::from(2_u8) + duration.seconds > doubled_incoming
    }) {
        return Err(format!(
            "{label} edit-point transition overlaps another transition"
        ));
    }
    Ok(())
}

fn validate_folded_sequence(sequence: &FoldedSequence) -> Result<(), String> {
    for (track_index, track) in sequence.video_tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            item.modifier_output_kind().map_err(|error| {
                format!(
                    "invalid modifier chain in folded sequence {} visual track {}, item {}: {error}",
                    sequence.id,
                    track_index + 1,
                    item_index + 1
                )
            })?;
            validate_visual_alpha_masks(item)?;
            validate_visual_transitions(item)?;
            paint::validate_item(item)?;
        }
        validate_track_times(
            "folded visual",
            track_index,
            track.items.iter().map(|item| (item.start, item.end)),
        )?;
        validate_visual_clip_transitions(track_index, &track.items)?;
    }
    for (track_index, track) in sequence.audio_tracks.iter().enumerate() {
        for item in &track.items {
            validate_transition_durations(
                item.end.saturating_sub(item.start),
                item.transitions.intro.as_ref().map(|value| value.duration),
                item.transitions.outro.as_ref().map(|value| value.duration),
            )?;
        }
        validate_track_times(
            "folded audio",
            track_index,
            track.items.iter().map(|item| (item.start, item.end)),
        )?;
        validate_audio_clip_transitions(track_index, &track.items)?;
    }
    Ok(())
}

fn validate_visual_alpha_masks(item: &VideoItem) -> Result<(), String> {
    for modifier in &item.modifiers {
        if modifier.alpha_mask.is_some() && !matches!(modifier.effect, ModifierEffect::Raster(_)) {
            return Err(format!(
                "modifier {} has an alpha mask outside the raster stage",
                modifier.id
            ));
        }
        if let Some(mask) = &modifier.alpha_mask {
            validate_visual_alpha_mask(mask)?;
        }
    }
    if let Some(mask) = &item.compositing.alpha_mask {
        validate_visual_alpha_mask(mask)?;
    }
    if item
        .compositing
        .alpha_mask
        .as_ref()
        .is_some_and(|mask| mask.enabled)
        && item.modifier_output_kind()? != shrimply_video_modifiers::VisualKind::Raster
    {
        return Err("enabled compositing alpha mask requires raster output".to_string());
    }
    Ok(())
}

fn validate_visual_alpha_mask(mask: &VisualAlphaMask) -> Result<(), String> {
    if mask.shape == AlphaMaskShape::Polygon
        && (mask.vertices.len() < 3 || mask.vertices.iter().any(|point| !point.is_finite()))
    {
        return Err("polygon alpha mask requires at least three finite vertices".to_string());
    }
    Ok(())
}

fn validate_sequence_references(project: &Project) -> Result<(), String> {
    fn visit(
        project: &Project,
        id: Uuid,
        visiting: &mut Vec<Uuid>,
        visited: &mut HashSet<Uuid>,
    ) -> Result<(), String> {
        if visited.contains(&id) {
            return Ok(());
        }
        if visiting.contains(&id) {
            return Err(format!("cyclic folded sequence reference involving {id}"));
        }
        let sequence = project
            .folded_sequence(id)
            .ok_or_else(|| format!("missing folded sequence {id}"))?;
        visiting.push(id);
        for reference in sequence_references(&sequence.video_tracks, &sequence.audio_tracks) {
            visit(project, reference.sequence_id, visiting, visited)?;
        }
        visiting.pop();
        visited.insert(id);
        Ok(())
    }

    for reference in sequence_references(&project.video_tracks, &project.audio_tracks) {
        if project.folded_sequence(reference.sequence_id).is_none() {
            return Err(format!("missing folded sequence {}", reference.sequence_id));
        }
    }
    let mut visiting = Vec::new();
    let mut visited = HashSet::new();
    for sequence in &project.folded_sequences {
        visit(project, sequence.id, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn sequence_references<'a>(
    video_tracks: &'a [VisualTrack],
    audio_tracks: &'a [AudioTrack],
) -> impl Iterator<Item = SequenceReference> + 'a {
    video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .filter_map(|item| match &item.content {
            VisualSource::FoldedSequence(reference) => Some(*reference),
            _ => None,
        })
        .chain(
            audio_tracks
                .iter()
                .flat_map(|track| &track.items)
                .filter_map(|item| match &item.source {
                    AudioSource::FoldedSequence(reference) => Some(*reference),
                    AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
                }),
        )
}

fn sequence_reference_items<'a>(
    video_tracks: &'a [VisualTrack],
    audio_tracks: &'a [AudioTrack],
) -> impl Iterator<Item = (Uuid, SequenceReference)> + 'a {
    video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .filter_map(|item| match &item.content {
            VisualSource::FoldedSequence(reference) => Some((item.id, *reference)),
            _ => None,
        })
        .chain(
            audio_tracks
                .iter()
                .flat_map(|track| &track.items)
                .filter_map(|item| match &item.source {
                    AudioSource::FoldedSequence(reference) => Some((item.id, *reference)),
                    AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
                }),
        )
}

static ACTIVE_PROJECT_PATH: OnceLock<RwLock<PathBuf>> = OnceLock::new();

pub fn set_active_project_path(path: &Path) {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    *ACTIVE_PROJECT_PATH
        .get_or_init(|| RwLock::new(PathBuf::new()))
        .write()
        .unwrap_or_else(|_| panic!("active project path lock died")) = path;
}

pub fn active_project_path() -> PathBuf {
    ACTIVE_PROJECT_PATH
        .get_or_init(|| RwLock::new(PathBuf::new()))
        .read()
        .unwrap_or_else(|_| panic!("active project path lock died"))
        .clone()
}

pub fn project_directory() -> PathBuf {
    active_project_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}
