use super::*;
use shrimply_preview_core::{PointerButton, PointerEvent};
use shrimply_preview_interaction_core::captions::{self, CaptionAppearance};
use shrimply_project::project::{ItemAddress, Project};
use shrimply_state::{player_state, preferences};
use shrimply_timeline_core::selection_state;

pub(in crate::macos::canvas) fn appearance(
    size: NSSize,
    prefs: &preferences::PreferencesSnapshot,
    bottom_inset: f32,
) -> CaptionAppearance {
    CaptionAppearance {
        preview_rect: shrimply_skia_adw_core::Rect::from_min_size(
            glam::Vec2::ZERO,
            glam::Vec2::new(size.width as f32, size.height as f32),
        ),
        font_size: prefs.caption_font_size,
        background_color: prefs.caption_background_color,
        bottom_inset,
    }
}

impl CanvasView {
    pub(super) fn preview_caption_pointer(&self, event: &PointerEvent<'_>) -> Result<bool, String> {
        let session = &self.ivars().session;
        let player = player_state::snapshot(&session.player_state);
        let prefs = preferences::snapshot(&session.preferences);
        let mut split = None;
        {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(state) = &mut *content else {
                return Ok(false);
            };
            let point = match event {
                PointerEvent::Hover(input) => Some(input.sample.position),
                PointerEvent::Begin(input) if input.button == PointerButton::Primary => {
                    Some(input.sample.position)
                }
                _ => None,
            };
            let target = if state.controller.sequence == PointerSequence::Idle
                && !state.guide_input.active()
                && (!matches!(event, PointerEvent::Hover(_))
                    || state.guide_input.cursor() == GuideCursor::Default)
            {
                point.and_then(|point| {
                    let project = session.project.borrow();
                    let address =
                        selection_state::focused_item_address(&session.selection_state, &project)?;
                    let byte = captions::split_at_position(
                        &project,
                        &address,
                        player.position,
                        appearance(self.bounds().size, &prefs, state.caption_bottom_inset),
                        point,
                    )?;
                    Some((address, byte))
                })
            } else {
                None
            };
            let previous = state.caption_split_hover;
            state.caption_split_hover =
                if matches!(event, PointerEvent::Hover(_)) && target.is_some() {
                    point
                } else {
                    None
                };
            if previous != state.caption_split_hover {
                self.setNeedsDisplay(true);
            }
            if state.caption_split_hover.is_some() {
                objc2_app_kit::NSCursor::IBeamCursor().set();
                return Ok(true);
            }
            if previous.is_some() && state.guide_input.cursor() == GuideCursor::Default {
                objc2_app_kit::NSCursor::arrowCursor().set();
            }
            if matches!(event, PointerEvent::Begin(_)) {
                split = target;
            }
        }
        let Some((address, byte)) = split else {
            return Ok(false);
        };
        let right = {
            let mut project = session.project.borrow_mut();
            let (_, right) = shrimply_timeline_core::edit::split_caption(
                &mut project,
                &address,
                player.position,
                byte,
            )?;
            shrimply_project::project::commit_edit(&project, "split-preview-caption");
            right
        };
        {
            let project = session.project.borrow();
            selection_state::set_selected_item_addresses(
                &session.selection_state,
                &project,
                vec![right.clone()],
                Some(right),
            );
        }
        player_state::refresh_project(
            &session.player_state,
            player_state::ProjectChange {
                captions: true,
                inspector: true,
                ..Default::default()
            },
        );
        self.setNeedsDisplay(true);
        Ok(true)
    }
}

pub(in crate::macos::canvas) fn draw(
    canvas: &skia_safe::Canvas,
    state: &State,
    project: &Project,
    position: shrimply_math_core::Time,
    appearance: CaptionAppearance,
    focused: Option<&ItemAddress>,
) {
    captions::draw_captions(
        &shrimply_skia_adw_core::canvas::TimelinePainter::new(canvas),
        project,
        position,
        appearance,
        focused
            .zip(state.caption_split_hover)
            .map(|(address, point)| {
                (
                    address,
                    point,
                    shrimply_cross_ui_theme::current().accent_blue_standalone,
                )
            }),
    );
}
