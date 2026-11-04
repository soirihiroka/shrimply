use super::Compositor;
use shrimply_preview_render_core::ManimFrame;
use shrimply_render_vulkan::{Buffer, ExternalImage};
use std::sync::Arc;

pub(super) struct Slot {
    pub id: usize,
    image: ExternalImage,
    prepared: Arc<shrimply_manim_wgpu::PreparedAnimation>,
    frame: usize,
    buffer: Buffer,
}

impl Compositor {
    pub(super) fn manim_frame(&mut self, frame: &ManimFrame) -> Result<Buffer, String> {
        self.used_manim.insert(frame.item_id);
        let result = self.render_manim(frame);
        self.manim_updates
            .push(frame.source.error(result.as_ref().err().cloned()));
        result
    }

    fn render_manim(&mut self, frame: &ManimFrame) -> Result<Buffer, String> {
        if let Some(slot) = self.manim_slots.get(&frame.item_id)
            && Arc::ptr_eq(&slot.prepared, &frame.prepared)
            && slot.frame == frame.frame_index
        {
            return Ok(slot.buffer.clone());
        }
        if self.manim.is_none() {
            self.manim = Some(shrimply_manim_wgpu::vulkan::Renderer::new(
                self.renderer.device_uuid(),
            )?);
        }
        let renderer = self
            .manim
            .as_mut()
            .expect("initialized Intel Manim renderer");
        let descriptor = shrimply_manim_wgpu::Renderer::external_frame_descriptor(&frame.prepared);
        let slot_id = match self.manim_slots.get(&frame.item_id) {
            Some(slot) if renderer.target_descriptor(slot.id) == Some(descriptor) => slot.id,
            _ => {
                if let Some(old) = self.manim_slots.remove(&frame.item_id) {
                    renderer.remove_target(old.id);
                }
                let id = self.next_manim_slot;
                self.next_manim_slot = id.checked_add(1).expect("Manim render slot overflow");
                id
            }
        };
        let rendered = renderer.render_shared(slot_id, &frame.prepared, frame.frame_index)?;
        if let Some(slot) = self.manim_slots.get_mut(&frame.item_id) {
            slot.buffer = self
                .renderer
                .copy_external(&slot.image, rendered.semaphore_value)?;
            slot.prepared = frame.prepared.clone();
            slot.frame = frame.frame_index;
            return Ok(slot.buffer.clone());
        }
        let imported = ExternalImage::new(renderer.export_frame(slot_id)?);
        let image = match imported {
            Ok(image) => image,
            Err(error) => {
                renderer.remove_target(slot_id);
                return Err(error);
            }
        };
        let buffer = self
            .renderer
            .copy_external(&image, rendered.semaphore_value)?;
        self.manim_slots.insert(
            frame.item_id,
            Slot {
                id: slot_id,
                image,
                prepared: frame.prepared.clone(),
                frame: frame.frame_index,
                buffer: buffer.clone(),
            },
        );
        Ok(buffer)
    }
}
