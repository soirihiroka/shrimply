use shrimply_video_cuda::modifier_cache;
use shrimply_video_modifiers::{
    ModifierEffect, RasterModifierEffect,
    cache::{CacheModifier, CacheQuality},
};

use crate::{
    CacheStatus, ControlKind, InspectorControl, InspectorController, InspectorRuntime,
    InspectorSection, InspectorTarget,
};

pub(super) fn presentation(
    value: &CacheModifier,
    index: usize,
    id: uuid::Uuid,
    _runtime: InspectorRuntime,
) -> InspectorSection {
    let base = format!("/modifiers/{index}/effect/effect/config");
    let status = visual_cache_status(id);
    let baking = matches!(status, CacheStatus::Baking { .. });
    let mut section = InspectorSection::default();
    section.add(
        InspectorControl::new(
            ControlKind::VisualCacheQuality,
            format!("{base}/quality"),
            "Format",
        )
        .value(
            serde_json::to_value(value.quality)
                .expect("cache quality must serialize")
                .as_str()
                .expect("cache quality must serialize as text"),
        )
        .choices(
            vec![
                "compact".into(),
                "balanced".into(),
                "high".into(),
                "lossless".into(),
            ],
            vec![
                "H.265 · Compact".into(),
                "H.265 · Balanced".into(),
                "H.265 · High".into(),
                "H.265 · Lossless".into(),
            ],
        )
        .target(id)
        .sensitive(!baking)
        .immediate_commit("visual-cache-format"),
    );
    let control = crate::cache_control_presentation(status, "");
    section.add(
        InspectorControl::new(ControlKind::VisualCache, format!("{base}/bake"), "")
            .value(control.label)
            .components(vec![
                control.progress.to_string(),
                u8::from(control.baking).to_string(),
            ])
            .tooltip(control.tooltip)
            .target(id),
    );
    section
}

pub fn visual_cache_status(id: uuid::Uuid) -> CacheStatus {
    match modifier_cache::status(id) {
        modifier_cache::Status::Missing => CacheStatus::Missing,
        modifier_cache::Status::Baking { completed, total } => {
            CacheStatus::Baking { completed, total }
        }
        modifier_cache::Status::Ready => CacheStatus::Ready,
        modifier_cache::Status::Failed(error) => CacheStatus::Failed(error),
    }
}

impl InspectorController {
    pub fn set_visual_cache_quality(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        quality: &str,
    ) -> Result<(), String> {
        let quality: CacheQuality =
            serde_json::from_value(serde_json::Value::String(quality.to_string()))
                .map_err(|_| format!("unknown visual cache quality: {quality}"))?;
        let mut project = self.project.borrow_mut();
        let cache = project
            .video_item_mut(super::video_address(target)?)
            .and_then(|item| item.modifiers.iter_mut().find(|modifier| modifier.id == id))
            .and_then(|modifier| match &mut modifier.effect {
                ModifierEffect::Raster(effect) => match &mut **effect {
                    RasterModifierEffect::Cache(cache) => Some(cache),
                    _ => None,
                },
                _ => None,
            })
            .ok_or_else(|| "visual cache modifier is no longer available".to_string())?;
        if cache.quality == quality {
            return Ok(());
        }
        cache.quality = quality;
        shrimply_project::project::commit_edit(&project, "visual-cache-format");
        drop(project);
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            shrimply_state::player_state::ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn toggle_visual_cache(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
    ) -> Result<(), String> {
        let address = super::video_address(target)?.clone();
        let project = self.project.borrow();
        let available = project.video_item(&address).is_some_and(|item| {
            item.modifiers.iter().any(|modifier| {
                modifier.id == id
                    && matches!(
                        modifier.effect,
                        ModifierEffect::Raster(ref effect)
                            if matches!(&**effect, RasterModifierEffect::Cache(_))
                    )
            })
        });
        if !available {
            return Err("visual cache modifier is no longer available".to_string());
        }
        if matches!(visual_cache_status(id), CacheStatus::Baking { .. }) {
            drop(project);
            modifier_cache::invalidate(id)?;
        } else {
            let project = project.clone();
            modifier_cache::bake(project, address, id)?;
        }
        self.refresh_visual_cache();
        Ok(())
    }

    pub fn refresh_visual_cache(&self) {
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            shrimply_state::player_state::ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        );
    }
}
