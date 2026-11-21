//! Shared Skia preview background and overlay renderer used by all platforms.
use std::cell::RefCell;

use glam::Vec2;
use shrimply_components_skia::canvas::TimelinePainter;
use shrimply_math_color::Color;
use shrimply_preview_provider_skia::{PreviewViewport, Rect, canvas};
use shrimply_project_document::project::{ItemAddress, PreviewGuides, Project, Time};
use shrimply_project_evaluation::TransformExpressionCache;

use crate::{
    captions::{self, CaptionAppearance},
    controller::Controller,
    guides,
};

pub fn background_color(window_color: Color, fullscreen: bool) -> Color {
    if fullscreen {
        Color::BLACK
    } else {
        window_color
    }
}

pub fn clipped_content_rect(content_rect: Rect, clip_rect: Rect) -> Rect {
    Rect::from_min_max(
        content_rect.min.max(clip_rect.min),
        content_rect.max.min(clip_rect.max),
    )
}

#[derive(Clone, Copy)]
pub struct BackgroundAppearance {
    pub content_rect: Rect,
    pub clip_rect: Rect,
    pub background: Color,
    pub shadow_size: u32,
    pub pixel_scale: f32,
}

pub fn draw_background(target: &skia_safe::Canvas, appearance: BackgroundAppearance) {
    canvas::draw_background(
        target,
        canvas::Appearance {
            content_rect: clipped_content_rect(appearance.content_rect, appearance.clip_rect),
            background: appearance.background,
            shadow_size: appearance.shadow_size,
            pixel_scale: appearance.pixel_scale,
        },
    );
}

pub struct InteractionOverlay<'a> {
    pub viewport: PreviewViewport,
    pub clip_rect: Rect,
    pub surface_rect: Rect,
    pub guides: Option<&'a PreviewGuides>,
}

pub struct PreviewOverlayFrame<'a> {
    pub project: &'a Project,
    pub position: Time,
    pub captions: CaptionAppearance,
    pub caption_split_hover: Option<(&'a ItemAddress, Vec2)>,
    pub interaction: InteractionOverlay<'a>,
}

/// Draws guides, rulers, and the clipped preview interaction provider on a Skia canvas.
pub fn draw_interaction(
    canvas: &skia_safe::Canvas,
    frame: &InteractionOverlay<'_>,
    controller: &mut Controller,
    expressions: &RefCell<TransformExpressionCache>,
    after_controls: impl FnOnce(&skia_safe::Canvas),
) {
    let accent = shrimply_cross_ui_theme::current().accent_bg;
    if let Some(guides) = frame.guides {
        guides::draw(
            &TimelinePainter::new(canvas),
            guides,
            frame.viewport,
            frame.surface_rect,
            accent,
        );
    }
    canvas.save();
    canvas.clip_rect(skia_safe::Rect::from(frame.clip_rect), None, false);
    if let Some(retiring) = controller.retiring_provider.as_mut() {
        retiring.context.viewport = frame.viewport;
    }
    controller.draw(canvas, expressions);
    after_controls(canvas);
    canvas.restore();
}

/// Draws captions, guides, rulers, and clipped preview provider controls on a Skia canvas.
pub fn draw_overlay(
    canvas: &skia_safe::Canvas,
    frame: &PreviewOverlayFrame<'_>,
    controller: &mut Controller,
    expressions: &RefCell<TransformExpressionCache>,
    after_controls: impl FnOnce(&skia_safe::Canvas),
) {
    let accent = shrimply_cross_ui_theme::current().accent_bg;
    captions::draw_captions(
        &TimelinePainter::new(canvas),
        frame.project,
        frame.position,
        frame.captions,
        frame
            .caption_split_hover
            .map(|(address, point)| (address, point, accent)),
    );
    draw_interaction(
        canvas,
        &frame.interaction,
        controller,
        expressions,
        after_controls,
    );
}
