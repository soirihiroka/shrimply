use crate::{COLOR_FORMAT, RenderSettings};
use std::sync::mpsc;

pub(crate) struct Target {
    pub texture: wgpu::Texture,
    buffer: wgpu::Buffer,
    pub dimensions: [u32; 2],
    row_bytes: u32,
    padded: u32,
}

impl Target {
    pub fn required_bytes(settings: &RenderSettings) -> u64 {
        let row = settings.width * 4;
        u64::from(
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
                + row,
        ) * u64::from(settings.height)
    }

    pub fn new(device: &wgpu::Device, settings: &RenderSettings) -> Self {
        let row_bytes = settings.width * 4;
        let padded = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Voxel export"),
            size: wgpu::Extent3d {
                width: settings.width,
                height: settings.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Voxel export readback"),
            size: u64::from(padded) * u64::from(settings.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            buffer,
            dimensions: [settings.width, settings.height],
            row_bytes,
            padded,
        }
    }

    pub fn bytes(&self) -> u64 {
        u64::from(self.row_bytes + self.padded) * u64::from(self.dimensions[1])
    }

    pub fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Vec<u8>, String> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Voxel export copy"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded),
                    rows_per_image: Some(self.dimensions[1]),
                },
            },
            self.texture.size(),
        );
        let submission = queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::sync_channel(1);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("wait for voxel export: {error}"))?;
        receive
            .recv()
            .map_err(|error| format!("receive voxel export: {error}"))?
            .map_err(|error| format!("map voxel export: {error}"))?;
        let mapped = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| format!("read voxel export: {error}"))?;
        let mut pixels = Vec::with_capacity(self.row_bytes as usize * self.dimensions[1] as usize);
        for row in mapped
            .chunks_exact(self.padded as usize)
            .take(self.dimensions[1] as usize)
        {
            pixels.extend_from_slice(&row[..self.row_bytes as usize]);
        }
        drop(mapped);
        self.buffer.unmap();
        Ok(pixels)
    }
}
