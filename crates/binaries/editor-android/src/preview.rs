//! Native Vulkan texture borrowing and presentation; planning and playback are shared.
use crate::runtime::Runtime;
use shrimply_preferences_core::{PreviewDownsampleMethod, PreviewUpsampleMethod};
use shrimply_preview_interaction_skia::captions;
use shrimply_preview_provider_skia::{Color, canvas, math};
use shrimply_project_document::{CaptionItem, Time};
use shrimply_render_vulkan::{Texture, surface::Renderer};

#[derive(Clone, PartialEq)]
pub(crate) struct Frame {
    pub texture: Option<Texture>,
    canvas_size: glam::UVec2,
    background: Color,
    padding: u32,
    shadow: u32,
    upsample: PreviewUpsampleMethod,
    downsample: PreviewDownsampleMethod,
    captions: Vec<CaptionItem>,
    caption_position: Time,
    caption_size: f32,
    caption_background: Color<u8>,
}

impl Frame {
    pub fn new(runtime: &Runtime, texture: Option<Texture>) -> Self {
        let project = runtime.backend.project();
        let size = project.as_ref().map_or_else(
            || shrimply_project_document::Project::default().canvas_size,
            |project| project.canvas_size,
        );
        let preferences = shrimply_preferences_core::snapshot(&runtime.backend.preferences);
        let position = shrimply_player_core::current_time(&runtime.player);
        let captions = project.as_ref().map_or_else(Vec::new, |project| {
            captions::active_captions(project, position, preferences.caption_background_color)
        });
        let caption_position = if captions.is_empty() {
            Time::ZERO
        } else {
            position
        };
        Self {
            texture,
            canvas_size: glam::uvec2(size.width, size.height),
            background: shrimply_cross_ui_theme::current().window_bg,
            padding: preferences.preview_padding_px,
            shadow: preferences.preview_shadow_size_px,
            upsample: preferences.preview_upsample_method,
            downsample: preferences.preview_downsample_method,
            captions,
            caption_position,
            caption_size: preferences.caption_font_size,
            caption_background: preferences.caption_background_color,
        }
    }
}

pub(crate) fn draw(renderer: &mut Renderer, frame: &Frame, density: f32) -> bool {
    renderer.draw_texture(
        frame.texture.as_ref(),
        None,
        |target, image, width, height| {
            let rect = math::video_content_rect(
                glam::ivec2(width as i32, height as i32),
                frame.canvas_size,
                (frame.padding as f32 * density).round() as u32,
            );
            canvas::draw_background(
                target,
                canvas::Appearance {
                    content_rect: rect,
                    background: frame.background,
                    shadow_size: (frame.shadow as f32 * density).round() as u32,
                    pixel_scale: density,
                },
            );
            if let Some(image) = image {
                let sampling = shrimply_preview_render_core::math::preview_sampling(
                    rect.width() < image.width() as f32 || rect.height() < image.height() as f32,
                    frame.upsample,
                    frame.downsample,
                );
                target.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    skia_safe::Rect::from_xywh(rect.min.x, rect.min.y, rect.width(), rect.height()),
                    sampling,
                    &skia_safe::Paint::default(),
                );
            }
            target.save();
            target.scale((density, density));
            captions::draw_active_captions(
                &shrimply_components_skia::canvas::TimelinePainter::new(target),
                &frame.captions,
                frame.caption_position,
                captions::CaptionAppearance {
                    preview_rect: shrimply_components_skia::canvas::Rect::from_min_size(
                        glam::Vec2::ZERO,
                        glam::vec2(width as f32 / density, height as f32 / density),
                    ),
                    font_size: frame.caption_size,
                    background_color: frame.caption_background,
                    bottom_inset: 0.0,
                },
                None,
            );
            target.restore();
        },
    )
}
