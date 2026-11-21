//! One bounded uniform allocation; lighting edits never invalidate voxel storage.
use crate::RenderSettings;
use sanim_voxel::{Light, LightKind, Lighting, MAX_LIGHTS, SceneLightParameters, ShadowMode};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    position_kind: [f32; 4],
    radiance_radius: [f32; 4],
    tangent_cosine: [f32; 4],
    bitangent_pdf: [f32; 4],
    cumulative: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLights {
    meta: [u32; 4],
    lights: [GpuLight; MAX_LIGHTS],
}
pub(crate) const UNIFORM_BYTES: u64 = size_of::<GpuLights>() as u64;

struct Key {
    lights: Option<Arc<[Light]>>,
    legacy: Lighting,
    dimensions: [u32; 3],
    mode: ShadowMode,
}
impl Key {
    fn matches(&self, settings: &RenderSettings, dimensions: [u32; 3], mode: ShadowMode) -> bool {
        self.dimensions == dimensions
            && self.mode == mode
            && match (&self.lights, &settings.lights) {
                (None, None) => self.legacy == settings.lighting,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                _ => false,
            }
    }
}

pub(crate) struct Lights {
    pub(crate) buffer: wgpu::Buffer,
    key: Option<Key>,
}
impl Lights {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Voxel scene lights"),
                size: UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            key: None,
        }
    }
    pub(crate) fn reset(&mut self) {
        self.key = None;
    }
    pub(crate) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        settings: &RenderSettings,
        dimensions: [u32; 3],
    ) -> Result<u64, String> {
        let mode = settings.effective_modes().0;
        if self
            .key
            .as_ref()
            .is_some_and(|key| key.matches(settings, dimensions, mode))
        {
            return Ok(0);
        }
        let resolved = resolved(settings, dimensions)?;
        let mut gpu: GpuLights = bytemuck::Zeroable::zeroed();
        gpu.meta[0] = resolved.len() as u32;
        for (output, input) in gpu.lights.iter_mut().zip(resolved) {
            *output = GpuLight {
                position_kind: [
                    input.position_or_direction[0],
                    input.position_or_direction[1],
                    input.position_or_direction[2],
                    input.kind as u32 as f32,
                ],
                radiance_radius: [
                    input.radiance[0],
                    input.radiance[1],
                    input.radiance[2],
                    input.radius,
                ],
                tangent_cosine: [
                    input.tangent[0],
                    input.tangent[1],
                    input.tangent[2],
                    input.cos_radius,
                ],
                bitangent_pdf: [
                    input.bitangent[0],
                    input.bitangent[1],
                    input.bitangent[2],
                    input.pdf,
                ],
                cumulative: [input.cdf, 0., 0., 0.],
            };
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&gpu));
        self.key = Some(Key {
            lights: settings.lights.clone(),
            legacy: settings.lighting,
            dimensions,
            mode,
        });
        Ok(UNIFORM_BYTES)
    }
}

pub(crate) fn resolved(
    settings: &RenderSettings,
    dimensions: [u32; 3],
) -> Result<Vec<SceneLightParameters>, String> {
    let mode = settings.effective_modes().0;
    Ok(if let Some(lights) = &settings.lights {
        sanim_voxel::scene_light_parameters(lights, dimensions, mode)?
    } else {
        let mut legacy = settings.lighting;
        legacy.shadows.mode = mode;
        let legacy = legacy.parameters()?;
        vec![SceneLightParameters {
            kind: LightKind::Sun,
            position_or_direction: legacy.direction,
            radiance: legacy.key_radiance,
            radius: 0.,
            cos_radius: legacy.cos_radius,
            tangent: legacy.tangent,
            bitangent: legacy.bitangent,
            pdf: 1.,
            cdf: 1.,
        }]
    })
}
