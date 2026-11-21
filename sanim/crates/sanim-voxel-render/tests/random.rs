use sanim_voxel_render::{Limits, Renderer};
use std::{borrow::Cow, sync::mpsc};

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn actual_gpu_random_never_rounds_the_open_upper_endpoint_to_one() {
    let renderer = Renderer::headless(Limits::default()).unwrap();
    let source = concat!(
        "const TAU: f32 = 6.28318530718;\n",
        include_str!("../src/math.wgsl"),
        r#"
        @group(0) @binding(0) var<storage, read_write> output: array<f32>;
        @compute @workgroup_size(1) fn check_rng(@builtin(global_invocation_id) index: vec3<u32>) {
            // Analytic inverses of the PCG permutation/LCG: their old full-u32
            // float conversion rounded the resulting top words up to1.0.
            let seeds = array<u32,3>(2222045655u,1164488078u,2150647986u);
            var state = seeds[index.x];
            output[index.x] = random(&state);
        }
    "#
    );
    let shader = renderer
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Actual RNG endpoint regression"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
        });
    let pipeline = renderer
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RNG endpoint test"),
            layout: None,
            module: &shader,
            entry_point: Some("check_rng"),
            compilation_options: Default::default(),
            cache: None,
        });
    let output = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("RNG samples"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("RNG samples readback"),
        size: 12,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = renderer
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RNG samples"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(3, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 12);
    let submission = renderer.queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::sync_channel(1);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    for sample in mapped.chunks_exact(4) {
        let sample = f32::from_le_bytes(sample.try_into().unwrap());
        assert!(
            (0.0..1.0).contains(&sample),
            "RNG sample must be within[0,1), got{sample}"
        );
    }
    drop(mapped);
    readback.unmap();
}
