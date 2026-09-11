pub mod graph;
mod layered;

use serde_json::Value;
use shrimply_project_document::project::ItemAddress;

use crate::{ControlKind, InspectorCommit, InspectorControl, InspectorTarget};

#[derive(Clone, Debug, PartialEq)]
pub enum BasicInspectorAction {
    Reset {
        path: String,
        value: Value,
    },
    ResetFields {
        values: Vec<(String, Value)>,
    },
    ResetAudioOutput,
    ResetAudioGenerator,
    ResetVideo(crate::VideoReset),
    ResetManim(crate::manim_parameters::ManimReset),
    ResetManimParameters(crate::manim_parameters::ManimParametersReset),
    SetBoolean {
        path: String,
        value: bool,
    },
    SetModifierEnabled {
        id: uuid::Uuid,
        enabled: bool,
        audio: bool,
    },
    ResetModifier {
        id: uuid::Uuid,
        audio: bool,
    },
    MoveModifier {
        id: uuid::Uuid,
        offset: isize,
        audio: bool,
    },
    RemoveModifier {
        id: uuid::Uuid,
        audio: bool,
    },
    SetAlphaMask {
        target: shrimply_project_document::project::VisualAlphaMaskTarget,
        enabled: bool,
    },
    Video(crate::video::VideoCardAction),
    CopyModifier {
        id: uuid::Uuid,
        audio: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InspectorToggleAction {
    AlphaMask(shrimply_project_document::project::VisualAlphaMaskTarget),
}

impl InspectorToggleAction {
    pub fn action(&self, enabled: bool) -> BasicInspectorAction {
        match self {
            Self::AlphaMask(target) => BasicInspectorAction::SetAlphaMask {
                target: *target,
                enabled,
            },
        }
    }
}

impl crate::InspectorController {
    pub fn poll_document_dependencies(&self) -> bool {
        let blender = crate::video::blender::poll_metadata();
        let manim_scenes = crate::manim_parameters::poll_scenes();
        let media = self.media_metadata.borrow_mut().poll();
        let voices = self.voice_models.borrow_mut().poll();
        let cameras = self.camera_models.borrow_mut().poll();
        let analysis = self.poll_camera_analysis();
        let sam2 = self.poll_sam2_analysis();
        let tts = self.poll_tts();
        let fill = self.poll_transparent_fill_analysis();
        let caches = self.poll_visual_caches();
        blender
            || manim_scenes
            || media
            || voices
            || cameras
            || analysis
            || sam2
            || tts
            || fill
            || caches
    }

    pub fn document_snapshot(&self, server_url: &str) -> crate::InspectorSnapshot {
        let camera_models = crate::camera_source::cached_tracking_models(server_url);
        let mut snapshot = self.snapshot_with_camera_models(camera_models.as_ref());
        if let Some(source) = snapshot
            .video
            .as_ref()
            .and_then(|video| video.blender.as_ref())
        {
            let asset = shrimply_project_document::project::Asset::from(std::path::Path::new(
                &source.asset,
            ));
            let metadata =
                crate::video::blender::metadata(&asset, shrimply_blender_core::binary().as_deref());
            if let crate::video::blender::MetadataState::Ready(metadata) = &metadata
                && self
                    .sync_blender_metadata(&snapshot.target, metadata)
                    .unwrap_or_else(|error| {
                        panic!("could not synchronize Blender metadata: {error}")
                    })
            {
                snapshot = self.snapshot_with_camera_models(camera_models.as_ref());
            }
            let video = snapshot.video.as_mut().expect("Blender video is selected");
            let source = video.blender.as_ref().expect("Blender source is selected");
            video.visual.insert(
                0,
                crate::video::blender::card(&source.item, &source.asset, &metadata),
            );
        }
        if snapshot.video.as_ref().is_some_and(|video| {
            video.visual.iter().any(|card| {
                card.section
                    .controls
                    .iter()
                    .any(|control| control.path == crate::camera_source::MODEL_PATH)
            })
        }) {
            self.camera_models.borrow_mut().request(
                server_url,
                camera_models,
                crate::camera_source::tracking_models,
            );
        }
        snapshot
    }

    pub fn document_metadata(
        &self,
        media: Option<&crate::InspectorMedia>,
        format_date: fn(i64) -> Option<String>,
    ) -> Option<crate::info::metadata::MetadataState> {
        self.media_metadata.borrow_mut().request(media, format_date)
    }

    pub fn populate_voice_model_control(
        &self,
        server_url: &str,
        control: &mut crate::InspectorControl,
    ) {
        self.voice_models.borrow_mut().populate(server_url, control);
    }

    pub fn property_clipboard(&self) -> shrimply_property_transfer::SharedClipboard {
        self.property_clipboard.clone()
    }

    pub fn add_control_modifier(
        &self,
        target: &InspectorTarget,
        kind: ControlKind,
        value: &str,
    ) -> Result<(), String> {
        match (kind, value) {
            (ControlKind::AudioModifierMenu, "__paste__") => self
                .paste_audio_modifiers(target, &self.property_clipboard)
                .map(|_| ()),
            (ControlKind::VisualModifierMenu, "__paste__") => self
                .paste_visual_modifiers(target, &self.property_clipboard)
                .map(|_| ()),
            (ControlKind::AudioModifierMenu, value) => self.add_audio_modifier(target, value),
            (ControlKind::VisualModifierMenu, value) => {
                self.add_visual_modifier(target, value).map(|_| ())
            }
            _ => Err("control is not a modifier menu".into()),
        }
    }

    pub fn apply_basic_action(
        &self,
        target: &InspectorTarget,
        action: &BasicInspectorAction,
    ) -> Result<(), String> {
        match action {
            BasicInspectorAction::CopyModifier { id, audio } => {
                if *audio {
                    self.copy_audio_modifier(target, *id, &self.property_clipboard)
                        .map(|_| ())
                } else {
                    self.copy_visual_modifier(target, *id, &self.property_clipboard)
                        .map(|_| ())
                }
            }
            BasicInspectorAction::SetModifierEnabled { id, enabled, audio } => {
                if *audio {
                    self.set_audio_modifier_enabled(target, *id, *enabled)
                } else {
                    self.set_visual_modifier_enabled(target, *id, *enabled)
                }
            }
            BasicInspectorAction::ResetModifier { id, audio } => {
                let InspectorTarget::Item(address) = target else {
                    return Err("modifier reset requires an item".into());
                };
                let project = self.project.borrow();
                if *audio {
                    let effect = project
                        .audio_item(address)
                        .and_then(|item| item.modifiers.iter().find(|modifier| modifier.id == *id))
                        .map(|modifier| crate::default_audio_modifier_effect(&modifier.effect))
                        .ok_or("audio modifier is no longer available")?;
                    drop(project);
                    self.reset_audio_modifier_effect(target, *id, effect)
                } else {
                    let effect = project
                        .video_item(address)
                        .and_then(|item| item.modifiers.iter().find(|modifier| modifier.id == *id))
                        .map(|modifier| crate::default_visual_modifier_effect(&modifier.effect))
                        .ok_or("visual modifier is no longer available")?;
                    drop(project);
                    self.reset_visual_modifier_effect(target, *id, effect)
                }
            }
            BasicInspectorAction::MoveModifier { id, offset, audio } => {
                if *audio {
                    self.move_audio_modifier(target, *id, *offset)
                } else {
                    self.move_visual_modifier(target, *id, *offset)
                }
            }
            BasicInspectorAction::RemoveModifier { id, audio } => {
                if *audio {
                    self.remove_audio_modifier(target, *id)
                } else {
                    self.remove_visual_modifier(target, *id)
                }
            }
            BasicInspectorAction::SetAlphaMask {
                target: mask,
                enabled,
            } => self.set_alpha_mask_enabled(target, *mask, *enabled),
            BasicInspectorAction::Video(crate::video::VideoCardAction::ReloadAsset {
                asset,
                kind,
            }) => crate::video::reload_asset(asset, *kind),
            BasicInspectorAction::Reset { path, value } => {
                self.set_value(target, path, value.clone())
            }
            BasicInspectorAction::ResetFields { values } => self.set_values(target, values),
            BasicInspectorAction::ResetAudioOutput => self.set_values(
                target,
                &[
                    ("/enabled".into(), Value::Bool(true)),
                    (
                        "/gain".into(),
                        serde_json::to_value(shrimply_audio_modifiers::GainModifier::default())
                            .expect("default audio gain must serialize"),
                    ),
                ],
            ),
            BasicInspectorAction::ResetAudioGenerator => {
                let defaults = serde_json::to_value(
                    shrimply_project_document::project::AudioGenerator::default(),
                )
                .expect("default audio generator must serialize");
                self.set_values(
                    target,
                    &defaults
                        .as_object()
                        .expect("generator must be an object")
                        .iter()
                        .map(|(field, value)| (format!("/source/{field}"), value.clone()))
                        .collect::<Vec<_>>(),
                )
            }
            BasicInspectorAction::ResetVideo(reset) => self.reset_video(target, reset),
            BasicInspectorAction::ResetManim(reset) => self.reset_manim(target, reset),
            BasicInspectorAction::ResetManimParameters(reset) => {
                self.reset_manim_parameters(target, reset)
            }
            BasicInspectorAction::SetBoolean { path, value } => {
                self.set_value(target, path, Value::Bool(*value))
            }
        }
    }

    pub fn set_basic_control_value(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        value: &str,
    ) -> Result<(), String> {
        match control.action {
            Some(crate::InspectorControlAction::SetSam2Model { modifier_id }) => {
                let model = serde_json::from_value(Value::String(value.to_string()))
                    .map_err(|error| format!("invalid SAM2 model: {error}"))?;
                return self.set_sam2_model(target, modifier_id, model);
            }
            Some(crate::InspectorControlAction::SetSam2PointLabel {
                modifier_id,
                point_id,
            }) => {
                let label = serde_json::from_value(Value::String(value.to_string()))
                    .map_err(|error| format!("invalid SAM2 point type: {error}"))?;
                return self.set_sam2_point_label(target, modifier_id, point_id, label);
            }
            _ => {}
        }
        if let Some(id) = control.target_id {
            match control.kind {
                ControlKind::AudioCachePreset => {
                    return self.set_audio_cache_preset(target, id, value);
                }
                ControlKind::VisualCacheQuality => {
                    return self.set_visual_cache_quality(target, id, value);
                }
                _ => {}
            }
        }
        if control.audio_modifier {
            let id = control
                .target_id
                .ok_or("audio modifier control has no owner")?;
            return if control.kind == ControlKind::LayeredNumber {
                let timeline = control
                    .timeline_id
                    .ok_or("audio modifier control has no timeline")?;
                let number = value
                    .parse::<f64>()
                    .map_err(|_| "invalid modifier number")?;
                self.set_audio_modifier_timeline_base_with_commit(
                    target,
                    id,
                    timeline,
                    control.store_number(number) as f32,
                    control_commit(control),
                )
            } else if control.kind == ControlKind::Number {
                self.set_audio_modifier_live_field(target, id, &control.path, value)
            } else {
                self.set_audio_modifier_field(target, id, &control.path, value)
            };
        }
        match control.kind {
            ControlKind::OptionalSelector => {
                self.set_optional_field(target, &control.path, (!value.is_empty()).then_some(value))
            }
            ControlKind::OptionalNumberSelector => self.set_optional_number_field(
                target,
                &control.path,
                (!value.is_empty()).then_some(value),
            ),
            ControlKind::LayeredNumber => value
                .parse::<f64>()
                .map_err(|_| format!("invalid numeric inspector value: {value}"))
                .and_then(|value| {
                    let value = control.store_number(value);
                    if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
                        if !value.is_finite()
                            || value.fract() != 0.0
                            || !(0.0..=f64::from(u32::MAX)).contains(&value)
                        {
                            return Err("invalid unsigned integer timeline value".to_string());
                        }
                        Ok(Value::from(value as u32))
                    } else {
                        finite_number(value)
                    }
                })
                .and_then(|value| {
                    self.set_timeline_base_with_commit(
                        target,
                        &control.path,
                        value,
                        control_commit(control),
                    )
                }),
            ControlKind::LayeredText => self.set_text_value(
                target,
                &control.path,
                control.timeline_id.ok_or("text timeline is unavailable")?,
                value.to_string(),
                control_commit(control),
            ),
            ControlKind::LayeredBoolean => value
                .parse::<bool>()
                .map_err(|_| format!("invalid boolean inspector value: {value}"))
                .and_then(|value| self.set_bool_value(target, &control.path, value)),
            ControlKind::LayeredSelector => self.set_timeline_base_with_commit(
                target,
                &control.path,
                Value::String(value.to_string()),
                control_commit(control),
            ),
            ControlKind::Number => value
                .parse::<f64>()
                .map(|value| control.store_number(value).to_string())
                .map_err(|_| format!("invalid numeric inspector value: {value}"))
                .and_then(|value| self.set_basic_regular_value(target, control, &value)),
            _ => self.set_basic_regular_value(target, control, value),
        }
    }

    pub fn set_basic_control_components(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        values: &[f64],
    ) -> Result<(), String> {
        let expected = match control.kind {
            ControlKind::Vector2 | ControlKind::LayeredVector2 => 2,
            ControlKind::Vector3 | ControlKind::LayeredVector3 => 3,
            ControlKind::Color | ControlKind::LayeredColor => 4,
            _ => return Err("inspector control does not have numeric components".into()),
        };
        if values.len() != expected || values.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "inspector control requires {expected} finite components"
            ));
        }
        if control.kind == ControlKind::Color
            && matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
            && !control.commit_name.is_empty()
            && values.iter().all(|value| (0.0..=255.0).contains(value))
            && let Some(result) = self.set_manim_color(
                target,
                &control.path,
                shrimply_project_document::project::Color::new(
                    values[0] as u8,
                    values[1] as u8,
                    values[2] as u8,
                    values[3] as u8,
                ),
                &control.commit_name,
            )
        {
            return result;
        }
        if let Some(crate::InspectorControlAction::SetSam2PointPosition {
            modifier_id,
            point_id,
        }) = control.action
        {
            return self.set_sam2_point_position(
                target,
                modifier_id,
                point_id,
                values[0] * control.store_multiplier,
                values[1] * control.store_multiplier,
            );
        }
        if control.kind == ControlKind::LayeredColor {
            if values.iter().any(|value| !(0.0..=255.0).contains(value)) {
                return Err("color channels must be between 0 and 255".into());
            }
            return self.set_color_value(
                target,
                &control.path,
                control.timeline_id.ok_or("color timeline is unavailable")?,
                shrimply_property_model::Color::new(
                    values[0] as u8,
                    values[1] as u8,
                    values[2] as u8,
                    values[3] as u8,
                ),
                control_commit(control),
            );
        }
        if control.kind == ControlKind::LayeredVector2 {
            return self.set_vector2_value(
                target,
                &control.path,
                values[0] * control.store_multiplier,
                values[1] * control.store_multiplier,
                control_commit(control),
            );
        }
        if control.kind == ControlKind::LayeredVector3 {
            return self.set_vector3_value(
                target,
                &control.path,
                values[0] * control.store_multiplier,
                values[1] * control.store_multiplier,
                values[2] * control.store_multiplier,
                control_commit(control),
            );
        }
        let values = values
            .iter()
            .enumerate()
            .map(|(index, value)| (index, (value * control.store_multiplier).to_string()))
            .collect::<Vec<_>>();
        if matches!(target, InspectorTarget::Transition { .. }) && !control.commit_name.is_empty() {
            self.set_transition_components(
                target,
                &control.path,
                &values,
                &control.commit_name,
                control.commit_immediately,
            )
        } else {
            self.set_components(target, &control.path, &values)
        }
    }

    pub fn set_basic_control_fraction(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        value: shrimply_math_core::Fraction,
    ) -> Result<(), String> {
        if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
            && !control.commit_name.is_empty()
        {
            self.set_video_fraction(target, &control.path, value, &control.commit_name)
        } else {
            self.set_fraction(target, &control.path, value)
        }
    }

    pub fn commit_basic_control(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<(), String> {
        if control_commit(control) == InspectorCommit::Deferred {
            let project = self.project.borrow();
            crate::model::target_value(&project, target)
                .ok_or("inspector target is no longer available")?;
            shrimply_project_document::project::commit_edit(&project, &control.commit_name);
            drop(project);
            shrimply_editor_state::player_state::refresh_project(
                &self.player_state,
                crate::refresh::target_change(target, None, true),
            );
            return Ok(());
        }
        if matches!(
            control.kind,
            ControlKind::LayeredNumber
                | ControlKind::LayeredBoolean
                | ControlKind::LayeredSelector
                | ControlKind::LayeredVector2
                | ControlKind::LayeredVector3
        ) {
            self.finish_live_inspector_edit(target)
        } else if matches!(target, InspectorTarget::Transition { .. })
            && !control.commit_name.is_empty()
            && !control.commit_immediately
        {
            self.commit_transition_field(target, &control.commit_name)
        } else if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
            && !control.commit_name.is_empty()
            && !control.commit_immediately
        {
            self.commit_video_field(target, &control.commit_name)
        } else {
            self.finish_live_edit();
            Ok(())
        }
    }

    fn set_basic_regular_value(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        value: &str,
    ) -> Result<(), String> {
        if matches!(target, InspectorTarget::Transition { .. }) && !control.commit_name.is_empty() {
            self.set_transition_field(
                target,
                &control.path,
                value,
                &control.commit_name,
                control.commit_immediately,
            )
        } else if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
            && !control.commit_name.is_empty()
        {
            self.set_video_field(
                target,
                &control.path,
                value,
                &control.commit_name,
                control.commit_immediately,
            )
        } else {
            self.set_field(target, &control.path, value)
        }
    }
}

fn control_commit(control: &InspectorControl) -> InspectorCommit<'_> {
    if control.commit_immediately {
        InspectorCommit::Immediate(&control.commit_name)
    } else if matches!(
        control.kind,
        ControlKind::LayeredNumber
            | ControlKind::LayeredVector2
            | ControlKind::LayeredVector3
            | ControlKind::LayeredColor
    ) {
        // Match GTK: changes only update the live preview; the picker commits once.
        InspectorCommit::Deferred
    } else {
        InspectorCommit::Coalesced(&control.commit_name)
    }
}

fn finite_number(value: f64) -> Result<Value, String> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| "timeline value must be finite".to_string())
}
