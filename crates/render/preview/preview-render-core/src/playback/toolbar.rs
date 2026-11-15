//! Preview status and completed-render throughput shared by native toolbars.
use std::time::Duration;

use shrimply_math_core::Fraction;
use shrimply_preview_provider_skia::playback::{playback_speed_label, rendered_frame_rate_label};

#[derive(Default)]
pub struct Toolbar {
    loading: bool,
    frame_rate: Option<String>,
}

pub struct Presentation {
    pub loading: bool,
    pub status: String,
    pub frame_rate: String,
    pub frame_rate_tooltip: String,
    pub speed: String,
    pub speed_tooltip: String,
    pub guides: String,
}

impl Toolbar {
    /// Observe completed compositor work using the same throughput display as GTK.
    pub fn update(&mut self, loading: Option<bool>, elapsed: Option<Duration>) -> bool {
        let mut changed = false;
        if let Some(loading) = loading {
            changed |= self.loading != loading;
            self.loading = loading;
        }
        if let Some(rate) = elapsed.and_then(rendered_frame_rate_label) {
            changed |= self.frame_rate.as_ref() != Some(&rate);
            self.frame_rate = Some(rate);
        }
        changed
    }

    pub fn presentation(&self, speed: Fraction) -> Presentation {
        Presentation {
            loading: self.loading,
            status: shrimply_i18n::text(if self.loading {
                "Loading preview"
            } else {
                "Preview ready"
            })
            .into_owned(),
            frame_rate: self
                .frame_rate
                .clone()
                .unwrap_or_else(|| shrimply_i18n::text("--").into_owned()),
            frame_rate_tooltip: shrimply_i18n::text("Frame rate").into_owned(),
            speed: playback_speed_label(speed),
            speed_tooltip: shrimply_i18n::text("Playback speed").into_owned(),
            guides: shrimply_i18n::text("Guide").into_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextAction {
    CopyImage,
    SaveImage,
    ShowPreviewControls,
}

pub struct ContextItem {
    pub action: ContextAction,
    pub label: String,
    pub checked: Option<bool>,
}

pub enum ContextRequest {
    CopyImage,
    SaveImage,
}

impl ContextAction {
    pub fn id(self) -> &'static str {
        match self {
            Self::CopyImage => "copy-image",
            Self::SaveImage => "save-image",
            Self::ShowPreviewControls => "show-preview-controls",
        }
    }

    pub fn activate(
        self,
        preferences: &shrimply_preferences_core::SharedPreferences,
    ) -> Option<ContextRequest> {
        match self {
            Self::CopyImage => Some(ContextRequest::CopyImage),
            Self::SaveImage => Some(ContextRequest::SaveImage),
            Self::ShowPreviewControls => {
                shrimply_preferences_core::set_preview_controls_visible(
                    preferences,
                    !shrimply_preferences_core::snapshot(preferences).preview_controls_visible,
                );
                None
            }
        }
    }
}

/// Image services are advertised only by hosts that can actually provide them.
pub fn context_menu(
    preferences: &shrimply_preferences_core::SharedPreferences,
    image_services: bool,
) -> Vec<ContextItem> {
    let mut items = Vec::new();
    if image_services {
        items.extend([
            ContextItem {
                action: ContextAction::CopyImage,
                label: shrimply_i18n::text("Copy Preview Image").into_owned(),
                checked: None,
            },
            ContextItem {
                action: ContextAction::SaveImage,
                label: shrimply_i18n::text("Save Preview Image…").into_owned(),
                checked: None,
            },
        ]);
    }
    items.push(ContextItem {
        action: ContextAction::ShowPreviewControls,
        label: shrimply_i18n::text(
            shrimply_preferences_core::PreferenceId::PreviewControlsVisible
                .presentation()
                .label,
        )
        .into_owned(),
        checked: Some(shrimply_preferences_core::snapshot(preferences).preview_controls_visible),
    });
    items
}
