use super::*;
use sanim_runtime::{Progress, State};
use shrimply_math_color::Color;
use skia_safe::{Font, FontMgr, FontStyle, Paint, Rect};

pub(super) struct Feedback {
    pub generation: u64,
    pub canvas: CanvasSize,
    pub base: Option<Arc<Recording>>,
    pub recording: Arc<Recording>,
}

/// Draw commands only: the compositor replays this on its existing GPU canvas.
pub(super) fn record(
    canvas: CanvasSize,
    progress: &Progress,
    error: Option<&str>,
    base: Option<&Recording>,
) -> Result<Arc<Recording>, String> {
    Recording::capture(canvas.width, canvas.height, |target| {
        if let Some(base) = base {
            base.draw(target);
        }
        let width = canvas.width as f32;
        let height = canvas.height as f32;
        let overlay = base.is_some();
        let size = if overlay {
            (width.min(height) * 0.02).clamp(8., 18.)
        } else {
            (width.min(height) * 0.033).clamp(12., 32.)
        };
        let margin = if overlay { size * 0.5 } else { size };
        let font = FontMgr::new()
            .legacy_make_typeface(None, FontStyle::default())
            .map(|face| Font::from_typeface(face, size))
            .unwrap_or_else(|| {
                let mut font = Font::default();
                font.set_size(size);
                font
            });
        let failed = error.is_some() || progress.state == State::Failed;
        let title = match (failed, overlay) {
            (true, true) => "Sanim reload failed — showing last valid animation",
            (true, false) => "Sanim could not load",
            (false, true) => "Reloading Sanim — showing last valid animation",
            (false, false) => "Preparing Sanim",
        };
        let mut lines = vec![title.to_owned()];
        if let Some(error) = error {
            lines.extend(wrap(error, &font, width - margin * 2.));
        } else {
            lines.push(progress.phase.label().into());
            if let Some(resource) = &progress.resource {
                lines.extend(wrap(resource, &font, width - margin * 2.));
            }
            if let (Some(done), Some(total)) = (progress.completed, progress.total) {
                lines.push(format!("{done} / {total} completed in this phase"));
            }
            lines.push(format!(
                "Elapsed: {:.1}s",
                progress.elapsed_micros as f64 / 1_000_000.
            ));
        }
        let line_height = size * 1.45;
        let max_height = if overlay { height * 0.25 } else { height };
        let available = ((max_height - margin * 2.) / line_height).floor().max(1.) as usize;
        if lines.len() > available {
            lines.truncate(available);
            if let Some(last) = lines.last_mut() {
                last.push_str(" …");
            }
        }
        let panel_height = (lines.len() as f32 * line_height + margin * 2.).min(max_height);
        let top = if overlay { height - panel_height } else { 0. };
        let mut paint = Paint::default();
        paint.set_color(skia_safe::Color::from(Color::WINDOW_BG_DARK));
        target.draw_rect(
            if overlay {
                Rect::from_xywh(0., top, width, panel_height)
            } else {
                Rect::from_wh(width, height)
            },
            &paint,
        );
        let start = if overlay {
            top + margin
        } else {
            ((height - panel_height) * 0.5).max(0.) + margin
        };
        for (index, line) in lines.iter().enumerate() {
            paint.set_color(skia_safe::Color::from(if index == 0 && failed {
                Color::ERROR_DARK
            } else {
                Color::LIGHT1
            }));
            target.draw_str(
                line,
                (margin, start + size + index as f32 * line_height),
                &font,
                &paint,
            );
        }
        Ok(())
    })
    .map(Arc::new)
}

fn wrap(text: &str, font: &Font, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        for character in line.chars() {
            current.push(character);
            if font.measure_str(&current, None).0 > width && current.len() > character.len_utf8() {
                current.pop();
                lines.push(std::mem::take(&mut current));
                current.push(character);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
