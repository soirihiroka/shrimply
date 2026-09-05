use serde_json::Value;
use shrimply_core::timeline_value::{TimelineBase, TimelineBool, TimelineValue};
use shrimply_project::project::{
    SkiaDrawingStrategy, Time, VideoItem, VideoItemContent, VideoStabilizationMethod,
    VisualCompositing,
};
use shrimply_state::player_state;
use shrimply_video_modifiers::VisualKind;

use crate::{
    ControlKind, GraphPoint, InspectorControl, InspectorControlAction, InspectorController,
    InspectorRuntime, InspectorSection, InspectorTarget, LayeredState, NumberSpec, ScalarGraph,
};

pub mod blender;
pub mod pdf;
pub mod playback;

#[derive(Clone, Debug, PartialEq)]
pub struct VideoPresentation {
    pub item_id: uuid::Uuid,
    pub title: &'static str,
    pub value: Value,
    pub visual: Vec<VideoCard>,
    pub modifiers: Vec<crate::VisualModifierPresentation>,
    pub modifier_choices: Vec<crate::VisualModifierChoice>,
    pub playback: Vec<VideoCard>,
    pub stream: Option<VideoStreamPresentation>,
    pub source_metadata: crate::info::SourceMetadata,
    pub blender: Option<BlenderSourcePresentation>,
    pub manim: Option<crate::manim_parameters::ManimPresentation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlenderSourcePresentation {
    pub item: shrimply_project::project::BlenderItem,
    pub asset: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoStreamPresentation {
    pub selected: u32,
    pub alpha_mask: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoCard {
    pub key: &'static str,
    pub title: &'static str,
    pub section: InspectorSection,
    pub reset: Option<VideoReset>,
    pub alpha_mask: Option<crate::AlphaMaskPresentation>,
    pub preview_facet: Option<shrimply_preview_core::PreviewFacetKey>,
    pub actions: Vec<crate::item::HeaderAction<VideoCardAction>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VideoCardAction {
    ReloadAsset { asset: String, kind: ReloadKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadKind {
    Blender,
    Manim,
}

pub fn reload_asset(asset: &str, kind: ReloadKind) -> Result<(), String> {
    let source = shrimply_project::project::Asset::from(std::path::Path::new(asset));
    match kind {
        ReloadKind::Blender => {
            shrimply_blender::invalidate_metadata(source.path());
            source
                .mark_dirty()
                .map_err(|error| format!("could not mark Blender source dirty: {error}"))
        }
        ReloadKind::Manim => crate::manim_parameters::reload_source(&source),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoReset {
    pub values: Vec<(String, Value)>,
    pub fraction: Option<(String, shrimply_math_core::Fraction)>,
    pub commit_name: &'static str,
    pub cancel_stabilization: bool,
    pub paint_palette: bool,
}

impl VideoCard {
    pub(crate) fn new(key: &'static str, title: &'static str, section: InspectorSection) -> Self {
        Self {
            key,
            title,
            section,
            reset: None,
            alpha_mask: None,
            preview_facet: None,
            actions: Vec::new(),
        }
    }

    pub(crate) fn actions(
        mut self,
        actions: impl IntoIterator<Item = crate::item::HeaderAction<VideoCardAction>>,
    ) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    pub(crate) fn preview_facet(mut self, facet: shrimply_preview_core::PreviewFacetKey) -> Self {
        self.preview_facet = Some(facet);
        self
    }

    pub(crate) fn reset(
        mut self,
        path: impl Into<String>,
        value: Value,
        commit_name: &'static str,
    ) -> Self {
        self.reset = Some(VideoReset {
            values: vec![(path.into(), value)],
            fraction: None,
            commit_name,
            cancel_stabilization: false,
            paint_palette: false,
        });
        self
    }

    fn reset_fraction(
        mut self,
        path: impl Into<String>,
        value: shrimply_math_core::Fraction,
        commit_name: &'static str,
    ) -> Self {
        self.reset = Some(VideoReset {
            values: Vec::new(),
            fraction: Some((path.into(), value)),
            commit_name,
            cancel_stabilization: false,
            paint_palette: false,
        });
        self
    }

    pub(crate) fn reset_fields(
        mut self,
        values: impl IntoIterator<Item = (impl Into<String>, Value)>,
        commit_name: &'static str,
    ) -> Self {
        self.reset = Some(VideoReset {
            values: values
                .into_iter()
                .map(|(path, value)| (path.into(), value))
                .collect(),
            fraction: None,
            commit_name,
            cancel_stabilization: false,
            paint_palette: false,
        });
        self
    }

    fn cancel_stabilization(mut self) -> Self {
        self.reset
            .as_mut()
            .expect("video card must have a reset before adding reset behavior")
            .cancel_stabilization = true;
        self
    }

    pub(crate) fn paint_palette_reset(mut self) -> Self {
        self.reset
            .as_mut()
            .expect("paint palette card must have a reset")
            .paint_palette = true;
        self
    }
}

impl InspectorController {
    pub fn reset_video(&self, target: &InspectorTarget, reset: &VideoReset) -> Result<(), String> {
        let InspectorTarget::Item(address @ shrimply_project::project::ItemAddress::Video { .. }) =
            target
        else {
            return Err("inspector target is not a video item".to_string());
        };
        if reset.commit_name.is_empty() {
            return Err("video reset has no commit name".to_string());
        }
        if reset.paint_palette {
            return self.reset_paint_palette(target);
        }
        if let [(path, page)] = reset.values.as_slice()
            && path == pdf::PAGE_PATH
        {
            let page = page
                .as_u64()
                .and_then(|page| u32::try_from(page).ok())
                .ok_or_else(|| "PDF reset page must be an unsigned integer".to_string())?;
            return self.set_pdf_page(target, page, reset.commit_name);
        }
        let mut project = self.project.borrow_mut();
        if let Some((path, value)) = &reset.fraction {
            let item = project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?;
            if !playback::set_fraction(item, path, *value)? {
                return Ok(());
            }
            let duration = (path == "/playback_speed").then(|| project.duration());
            shrimply_project::project::commit_edit(&project, reset.commit_name);
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    duration,
                    video: true,
                    inspector: true,
                    ..player_state::ProjectChange::default()
                },
            );
            return Ok(());
        }
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let mut value = serde_json::to_value(item).expect("video inspector item must serialize");
        let mut changed = false;
        for (path, replacement) in &reset.values {
            let current = value
                .pointer_mut(path)
                .ok_or_else(|| format!("video reset field is no longer available: {path}"))?;
            if current != replacement {
                *current = replacement.clone();
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
        if matches!(item.content, VideoItemContent::Paint(_)) {
            crate::paint::bump_serialized_revision(&mut value)?;
        }
        if reset.cancel_stabilization {
            shrimply_video_cuda::video_stabilization::cancel(
                project
                    .video_item(address)
                    .expect("validated video item must remain available"),
            );
        }
        crate::model::replace_target(&mut project, target, value)?;
        shrimply_project::project::commit_edit(&project, reset.commit_name);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }

    pub fn set_video_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
        commit_name: &str,
        commit_immediately: bool,
    ) -> Result<(), String> {
        if path == pdf::PAGE_PATH {
            let displayed = text
                .parse::<f64>()
                .map_err(|_| format!("invalid PDF page: {text}"))?;
            if !displayed.is_finite() || displayed < 1.0 {
                return Err("PDF page must be a positive integer".to_string());
            }
            return self.set_pdf_page(
                target,
                displayed.round().min(f64::from(u32::MAX)) as u32 - 1,
                commit_name,
            );
        }
        if let Some(result) = self.set_camera_source_field(target, path, text) {
            return result;
        }
        if path == "/stabilization_method" {
            return self.set_video_stabilization_method(target, text);
        }
        if path == "/playback_fps" {
            return self.set_video_frame_rate(target, text, commit_name);
        }
        validate_video_edit(target, commit_name)?;
        let mut project = self.project.borrow_mut();
        let InspectorTarget::Item(address) = target else {
            unreachable!("validated video target must be an item")
        };
        if let Some(changed) = playback::set_field(
            project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?,
            path,
            text,
        ) {
            if !changed? {
                return Ok(());
            }
            if commit_immediately {
                shrimply_project::project::commit_edit(&project, commit_name);
            }
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: true,
                    inspector: path == "/motion_blur/enabled",
                    ..player_state::ProjectChange::default()
                },
            );
            return Ok(());
        }
        if let Some(changed) = blender::set_field(
            project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?,
            path,
            text,
        ) {
            if !changed {
                return Ok(());
            }
            if commit_immediately {
                shrimply_project::project::commit_edit(&project, commit_name);
            }
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: true,
                    inspector: path == blender::SCENE_PATH,
                    ..Default::default()
                },
            );
            return Ok(());
        }
        if let Some(changed) = crate::background::set_kind(
            project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?,
            path,
            text,
        ) {
            if !changed? {
                return Ok(());
            }
            if commit_immediately {
                shrimply_project::project::commit_edit(&project, commit_name);
            }
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: true,
                    inspector: true,
                    ..player_state::ProjectChange::default()
                },
            );
            return Ok(());
        }
        if let Some(changed) = crate::visual_modifiers::set_visual_modifier_field(
            project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?,
            path,
            text,
        ) {
            if !changed? {
                return Ok(());
            }
            if commit_immediately {
                shrimply_project::project::commit_edit(&project, commit_name);
            }
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: true,
                    inspector: commit_immediately,
                    ..player_state::ProjectChange::default()
                },
            );
            return Ok(());
        }
        if let Some(changed) = crate::generated::set_field(
            project
                .video_item_mut(address)
                .ok_or_else(|| "video item is no longer available".to_string())?,
            path,
            text,
        ) {
            if !changed? {
                return Ok(());
            }
            if commit_immediately {
                shrimply_project::project::commit_edit(&project, commit_name);
            }
            drop(project);
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: true,
                    inspector: true,
                    ..player_state::ProjectChange::default()
                },
            );
            return Ok(());
        }
        let mut value = crate::model::target_value(&project, target)
            .ok_or_else(|| "video item is no longer available".to_string())?
            .1;
        let current = value
            .pointer_mut(path)
            .ok_or_else(|| format!("video field is no longer available: {path}"))?;
        let replacement = crate::model::parsed_value(current, text)?;
        if *current == replacement {
            return Ok(());
        }
        *current = replacement;
        if project
            .video_item(address)
            .is_some_and(|item| matches!(item.content, VideoItemContent::Paint(_)))
        {
            crate::paint::bump_serialized_revision(&mut value)?;
        }
        crate::model::replace_target(&mut project, target, value)?;
        if stabilization_setting(path) {
            let InspectorTarget::Item(address) = target else {
                unreachable!("validated video target must be an item")
            };
            let item = project
                .video_item(address)
                .expect("replaced video item must remain available");
            if item.stabilize_video {
                shrimply_video_cuda::video_stabilization::request(item);
            }
        }
        if commit_immediately {
            shrimply_project::project::commit_edit(&project, commit_name);
        }
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                video: true,
                inspector: matches!(
                    path,
                    "/motion_blur/enabled"
                        | crate::scene_3d::CAMERA_PROJECTION_PATH
                        | crate::scene_3d::CAMERA_BACKGROUND_ENABLED_PATH
                        | crate::scene_3d::CAMERA_BACKGROUND_ADDRESS_PATH
                        | crate::scene_3d::SHADING_PATH
                        | crate::scene_3d::OUTLINE_MODE_PATH
                        | crate::scene_3d::OUTLINE_METHOD_PATH
                        | crate::scene_3d::TEXTURE_FILTER_PATH
                        | crate::scene_3d::SHADOW_KIND_PATH
                        | crate::scene_3d::PATH_TRACING_PATH
                        | crate::scene_3d::LIGHT_SAMPLING_PATH
                        | crate::scene_3d::OPTIX_DENOISING_PATH
                        | crate::scene_3d::ENVIRONMENT_SOURCE_PATH
                ),
                ..player_state::ProjectChange::default()
            },
        );
        Ok(())
    }

    pub fn set_video_fraction(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: shrimply_math_core::Fraction,
        commit_name: &str,
    ) -> Result<(), String> {
        validate_video_edit(target, commit_name)?;
        let InspectorTarget::Item(address) = target else {
            unreachable!("validated video target must be an item")
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !playback::set_fraction(item, path, value)? {
            return Ok(());
        }
        let duration = (path == "/playback_speed").then(|| project.duration());
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                duration,
                video: true,
                ..player_state::ProjectChange::default()
            },
        );
        Ok(())
    }

    fn set_video_frame_rate(
        &self,
        target: &InspectorTarget,
        text: &str,
        commit_name: &str,
    ) -> Result<(), String> {
        validate_video_edit(target, commit_name)?;
        let (numerator, denominator) = text
            .split_once('/')
            .ok_or_else(|| format!("invalid video frame rate: {text}"))?;
        let numerator = numerator
            .parse::<i64>()
            .map_err(|_| format!("invalid video frame-rate numerator: {numerator}"))?;
        let denominator = denominator
            .parse::<i64>()
            .map_err(|_| format!("invalid video frame-rate denominator: {denominator}"))?;
        if numerator < 0 || denominator <= 0 {
            return Err("video frame rate must be nonnegative and finite".to_string());
        }
        let value = shrimply_math_core::fraction_new(numerator, denominator);
        let InspectorTarget::Item(address) = target else {
            unreachable!("validated video target must be an item")
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !playback::set_fraction(item, "/playback_fps", value)? {
            return Ok(());
        }
        shrimply_project::project::commit_edit(&project, commit_name);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                video: true,
                ..player_state::ProjectChange::default()
            },
        );
        Ok(())
    }

    pub fn commit_video_field(
        &self,
        target: &InspectorTarget,
        commit_name: &str,
    ) -> Result<(), String> {
        validate_video_edit(target, commit_name)?;
        let project = self.project.borrow();
        crate::model::target_value(&project, target)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        shrimply_project::project::commit_edit(&project, commit_name);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                inspector: true,
                ..player_state::ProjectChange::default()
            },
        );
        Ok(())
    }

    pub fn trigger_video_control_action(
        &self,
        target: &InspectorTarget,
        action: InspectorControlAction,
    ) -> Result<(), String> {
        validate_video_target(target)?;
        let InspectorTarget::Item(address) = target else {
            unreachable!("validated video target must be an item")
        };
        match action {
            InspectorControlAction::RebuildVideoStabilization => {
                let project = self.project.borrow();
                let item = project
                    .video_item(address)
                    .ok_or_else(|| "video item is no longer available".to_string())?;
                if shrimply_video_cuda::video_stabilization::is_generating(item) {
                    shrimply_video_cuda::video_stabilization::cancel(item);
                } else {
                    let timeline_position = player_state::current_time(&self.player_state);
                    let source_position =
                        shrimply_project::project::video_source_time_at(item, timeline_position)
                            .unwrap_or(item.time_offset);
                    shrimply_video_cuda::video_stabilization::rebuild(item, source_position);
                }
            }
            InspectorControlAction::ClearMaskSource { modifier_id } => {
                self.clear_mask_source(target, modifier_id)?;
            }
            InspectorControlAction::SelectObject3dModel { .. } => {
                return Err("3D model selection requires a native file picker".to_string());
            }
            InspectorControlAction::ClearObject3dModel { modifier_id } => {
                self.clear_object_3d_model(target, modifier_id)?;
            }
            InspectorControlAction::SelectScene3dEnvironment => {
                return Err("scene environment selection requires a native file picker".to_string());
            }
            InspectorControlAction::ClearScene3dEnvironment => {
                self.clear_scene_3d_environment(target)?;
            }
            InspectorControlAction::AddDitheringPaletteColor { modifier_id } => {
                self.add_dithering_palette_color(target, modifier_id)?;
            }
            InspectorControlAction::RemoveDitheringPaletteColor {
                modifier_id,
                color_id,
            } => {
                self.remove_dithering_palette_color(target, modifier_id, color_id)?;
            }
            InspectorControlAction::AddPaintPaletteColor => {
                self.add_paint_palette_color(target)?;
            }
            InspectorControlAction::RemovePaintPaletteColor { color_id } => {
                self.remove_paint_palette_color(target, color_id)?;
            }
            InspectorControlAction::SelectPaintTexture { .. } => {
                return Err("paint texture selection requires a native file picker".to_string());
            }
            InspectorControlAction::ClearPaintTexture { color_id } => {
                self.clear_paint_texture(target, color_id)?;
            }
            InspectorControlAction::RemoveSam2Point {
                modifier_id,
                point_id,
            } => {
                self.remove_sam2_point(target, modifier_id, point_id)?;
            }
            InspectorControlAction::SetSam2PointLabel { .. } => {
                return Err("SAM2 point type must be selected from its control".to_string());
            }
            InspectorControlAction::SetSam2Model { .. }
            | InspectorControlAction::SetSam2PointPosition { .. } => {
                return Err("SAM2 value must be edited through its control".to_string());
            }
            InspectorControlAction::RemoveSam2Box {
                modifier_id,
                box_id,
            } => {
                self.remove_sam2_box(target, modifier_id, box_id)?;
            }
            InspectorControlAction::ToggleSam2Analysis { .. } => {
                return Err("SAM2 analysis requires the configured compute server".to_string());
            }
            InspectorControlAction::RemoveTransparentFillPoint {
                modifier_id,
                point_id,
            } => {
                self.remove_transparent_fill_point(target, modifier_id, point_id)?;
            }
            InspectorControlAction::ToggleTransparentFillAnalysis { modifier_id } => {
                self.toggle_transparent_fill_analysis(target, modifier_id)?;
            }
            InspectorControlAction::ToggleCameraAnalysis => {
                return Err("camera analysis requires the configured compute server".to_string());
            }
        }
        Ok(())
    }

    pub fn video_stabilization_generating(&self, target: &InspectorTarget) -> Option<bool> {
        let InspectorTarget::Item(address @ shrimply_project::project::ItemAddress::Video { .. }) =
            target
        else {
            return None;
        };
        self.project
            .borrow()
            .video_item(address)
            .map(shrimply_video_cuda::video_stabilization::is_generating)
    }
}

fn validate_video_edit(target: &InspectorTarget, commit_name: &str) -> Result<(), String> {
    validate_video_target(target)?;
    if commit_name.is_empty() {
        Err("video edit has no commit name".to_string())
    } else {
        Ok(())
    }
}

fn validate_video_target(target: &InspectorTarget) -> Result<(), String> {
    if matches!(
        target,
        InspectorTarget::Item(shrimply_project::project::ItemAddress::Video { .. })
    ) {
        Ok(())
    } else {
        Err("inspector target is not a video item".to_string())
    }
}

fn stabilization_setting(path: &str) -> bool {
    path.starts_with("/stabilization_") || path.starts_with("/mesh_flow_")
}

pub fn presentation(
    project: &shrimply_project::project::Project,
    address: &shrimply_project::project::ItemAddress,
    item: &VideoItem,
    runtime: InspectorRuntime,
    camera_models: Option<&Result<Vec<String>, String>>,
    default_text_font: Option<&shrimply_core::FontFamily>,
) -> VideoPresentation {
    let value = serde_json::to_value(item).expect("video inspector item must serialize");
    let static_visual = item.is_static_visual_media() || item.is_generated();

    let mut visual_items = Vec::new();
    let mut playback_items = Vec::new();

    if matches!(item.content, VideoItemContent::Media) {
        visual_items.push(stabilization_item(item));
    }

    if let Some(cards) =
        crate::generated::cards(item, project.canvas_size, runtime, default_text_font)
    {
        visual_items.extend(cards);
    }
    if let VideoItemContent::Paint(paint) = &item.content {
        visual_items.extend(crate::paint::cards(paint, project.canvas_size, runtime));
    }
    if matches!(item.content, VideoItemContent::Pdf(_)) {
        visual_items.push(pdf::card(item));
    }

    if !static_visual {
        playback_items.push(playback::speed(item));
    }
    if !matches!(
        item.content,
        VideoItemContent::Media | VideoItemContent::Gif | VideoItemContent::Background(_)
    ) {
        playback_items.push(playback::frame_rate(item));
    }
    if item.source_visual_kind() == VisualKind::Vector {
        visual_items.push(skia_drawing_item(&value));
    }
    if !matches!(
        item.content,
        VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_) | VideoItemContent::Background(_)
    ) {
        playback_items.push(playback::motion_blur(item));
    }
    if !matches!(item.content, VideoItemContent::Background(_)) {
        playback_items.push(playback::repeat(item));
    }

    if let VideoItemContent::Background(background) = &item.content {
        visual_items.push(crate::background::card(background, runtime));
    }

    match &item.content {
        VideoItemContent::Obj(scene) => visual_items.extend(crate::scene_3d::cards(
            project,
            address,
            scene,
            runtime,
            camera_models,
        )),
        VideoItemContent::Gaussian(scene) => visual_items.extend(crate::gaussian_3d::cards(
            project,
            address,
            scene,
            runtime,
            camera_models,
        )),
        _ => {}
    }

    visual_items.push(compositing_item(
        item,
        &value,
        runtime,
        item.source_visual_kind() == VisualKind::Raster,
    ));
    if let Some(transform) = crate::transform::card(project, address, item, runtime) {
        visual_items.push(transform);
    }

    VideoPresentation {
        item_id: item.id,
        title: crate::model::video_title(item),
        value,
        visual: visual_items,
        modifiers: crate::visual_modifier_presentations(project, address, item, runtime),
        modifier_choices: crate::visual_modifier_catalog(item),
        playback: playback_items,
        stream: matches!(item.content, VideoItemContent::Media).then_some(
            VideoStreamPresentation {
                selected: item.track_id,
                alpha_mask: item.alpha_mask_video,
            },
        ),
        source_metadata: if matches!(
            item.content,
            VideoItemContent::Media | VideoItemContent::Gif
        ) {
            crate::info::SourceMetadata::Video(item.track_id)
        } else {
            crate::info::SourceMetadata::None
        },
        blender: match &item.content {
            VideoItemContent::Blender(blender) => Some(BlenderSourcePresentation {
                item: (**blender).clone(),
                asset: item.file.path().to_string_lossy().into_owned(),
            }),
            _ => None,
        },
        manim: crate::manim_parameters::presentation(item),
    }
}

fn skia_drawing_item(video: &Value) -> VideoCard {
    let mut section = InspectorSection::default();
    section.add(
        crate::selector::selector(
            "/skia_drawing_strategy",
            "Strategy",
            text(video, "/skia_drawing_strategy"),
            [
                ("immediate".to_string(), "Immediate".to_string()),
                ("picture".to_string(), "Picture".to_string()),
            ],
        )
        .immediate_commit("skia-drawing-strategy"),
    );
    VideoCard::new("skia-drawing", "Skia drawing", section).reset(
        "/skia_drawing_strategy",
        serde_json::to_value(SkiaDrawingStrategy::default())
            .expect("default Skia drawing strategy must serialize"),
        "reset-skia-drawing",
    )
}

fn compositing_item(
    item: &VideoItem,
    video: &Value,
    runtime: InspectorRuntime,
    show_upsampling: bool,
) -> VideoCard {
    let local_time = runtime.local_time.unwrap_or(Time::ZERO);
    let mut section = InspectorSection::default();
    section.add(layered_boolean(
        video,
        "/visibility",
        "Visible",
        item.visibility.value_at(local_time).get(),
        runtime,
    ));
    if show_upsampling {
        section.add(
            crate::selector::layered_step_selector(
                "/sample_method",
                "Upsampling",
                &item.sample_method,
                runtime,
            )
            .live_commit("video-upsampling"),
        );
    }
    section.add(
        layered_number(
            video,
            "/compositing/opacity",
            "Opacity",
            f64::from(item.compositing.opacity.value_at(local_time)) * 100.0,
            NumberSpec {
                minimum: 0.0,
                maximum: 100.0,
                drag_step: 1.0,
                digits: 0,
                unit: "%",
            },
            0.01,
        )
        .live_commit("visual-compositing-opacity"),
    );
    section.add(
        crate::selector::layered_step_selector(
            "/compositing/blend_mode",
            "Blend mode",
            &item.compositing.blend_mode,
            runtime,
        )
        .live_commit("video-compositing-blend-mode"),
    );

    let mut card = VideoCard::new("compositing", "Compositing", section).reset_fields(
        [
            (
                "/compositing",
                serde_json::to_value(VisualCompositing::default())
                    .expect("default visual compositing must serialize"),
            ),
            (
                "/visibility",
                serde_json::to_value(TimelineValue::<TimelineBool>::default())
                    .expect("default visibility must serialize"),
            ),
            (
                "/sample_method",
                serde_json::to_value(
                    TimelineValue::<shrimply_project::project::VideoSampleMethod>::default(),
                )
                .expect("default sample method must serialize"),
            ),
        ],
        "reset-video-compositing",
    );
    if item.modifier_output_kind().ok() == Some(VisualKind::Raster) {
        card.alpha_mask = Some(crate::alpha_mask::presentation(
            item.compositing.alpha_mask.as_ref(),
            "/compositing/alpha_mask",
            None,
            crate::alpha_mask::preview_focus(
                item.id,
                shrimply_project::project::VisualAlphaMaskTarget::Compositing,
                true,
            ),
            runtime,
        ));
    }
    card
}

fn stabilization_item(item: &VideoItem) -> VideoCard {
    let unavailable = item.alpha_mask_video.is_some();
    let method = item.stabilization_method();
    let selected = if item.stabilize_video {
        enum_text(method)
    } else {
        "off".to_string()
    };
    let mut section = InspectorSection::default();
    section.add(
        crate::selector::selector(
            "/stabilization_method",
            "Method",
            selected,
            [
                ("off".to_string(), "Off".to_string()),
                ("l1".to_string(), "L1".to_string()),
                ("mesh_flow".to_string(), "MeshFlow".to_string()),
            ],
        )
        .subtitle(stabilization_status(item))
        .sensitive(!unavailable)
        .busy(shrimply_video_cuda::video_stabilization::is_generating(
            item,
        ))
        .immediate_commit("video-stabilization-method"),
    );
    if method != VideoStabilizationMethod::Off {
        section.add(
            value_number(
                "/stabilization_crop_ratio",
                "Crop ratio",
                f64::from(item.stabilization_crop_ratio) * 100.0,
            )
            .subtitle("Visible source area after stabilization")
            .number(NumberSpec {
                minimum: 10.0,
                maximum: 100.0,
                drag_step: 1.0,
                digits: 0,
                unit: "%",
            })
            .store_multiplier(0.01)
            .sensitive(!unavailable)
            .immediate_commit("video-stabilization-crop-ratio"),
        );
    }
    if method == VideoStabilizationMethod::L1 {
        section.add(stabilization_weight(
            "/stabilization_first_derivative_weight",
            "Static-camera weight",
            "Preference for frames with no camera motion",
            item.stabilization_first_derivative_weight,
            "video-stabilization-first-derivative-weight",
            unavailable,
        ));
        section.add(stabilization_weight(
            "/stabilization_second_derivative_weight",
            "Constant-motion weight",
            "Preference for a steady camera velocity",
            item.stabilization_second_derivative_weight,
            "video-stabilization-second-derivative-weight",
            unavailable,
        ));
        section.add(stabilization_weight(
            "/stabilization_third_derivative_weight",
            "Constant-acceleration weight",
            "Preference for smoothly changing camera motion",
            item.stabilization_third_derivative_weight,
            "video-stabilization-third-derivative-weight",
            unavailable,
        ));
    } else if method == VideoStabilizationMethod::MeshFlow {
        section.add(
            value_number(
                "/mesh_flow_rows",
                "Mesh rows",
                f64::from(item.mesh_flow_rows),
            )
            .subtitle("Number of independently moving cell rows")
            .number(integer_spec(2.0, 32.0))
            .sensitive(!unavailable)
            .immediate_commit("mesh-flow-rows"),
        );
        section.add(
            value_number(
                "/mesh_flow_columns",
                "Mesh columns",
                f64::from(item.mesh_flow_columns),
            )
            .subtitle("Number of independently moving cell columns")
            .number(integer_spec(2.0, 32.0))
            .sensitive(!unavailable)
            .immediate_commit("mesh-flow-columns"),
        );
        section.add(
            value_number(
                "/mesh_flow_smoothing_radius",
                "Smoothing radius",
                f64::from(item.mesh_flow_smoothing_radius),
            )
            .subtitle("Neighboring frames considered on each side")
            .number(integer_spec(1.0, 120.0))
            .sensitive(!unavailable)
            .immediate_commit("mesh-flow-smoothing-radius"),
        );
        section.add(
            value_number(
                "/mesh_flow_iterations",
                "Optimization iterations",
                f64::from(item.mesh_flow_iterations),
            )
            .subtitle("Jacobi passes used to minimize the MeshFlow energy")
            .number(integer_spec(1.0, 500.0))
            .sensitive(!unavailable)
            .immediate_commit("mesh-flow-iterations"),
        );
        section.add(
            crate::selector::selector(
                "/mesh_flow_adaptive_weights",
                "Adaptive weights",
                enum_text(item.mesh_flow_adaptive_weights),
                [
                    ("original", "Original"),
                    ("flipped", "Flipped"),
                    ("constant_high", "Constant high"),
                    ("constant_low", "Constant low"),
                ]
                .map(|(value, label)| (value.to_string(), label.to_string())),
            )
            .tooltip("Motion-dependent temporal smoothing model")
            .sensitive(!unavailable)
            .immediate_commit("mesh-flow-adaptive-weights"),
        );
    }
    if method != VideoStabilizationMethod::Off {
        let generating = shrimply_video_cuda::video_stabilization::is_generating(item);
        section.add(
            InspectorControl::new(ControlKind::Action, "", "Stabilization cache")
                .subtitle("Discard and reanalyze the current source-time chunk")
                .value(if generating { "Cancel" } else { "Rebuild" })
                .sensitive(item.stabilize_video && !unavailable)
                .busy(generating)
                .action(InspectorControlAction::RebuildVideoStabilization),
        );
    }

    VideoCard::new("stabilization", "Stabilization", section).reset_fields(
        [
            ("/stabilize_video", Value::Bool(false)),
            (
                "/stabilization_method",
                serde_json::to_value(VideoStabilizationMethod::default())
                    .expect("default stabilization method must serialize"),
            ),
            (
                "/stabilization_crop_ratio",
                Value::from(f64::from(
                    shrimply_project::project::default_video_stabilization_crop_ratio(),
                )),
            ),
            (
                "/stabilization_first_derivative_weight",
                Value::from(f64::from(
                    shrimply_project::project::default_video_stabilization_first_derivative_weight(),
                )),
            ),
            (
                "/stabilization_second_derivative_weight",
                Value::from(f64::from(
                    shrimply_project::project::default_video_stabilization_second_derivative_weight(),
                )),
            ),
            (
                "/stabilization_third_derivative_weight",
                Value::from(f64::from(
                    shrimply_project::project::default_video_stabilization_third_derivative_weight(),
                )),
            ),
            (
                "/mesh_flow_rows",
                Value::from(shrimply_project::project::default_mesh_flow_rows()),
            ),
            (
                "/mesh_flow_columns",
                Value::from(shrimply_project::project::default_mesh_flow_columns()),
            ),
            (
                "/mesh_flow_smoothing_radius",
                Value::from(shrimply_project::project::default_mesh_flow_smoothing_radius()),
            ),
            (
                "/mesh_flow_iterations",
                Value::from(shrimply_project::project::default_mesh_flow_iterations()),
            ),
            (
                "/mesh_flow_adaptive_weights",
                serde_json::to_value(
                    shrimply_project::project::MeshFlowAdaptiveWeights::default(),
                )
                .expect("default MeshFlow adaptive weights must serialize"),
            ),
        ],
        "reset-video-stabilization",
    )
    .cancel_stabilization()
}

fn stabilization_weight(
    path: &str,
    label: &str,
    subtitle: &str,
    value: f32,
    commit_name: &'static str,
    unavailable: bool,
) -> InspectorControl {
    value_number(path, label, f64::from(value))
        .subtitle(subtitle)
        .number(NumberSpec {
            minimum: 0.0,
            maximum: 1_000.0,
            drag_step: 1.0,
            digits: 1,
            ..NumberSpec::default()
        })
        .sensitive(!unavailable)
        .immediate_commit(commit_name)
}

fn integer_spec(minimum: f64, maximum: f64) -> NumberSpec {
    NumberSpec {
        minimum,
        maximum,
        digits: 0,
        ..NumberSpec::default()
    }
}

fn stabilization_status(item: &VideoItem) -> &'static str {
    if item.alpha_mask_video.is_some() {
        "Unavailable while an alpha-mask stream is selected"
    } else if !item.stabilize_video {
        ""
    } else if shrimply_video_cuda::video_stabilization::is_generating(item) {
        "Analyzing source motion…"
    } else if shrimply_video_cuda::video_stabilization::has_failed(item) {
        "Analysis failed; use Rebuild to retry"
    } else if shrimply_video_cuda::video_stabilization::is_ready(item) {
        "Using the reusable chunked analysis cache"
    } else {
        "Analysis starts as source-time chunks are viewed"
    }
}

fn enum_text(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("video inspector enum must serialize")
        .as_str()
        .expect("video inspector enum must serialize as text")
        .to_string()
}

fn layered_number(
    value: &Value,
    path: &str,
    label: &str,
    current: f64,
    number: NumberSpec,
    store_multiplier: f64,
) -> InspectorControl {
    InspectorControl::new(ControlKind::LayeredNumber, path, label)
        .value(current.to_string())
        .number(number)
        .store_multiplier(store_multiplier)
        .layered(path, layered_state(value, path))
}

fn layered_boolean(
    value: &Value,
    path: &str,
    label: &str,
    current: bool,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let timeline: TimelineValue<TimelineBool> = serde_json::from_value(
        value
            .pointer(path)
            .cloned()
            .expect("boolean timeline must exist"),
    )
    .expect("boolean timeline must be valid");
    InspectorControl::new(ControlKind::LayeredBoolean, path, label)
        .value(current.to_string())
        .layered(path, layered_state(value, path))
        .graph(bool_graph(&timeline, runtime))
        .live_commit("visual-bool")
        .timeline_commits("visual-bool-keyframes", "visual-bool-expression")
}

fn bool_graph(
    timeline: &TimelineValue<TimelineBool>,
    runtime: InspectorRuntime,
) -> Option<ScalarGraph> {
    if !matches!(timeline.base, TimelineBase::Keyframes(_)) {
        return None;
    }
    let crate::keyframe_graph::KeyframeGraph::Step { points } =
        crate::keyframe_model::step_graph_with(
            timeline,
            |value| {
                if value.get() { 1.0 } else { 0.0 }
            },
        )
    else {
        unreachable!("boolean timeline must produce a step graph")
    };
    Some(ScalarGraph {
        points: points
            .into_iter()
            .map(|point| GraphPoint {
                time: point.time,
                value: point.value,
            })
            .collect(),
        segments: Vec::new(),
        range: runtime.keyframe_range.unwrap_or((Time::ZERO, Time::ZERO)),
        frame_step: runtime.frame_step,
        playhead: runtime.keyframe_playhead.unwrap_or(Time::ZERO),
    })
}

fn layered_state(value: &Value, path: &str) -> LayeredState {
    let timeline = object(value, path);
    let base = timeline
        .get("base")
        .and_then(Value::as_object)
        .expect("video timeline base must be an object");
    let expression = timeline.get("expression").and_then(Value::as_object);
    LayeredState {
        keyframes: base.contains_key("keyframes"),
        expression: expression
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        expression_source: expression
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn value_number(path: &str, label: &str, value: f64) -> InspectorControl {
    InspectorControl::new(ControlKind::Number, path, label).value(value.to_string())
}

fn object<'a>(value: &'a Value, path: &str) -> &'a serde_json::Map<String, Value> {
    value
        .pointer(path)
        .and_then(Value::as_object)
        .expect("video value must be an object")
}

fn text<'a>(value: &'a Value, path: &str) -> &'a str {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .expect("video value must be text")
}
