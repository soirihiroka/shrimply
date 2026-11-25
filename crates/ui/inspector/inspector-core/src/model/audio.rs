use serde_json::Value;
use shrimply_audio_modifiers::{AudioModifier, AudioModifierEffect};
use shrimply_editor_state::player_state;
use shrimply_project_document::project::{ItemAddress, Time};
use shrimply_property_model::modifier_model::ModifierModel;
use shrimply_property_model::timeline_value::TimelineValue;

use super::{
    AudioCacheStatus, AudioModifierKeyframeMove, EditKind, INSPECTOR_EDIT_COMMIT, InspectorCommit,
    InspectorController, InspectorExpressionOutput, TimelineModeChange, deserialize,
    target_runtime, target_value,
};
use crate::audio_cache::audio_cache_status;
use crate::audio_modifiers::{
    audio_item_address, audio_modifier_evaluation_time, audio_modifier_key,
    audio_modifier_keyframe_time, audio_modifier_number, audio_modifier_number_mut,
    audio_modifier_time,
};
use crate::target::InspectorTarget;

impl InspectorController {
    pub fn audio_beat_detection_loading(&self, target: &InspectorTarget) -> bool {
        let Ok(address) = audio_item_address(target) else {
            return false;
        };
        self.project
            .borrow()
            .audio_item(address)
            .is_some_and(|item| {
                item.beat_detection && shrimply_audio_engine::beat::is_loading(item.id)
            })
    }

    pub fn set_audio_modifier_enabled(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let address = audio_item_address(target)?;
        let modifier = project
            .audio_item_mut(address)
            .and_then(|item| item.modifiers.iter_mut().find(|modifier| modifier.id == id))
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        if modifier.enabled == enabled {
            return Ok(());
        }
        modifier.enabled = enabled;
        shrimply_project_document::project::commit_edit(&project, "toggle-audio-modifier");
        drop(project);
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn set_audio_modifier_field(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        path: &str,
        text: &str,
    ) -> Result<(), String> {
        let path = self.audio_modifier_path(target, id, path)?;
        self.set_field_with_kind(target, &path, text, EditKind::AudioModifierStructural)
    }

    pub fn set_audio_modifier_live_field(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        path: &str,
        text: &str,
    ) -> Result<(), String> {
        let path = self.audio_modifier_path(target, id, path)?;
        self.set_field_with_kind(target, &path, text, EditKind::AudioModifierLive)
    }

    pub fn set_audio_modifier_timeline_base(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        next: f32,
    ) -> Result<(), String> {
        self.set_audio_modifier_timeline_base_with_commit(
            target,
            modifier_id,
            value_id,
            next,
            InspectorCommit::Coalesced("audio-modifier-value"),
        )
    }

    pub(crate) fn set_audio_modifier_timeline_base_with_commit(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        next: f32,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let time = audio_modifier_time(&project, &self.player_state, target)?;
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        if !crate::keyframe_model::set_scalar_value(value, time, next) {
            return Ok(());
        }
        match commit {
            InspectorCommit::Deferred => {}
            InspectorCommit::Coalesced(name) => {
                shrimply_project_document::project::commit_coalesced_edit(&project, name);
            }
            InspectorCommit::Immediate(name) => {
                shrimply_project_document::project::commit_edit(&project, name);
            }
        }
        drop(project);
        self.refresh_audio_modifier(false, false, true);
        Ok(())
    }

    pub fn timeline_number_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: Option<uuid::Uuid>,
    ) -> Result<f64, String> {
        let project = self.project.borrow();
        if path.starts_with("/content/generator/")
            && let (Some(timeline_id), InspectorTarget::Item(address @ ItemAddress::Video { .. })) =
                (timeline_id, target)
        {
            let item = project
                .video_item(address)
                .ok_or_else(|| "video item is no longer available".to_string())?;
            let local_time = target_runtime(&project, &self.player_state, target)
                .local_time
                .ok_or_else(|| "the current item time is not available".to_string())?;
            if let Some(timeline) = crate::background::number_value(item, timeline_id) {
                return Ok(f64::from(timeline.value_at(local_time)));
            }
            if let Some(timeline) = crate::background::integer_value(item, timeline_id) {
                return Ok(f64::from(timeline.value_at(local_time)));
            }
            return Err(format!("background number is no longer available: {path}"));
        }
        if path.starts_with("/modifiers/")
            && let (Some(timeline_id), InspectorTarget::Item(address @ ItemAddress::Video { .. })) =
                (timeline_id, target)
        {
            let item = project
                .video_item(address)
                .ok_or_else(|| "video item is no longer available".to_string())?;
            let timeline = crate::visual_modifiers::visual_modifier_number(item, path, timeline_id)
                .ok_or_else(|| format!("visual modifier number is no longer available: {path}"))?;
            let local_time = target_runtime(&project, &self.player_state, target)
                .local_time
                .ok_or_else(|| "the current item time is not available".to_string())?;
            return Ok(f64::from(timeline.value_at(local_time)));
        }
        let value = target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?
            .1;
        let timeline: shrimply_property_model::timeline_value::TimelineValue<f32> =
            deserialize(value.pointer(path).cloned().ok_or_else(|| {
                format!("inspector timeline value is no longer available: {path}")
            })?)?;
        if timeline_id.is_some_and(|timeline_id| timeline.id != timeline_id) {
            return Err(format!(
                "inspector timeline value is no longer available: {path}"
            ));
        }
        let local_time = match target {
            InspectorTarget::Item(ItemAddress::Audio { .. }) => {
                audio_modifier_evaluation_time(&project, &self.player_state, target)?
            }
            InspectorTarget::Item(address) => {
                let position = crate::model::player_snapshot(&self.player_state).position;
                project
                    .keyframe_time(address, position)
                    .ok_or_else(|| "the current item time is not available".to_string())?
            }
            _ => Time::ZERO,
        };
        Ok(f64::from(timeline.value_at(local_time)))
    }

    pub fn ensure_visual_modifier_number(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier number target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        crate::visual_modifiers::visual_modifier_number(item, path, timeline_id)
            .map(|_| ())
            .ok_or_else(|| format!("visual modifier number is no longer available: {path}"))
    }

    pub fn ensure_visual_modifier_text(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier text target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        crate::visual_modifiers::visual_modifier_text(item, path, timeline_id)
            .map(|_| ())
            .ok_or_else(|| format!("visual modifier text is no longer available: {path}"))
    }

    pub fn ensure_visual_modifier(
        &self,
        target: &InspectorTarget,
        path: &str,
        modifier_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        crate::visual_modifiers::visual_modifier_matches(item, path, modifier_id)
            .then_some(())
            .ok_or_else(|| "visual modifier is no longer available".to_string())
    }

    pub fn ensure_visual_modifier_vector2(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier vector target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        crate::visual_modifiers::visual_modifier_vector2(item, path, timeline_id)
            .map(|_| ())
            .ok_or_else(|| format!("visual modifier vector is no longer available: {path}"))
    }

    pub fn ensure_visual_modifier_vector3(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier vector target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        crate::visual_modifiers::visual_modifier_vector3(item, path, timeline_id)
            .map(|_| ())
            .ok_or_else(|| format!("visual modifier vector is no longer available: {path}"))
    }

    pub fn ensure_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let value = target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?
            .1;
        let current = value
            .pointer(path)
            .and_then(|timeline| timeline.get("id"))
            .and_then(Value::as_str)
            .and_then(|id| uuid::Uuid::parse_str(id).ok());
        (current == Some(timeline_id))
            .then_some(())
            .ok_or_else(|| format!("inspector timeline is no longer available: {path}"))
    }

    pub fn timeline_vector2_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: Option<uuid::Uuid>,
    ) -> Result<glam::Vec2, String> {
        let project = self.project.borrow();
        if path.starts_with("/content/generator/")
            && let (Some(timeline_id), InspectorTarget::Item(address @ ItemAddress::Video { .. })) =
                (timeline_id, target)
        {
            let item = project
                .video_item(address)
                .ok_or_else(|| "video item is no longer available".to_string())?;
            let timeline = crate::background::vector_value(item, timeline_id)
                .ok_or_else(|| format!("background vector is no longer available: {path}"))?;
            let local_time = target_runtime(&project, &self.player_state, target)
                .local_time
                .ok_or_else(|| "the current item time is not available".to_string())?;
            return Ok(crate::timeline_value::vector::vec2::value_at(
                timeline, local_time,
            ));
        }
        if path.starts_with("/modifiers/")
            && let (Some(timeline_id), InspectorTarget::Item(address @ ItemAddress::Video { .. })) =
                (timeline_id, target)
        {
            let item = project
                .video_item(address)
                .ok_or_else(|| "video item is no longer available".to_string())?;
            let timeline =
                crate::visual_modifiers::visual_modifier_vector2(item, path, timeline_id)
                    .ok_or_else(|| {
                        format!("visual modifier vector is no longer available: {path}")
                    })?;
            let local_time = target_runtime(&project, &self.player_state, target)
                .local_time
                .ok_or_else(|| "the current item time is not available".to_string())?;
            return Ok(crate::timeline_value::vector::vec2::value_at(
                timeline, local_time,
            ));
        }
        let value = target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?
            .1;
        let timeline: shrimply_property_model::timeline_value::TimelineValue<glam::Vec2> =
            deserialize(value.pointer(path).cloned().ok_or_else(|| {
                format!("inspector timeline vector is no longer available: {path}")
            })?)?;
        if timeline_id.is_some_and(|timeline_id| timeline.id != timeline_id) {
            return Err(format!(
                "inspector timeline vector is no longer available: {path}"
            ));
        }
        let local_time = target_runtime(&project, &self.player_state, target)
            .local_time
            .unwrap_or(Time::ZERO);
        Ok(crate::timeline_value::vector::vec2::value_at(
            &timeline, local_time,
        ))
    }

    pub fn timeline_vector3_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<glam::Vec3, String> {
        let project = self.project.borrow();
        let value = target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?
            .1;
        let timeline: shrimply_property_model::timeline_value::TimelineValue<glam::Vec3> =
            deserialize(value.pointer(path).cloned().ok_or_else(|| {
                format!("inspector timeline vector is no longer available: {path}")
            })?)?;
        if timeline.id != timeline_id {
            return Err(format!(
                "inspector timeline vector is no longer available: {path}"
            ));
        }
        let local_time = target_runtime(&project, &self.player_state, target)
            .local_time
            .ok_or_else(|| "the current item time is not available".to_string())?;
        Ok(crate::timeline_value::vector::vec3::value_at(
            &timeline, local_time,
        ))
    }

    pub fn visual_modifier_number_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::visual_modifier_number(item, path, timeline_id)
            .ok_or_else(|| format!("visual modifier number is no longer available: {path}"))?;
        let runtime = target_runtime(&project, &self.player_state, target);
        let current = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
        Ok(crate::transform::scalar_graph(timeline, current, runtime))
    }

    pub fn visual_modifier_text_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::visual_modifier_text(item, path, timeline_id)
            .ok_or_else(|| format!("visual modifier text is no longer available: {path}"))?;
        Ok(crate::timeline_text::speed_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub(crate) fn generated_text_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("generated text graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::generated::text_value(item, path, timeline_id)
            .ok_or_else(|| format!("generated text timeline is no longer available: {path}"))?;
        Ok(crate::timeline_text::speed_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn visual_modifier_vector2_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::visual_modifier_vector2(item, path, timeline_id)
            .ok_or_else(|| format!("visual modifier vector is no longer available: {path}"))?;
        Ok(crate::transform::vector_speed_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn visual_modifier_vector3_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::visual_modifier_vector3(item, path, timeline_id)
            .ok_or_else(|| format!("visual modifier vector is no longer available: {path}"))?;
        Ok(crate::visual_modifiers::vector3_speed_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn visual_modifier_color_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::visual_modifier_color(item, path, timeline_id)
            .ok_or_else(|| format!("visual modifier color is no longer available: {path}"))?;
        Ok(crate::timeline_color::speed_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn erode_dilate_operation_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::erode_dilate_operation(item, path, timeline_id)
            .ok_or_else(|| format!("erode/dilate operation is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn halftone_mode_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::halftone_mode(item, path, timeline_id)
            .ok_or_else(|| format!("halftone mode is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn mask_mode_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        modifier_id: uuid::Uuid,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !crate::visual_modifiers::visual_modifier_matches(item, path, modifier_id) {
            return Err("mask modifier is no longer available".to_string());
        }
        let timeline = crate::visual_modifiers::mask_mode(item, path, timeline_id)
            .ok_or_else(|| format!("mask mode is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn kuwahara_version_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::kuwahara_version_timeline(item, path, timeline_id)
            .ok_or_else(|| format!("Kuwahara version is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn rasterize_sample_method_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline =
            crate::visual_modifiers::rasterize_sample_method_timeline(item, path, timeline_id)
                .ok_or_else(|| format!("Rasterize sample method is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn sampling_method_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        modifier_id: uuid::Uuid,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !crate::visual_modifiers::visual_modifier_matches(item, path, modifier_id) {
            return Err("Sampling modifier is no longer available".to_string());
        }
        let timeline = crate::visual_modifiers::sampling_method_timeline(item, path, timeline_id)
            .ok_or_else(|| format!("Sampling method is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn texture_bounds_address_mode_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        modifier_id: uuid::Uuid,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !crate::visual_modifiers::visual_modifier_matches(item, path, modifier_id) {
            return Err("Texture bounds modifier is no longer available".to_string());
        }
        let timeline =
            crate::visual_modifiers::texture_bounds_address_mode_timeline(item, path, timeline_id)
                .ok_or_else(|| format!("Texture addressing is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn repeat_offset_axis_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        modifier_id: uuid::Uuid,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if !crate::visual_modifiers::visual_modifier_matches(item, path, modifier_id) {
            return Err("Repeat modifier is no longer available".to_string());
        }
        let timeline =
            crate::visual_modifiers::repeat_offset_axis_timeline(item, path, timeline_id)
                .ok_or_else(|| format!("Repeat offset axis is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn dithering_pattern_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::dithering_pattern(item, path, timeline_id)
            .ok_or_else(|| format!("dithering pattern is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn dithering_color_mode_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let project = self.project.borrow();
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("visual modifier graph target is not a video item".to_string());
        };
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let timeline = crate::visual_modifiers::dithering_color_mode(item, path, timeline_id)
            .ok_or_else(|| format!("dithering color mode is no longer available: {path}"))?;
        Ok(crate::selector::step_graph(
            timeline,
            target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn audio_modifier_number_value(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        value_id: uuid::Uuid,
    ) -> Result<f64, String> {
        let project = self.project.borrow();
        let local_time = audio_modifier_evaluation_time(&project, &self.player_state, target)?;
        let value = audio_modifier_number(&project, target, id, value_id)?;
        Ok(f64::from(value.value_at(local_time)))
    }

    pub fn audio_modifier_timeline(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        value_id: uuid::Uuid,
    ) -> Result<TimelineValue<f32>, String> {
        let project = self.project.borrow();
        audio_modifier_number(&project, target, id, value_id).cloned()
    }

    pub fn audio_modifier_expression_output(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
    ) -> Result<InspectorExpressionOutput, String> {
        let project = self.project.borrow();
        let local_time = audio_modifier_evaluation_time(&project, &self.player_state, target)?;
        let item = project
            .audio_item(audio_item_address(target)?)
            .ok_or_else(|| "audio modifier item is no longer available".to_string())?;
        let value = item
            .modifiers
            .iter()
            .find(|modifier| modifier.id == modifier_id)
            .and_then(|modifier| modifier.effect.number(value_id))
            .ok_or_else(|| "audio modifier number is no longer available".to_string())?;
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_audio_item_local_time(
            &project, item, local_time,
        );
        let outcome = shrimply_project_evaluation::resolve_with_error(
            value,
            &evaluation,
            &mut self.expression_cache.borrow_mut(),
        );
        Ok(InspectorExpressionOutput {
            value: outcome.value,
            error: outcome.error,
        })
    }

    pub fn set_audio_modifier_timeline_mode(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        path: &str,
        change: TimelineModeChange<'_>,
    ) -> Result<(), String> {
        if change.keyframes {
            return self.set_audio_modifier_keyframes_enabled(
                target,
                modifier_id,
                value_id,
                change.enabled,
            );
        }
        self.set_audio_modifier_expression_enabled(
            target,
            modifier_id,
            path,
            change.enabled,
            change.default_expression,
        )
    }

    pub fn set_audio_modifier_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let evaluation_time = audio_modifier_evaluation_time(&project, &self.player_state, target)?;
        let keyframe_time = audio_modifier_time(&project, &self.player_state, target)?;
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        let current = value.value_at(evaluation_time);
        if !crate::keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "audio-modifier-value");
        drop(project);
        self.refresh_audio_modifier(true, true, false);
        Ok(())
    }

    pub fn set_audio_modifier_expression_enabled(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        path: &str,
        enabled: bool,
        default_expression: &str,
    ) -> Result<(), String> {
        self.set_timeline_mode_with_kind(
            target,
            &self.audio_modifier_path(target, modifier_id, path)?,
            TimelineModeChange {
                keyframes: false,
                enabled,
                current: Value::Null,
                default_expression,
            },
            EditKind::AudioModifierStructural,
            InspectorCommit::Immediate(INSPECTOR_EDIT_COMMIT),
        )
    }

    pub fn set_audio_modifier_expression_source(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        path: &str,
        source: &str,
    ) -> Result<(), String> {
        self.set_expression_source_with_kind(
            target,
            &self.audio_modifier_path(target, id, path)?,
            source,
            EditKind::AudioModifierLive,
            InspectorCommit::Coalesced(INSPECTOR_EDIT_COMMIT),
        )
    }

    pub fn move_audio_modifier_keyframe(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        change: AudioModifierKeyframeMove,
    ) -> Result<(), String> {
        self.move_audio_modifier_keyframes(
            target,
            modifier_id,
            value_id,
            std::slice::from_ref(&change),
        )
        .map(|_| ())
    }

    pub fn move_audio_modifier_keyframes(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        changes: &[AudioModifierKeyframeMove],
    ) -> Result<Vec<Time>, String> {
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let mut project = self.project.borrow_mut();
        let changes = changes
            .iter()
            .map(|change| {
                Ok((
                    change.old_time,
                    audio_modifier_keyframe_time(&project, target, change.time)?,
                    (change.displayed_value * change.store_multiplier) as f32,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        if !crate::timeline_value::scalar::move_stored_keyframes(
            value,
            &changes,
            crate::NumberConstraint::default(),
        ) {
            return Err("audio keyframe move contains invalid values or stale source keys".into());
        }
        shrimply_project_document::project::commit_coalesced_edit(
            &project,
            "audio-modifier-keyframe",
        );
        drop(project);
        self.refresh_audio_modifier_graph();
        Ok(changes.iter().map(|change| change.1).collect())
    }

    pub fn delete_audio_modifier_keyframe(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        time: Time,
    ) -> Result<(), String> {
        self.delete_audio_modifier_keyframes(
            target,
            modifier_id,
            value_id,
            std::slice::from_ref(&time),
        )
    }

    pub fn delete_audio_modifier_keyframes(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        times: &[Time],
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        if !crate::keyframe_model::edit_keyframe_selection(value, times, |value, time| {
            Ok(crate::keyframe_model::delete_scalar_keyframe(value, time))
        })? {
            return Ok(());
        }
        let keyframes_disabled = !matches!(
            value.base,
            shrimply_property_model::timeline_value::TimelineBase::Keyframes(_)
        );
        shrimply_project_document::project::commit_edit(&project, "audio-modifier-keyframe");
        drop(project);
        self.refresh_audio_modifier(keyframes_disabled, true, true);
        Ok(())
    }

    pub fn add_audio_modifier_keyframe(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        time: Time,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let time = audio_modifier_keyframe_time(&project, target, time)?;
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        if !shrimply_property_model::timeline_value::add_curve_keyframe(
            value,
            time,
            std::convert::identity,
        ) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "audio-modifier-keyframe");
        drop(project);
        self.refresh_audio_modifier_graph();
        Ok(())
    }

    pub fn set_audio_modifier_keyframe_interpolation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let mut project = self.project.borrow_mut();
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        if !crate::keyframe_model::set_scalar_interpolation(value, owner_id, interpolation) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "audio-modifier-keyframe");
        drop(project);
        self.refresh_audio_modifier_graph();
        Ok(())
    }

    pub fn seek_audio_modifier_keyframe(
        &self,
        target: &InspectorTarget,
        time: Time,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let position = project
            .keyframe_timeline_time(audio_item_address(target)?, time)
            .ok_or_else(|| "audio modifier keyframe time is no longer available".to_string())?;
        drop(project);
        player_state::seek_time(&self.player_state, position);
        Ok(())
    }

    pub fn toggle_keyframe_playback(&self) {
        player_state::toggle_playing(&self.player_state);
    }

    pub fn copy_audio_modifier_keyframes(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        selected: &[Time],
    ) -> Result<usize, String> {
        let project = self.project.borrow();
        let value = audio_modifier_number(&project, target, modifier_id, value_id)?;
        let Some(mut clipboard) = crate::keyframe_model::copy_keyframes(value, selected) else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let address = audio_item_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_audio_modifier_keyframes(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        value_id: uuid::Uuid,
        time: Time,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let mut project = self.project.borrow_mut();
        let address = audio_item_address(target)?;
        let Some(times) =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time)
        else {
            return Err("audio modifier keyframes cannot be pasted at this time".to_string());
        };
        let value = audio_modifier_number_mut(&mut project, target, modifier_id, value_id)?;
        let Some(pasted) = crate::keyframe_model::paste_keyframes(value, &clipboard, &times) else {
            return Ok(0);
        };
        shrimply_project_document::project::commit_edit(&project, "audio-modifier-keyframe");
        drop(project);
        self.refresh_audio_modifier(true, true, true);
        Ok(pasted.len())
    }

    fn audio_modifier_path(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        relative_path: &str,
    ) -> Result<String, String> {
        if !relative_path.starts_with('/') {
            return Err("audio modifier path must be relative to the modifier".to_string());
        }
        let project = self.project.borrow();
        let index = project
            .audio_item(audio_item_address(target)?)
            .and_then(|item| item.modifiers.iter().position(|modifier| modifier.id == id))
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        Ok(format!("/modifiers/{index}{relative_path}"))
    }

    pub fn reset_audio_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        effect: Value,
    ) -> Result<(), String> {
        let effect: AudioModifierEffect = deserialize(effect)?;
        self.reset_audio_modifier_effect(target, id, effect)
    }

    pub fn reset_audio_modifier_effect(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        effect: AudioModifierEffect,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let modifier = project
            .audio_item_mut(audio_item_address(target)?)
            .and_then(|item| item.modifiers.iter_mut().find(|modifier| modifier.id == id))
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        if crate::audio_modifiers::same_audio_modifier_effect(&modifier.effect, &effect) {
            return Ok(());
        }
        modifier.effect = effect;
        shrimply_project_document::project::commit_edit(&project, "reset-audio-modifier");
        drop(project);
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn copy_audio_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> Result<String, String> {
        let project = self.project.borrow();
        let modifier = project
            .audio_item(audio_item_address(target)?)
            .and_then(|item| item.modifiers.iter().find(|modifier| modifier.id == id))
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        let name = modifier.effect.display_name().to_string();
        clipboard.borrow_mut().copy_audio_modifier(modifier);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(name)
    }

    pub fn move_audio_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        offset: isize,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .audio_item_mut(audio_item_address(target)?)
            .ok_or_else(|| "audio item is no longer available".to_string())?;
        let index = item
            .modifiers
            .iter()
            .position(|modifier| modifier.id == id)
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        let destination = index
            .checked_add_signed(offset)
            .filter(|destination| *destination < item.modifiers.len())
            .ok_or_else(|| "audio modifier cannot be moved outside the chain".to_string())?;
        item.modifiers.swap(index, destination);
        shrimply_project_document::project::commit_edit(&project, "edit-audio-modifier-chain");
        drop(project);
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn remove_audio_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .audio_item_mut(audio_item_address(target)?)
            .ok_or_else(|| "audio item is no longer available".to_string())?;
        let index = item
            .modifiers
            .iter()
            .position(|modifier| modifier.id == id)
            .ok_or_else(|| "audio modifier is no longer available".to_string())?;
        if matches!(item.modifiers[index].effect, AudioModifierEffect::Cache(_)) {
            shrimply_audio_engine::modifier_cache::invalidate(id)?;
        }
        item.modifiers.remove(index);
        shrimply_project_document::project::commit_edit(&project, "edit-audio-modifier-chain");
        drop(project);
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn add_audio_modifier(&self, target: &InspectorTarget, kind: &str) -> Result<(), String> {
        let effect = AudioModifierEffect::CATALOG
            .iter()
            .map(|new| new())
            .find(|effect| audio_modifier_key(effect) == kind)
            .ok_or_else(|| format!("unknown audio modifier: {kind}"))?;
        let mut project = self.project.borrow_mut();
        project
            .audio_item_mut(audio_item_address(target)?)
            .ok_or_else(|| "audio item is no longer available".to_string())?
            .modifiers
            .push(AudioModifier::new(effect));
        shrimply_project_document::project::commit_edit(&project, "add-audio-modifier");
        drop(project);
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn can_paste_audio_modifiers(
        &self,
        target: &InspectorTarget,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> bool {
        let Ok(address) = audio_item_address(target) else {
            return false;
        };
        clipboard
            .borrow()
            .can_append_modifiers(&self.project.borrow(), std::slice::from_ref(address))
    }

    pub fn paste_audio_modifiers(
        &self,
        target: &InspectorTarget,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> Result<usize, String> {
        let address = audio_item_address(target)?.clone();
        let mut project = self.project.borrow_mut();
        let result = clipboard
            .borrow()
            .append_modifiers(&mut project, std::slice::from_ref(&address));
        if !result.changed {
            return Ok(0);
        }
        shrimply_project_document::project::commit_edit(&project, "paste-item-modifiers");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                audio: result.audio,
                audio_waveforms: result.audio_waveforms,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(result.modifiers_added)
    }

    pub fn set_audio_cache_preset(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        preset: &str,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let cache = project
            .audio_item_mut(audio_item_address(target)?)
            .and_then(|item| item.modifiers.iter_mut().find(|modifier| modifier.id == id))
            .and_then(|modifier| match &mut modifier.effect {
                AudioModifierEffect::Cache(cache) => Some(cache),
                _ => None,
            })
            .ok_or_else(|| "audio cache modifier is no longer available".to_string())?;
        let preset = crate::AudioCachePreset::from_key(preset)
            .ok_or_else(|| format!("unknown audio cache preset: {preset}"))?;
        if !preset.apply(cache) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, "audio-cache-format");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn toggle_audio_cache(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
    ) -> Result<(), String> {
        if matches!(audio_cache_status(id), AudioCacheStatus::Baking { .. }) {
            shrimply_audio_engine::modifier_cache::invalidate(id)?;
        } else {
            shrimply_audio_engine::modifier_cache::bake(
                self.project.borrow().clone(),
                audio_item_address(target)?.clone(),
                id,
            )?;
        }
        self.audio_cache_transitions.borrow_mut().observe(
            target.clone(),
            id,
            audio_cache_status(id),
        );
        self.refresh_audio_inspector();
        Ok(())
    }

    pub fn refresh_audio_cache(&self) {
        self.refresh_audio_inspector();
    }

    fn refresh_audio_inspector(&self) {
        self.refresh_audio_modifier(true, true, false);
    }

    fn refresh_audio_modifier_graph(&self) {
        self.refresh_audio_modifier(false, true, true);
    }

    fn refresh_audio_modifier(&self, inspector: bool, audio_waveforms: bool, live_preview: bool) {
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                audio: true,
                audio_waveforms,
                inspector,
                live_preview,
                ..Default::default()
            },
        );
    }
}
