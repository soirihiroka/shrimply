use crate::{Buffer, Renderer, Submission};
use shrimply_asset::AssetSnapshot;
use shrimply_render_3d_core::{
    ComputeMaterial, ComputePbr, PreparedFrame, SceneIdentity, SoftwareBvhNode, obj as shader,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct SoftwareTexture {
    pixels: u64,
    width: u32,
    height: u32,
    levels: u32,
    padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OutputSize {
    width: u32,
    height: u32,
    denoising: u32,
    guides: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SoftwareResources {
    scene_data: u64,
    output_size_data: OutputSize,
    positions_data: u64,
    normals_data: u64,
    tangents_data: u64,
    tex_coords_0_data: u64,
    tex_coords_1_data: u64,
    vertex_colors_data: u64,
    materials_data: u64,
    mesh_instances_data: u64,
    bvh_nodes: u64,
    bvh_triangles: u64,
    material_texture_data: SoftwareTexture,
    environment_texture_data: SoftwareTexture,
    composite_texture_data: SoftwareTexture,
    output_pixels_data: u64,
    outline_guide_data: u64,
    outline_distance_data: u64,
    denoiser_beauty_data: u64,
    denoiser_background_data: u64,
    denoiser_alpha_data: u64,
    denoiser_depth_normal_data: u64,
    denoiser_albedo_data: u64,
    denoised_texture_data: u64,
    denoised_background_texture_data: u64,
    denoised_alpha_texture_data: u64,
    transport_work_data: u64,
    sample_work_data: u64,
    accumulation_data: u64,
    pass_count_data: u64,
    wavefront_origin: [u32; 2],
    wavefront_extent: [u32; 2],
}
include!(concat!(env!("OUT_DIR"), "/software_abi.rs"));
include!(concat!(env!("OUT_DIR"), "/software_uniforms.rs"));
include!(concat!(env!("OUT_DIR"), "/software_options.rs"));
include!(concat!(env!("OUT_DIR"), "/software_scratch.rs"));

// Keep per-ray state bounded independently of the canvas and reuse it across tiles.
const MAX_OBJECT_SCRATCH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SamplingKey {
    anti_aliasing: u32,
    path_tracing: u32,
    quality: u32,
    depth_of_field: bool,
    transmission: bool,
    light_sampling: u32,
    options: u32,
}

pub(super) struct SamplingCache {
    key: SamplingKey,
    samples: u32,
    transport_steps: u32,
}

pub(crate) struct Uploaded {
    identity: SceneIdentity,
    positions: Buffer,
    normals: Buffer,
    tangents: Buffer,
    tex_coords_0: Buffer,
    tex_coords_1: Buffer,
    colors: Buffer,
    materials: Buffer,
    instances: Buffer,
    nodes: Buffer,
    triangles: Buffer,
    texture: Buffer,
    material_texture: SoftwareTexture,
}

pub(crate) struct Environment {
    identity: AssetSnapshot,
    buffer: Buffer,
    texture: SoftwareTexture,
}

impl Renderer {
    pub fn object(
        &mut self,
        plan: &PreparedFrame,
        background: Option<(&Buffer, (u32, u32))>,
        submissions: &mut Vec<Submission>,
    ) -> Result<Buffer, String> {
        self.upload_object(plan)?;
        let count = (plan.width as usize)
            .checked_mul(plan.height as usize)
            .ok_or("3D canvas size overflow")?;
        let pixel_bytes = count
            .checked_mul(size_of::<u32>())
            .ok_or("3D output size overflow")?;
        let float_bytes = count
            .checked_mul(size_of::<[f32; 4]>())
            .ok_or("3D float output size overflow")?;
        let output = self.allocate(pixel_bytes)?;
        let fallback = self.upload(&[0; size_of::<[f32; 4]>()])?;
        let empty_texture = SoftwareTexture {
            pixels: fallback.address(),
            width: 1,
            height: 1,
            levels: 1,
            padding: 0,
        };
        let (environment_texture, environment_buffer) =
            self.object_environment(plan, empty_texture, &fallback, submissions)?;
        let (composite_texture, composite_buffer) = if let Some((background, size)) = background {
            let (texture, buffer) = self.float_texture(size, None)?;
            let mut args = self.arguments("packed_rgba_to_float")?;
            let pixels = u64::from(size.0) * u64::from(size.1);
            if usize::try_from(pixels).ok().and_then(|n| n.checked_mul(4)) != Some(background.len())
            {
                return Err("3D background dimensions do not match its buffer".into());
            }
            args.set("input", &background.address().to_ne_bytes())?
                .set("output", &buffer.address().to_ne_bytes())?
                .set("count", &pixels.to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    vec![background.clone(), buffer.clone()],
                    [pixels as usize, 1, 1],
                )
            }?);
            self.float_mips(&buffer, size, submissions)?;
            (texture, buffer)
        } else {
            (empty_texture, fallback.clone())
        };
        let denoising = plan.params.denoising != shrimply_scene_3d::DenoisingMethod::Off
            && plan.params.shading_model == shader::ShadingModel::Pbr;
        let oidn = denoising && match plan.params.denoising {
            #[cfg(target_os = "linux")]
            shrimply_scene_3d::DenoisingMethod::Auto => {
                shrimply_denoise_oidn::vulkan_supported(self.device_uuid())
            }
            #[cfg(target_os = "linux")]
            shrimply_scene_3d::DenoisingMethod::OpenImageDenoise => true,
            #[cfg(target_os = "android")]
            shrimply_scene_3d::DenoisingMethod::Auto => false,
            #[cfg(target_os = "android")]
            shrimply_scene_3d::DenoisingMethod::OpenImageDenoise => {
                return Err(
                    "Open Image Denoise is unavailable on Android Vulkan; select Auto or À-trous"
                        .into(),
                );
            }
            shrimply_scene_3d::DenoisingMethod::Atrous
            | shrimply_scene_3d::DenoisingMethod::Off => false,
            _ => return Err(
                "Selected denoiser is unavailable on this Vulkan device; select Auto or À-trous"
                    .into(),
            ),
        };
        let region_outline = plan.params.toon_outline_mode != shader::OutlineMode::Off
            && plan.params.toon_outline_method == shader::OutlineMethod::RegionBoundary;
        let guide = if region_outline {
            self.allocate(float_bytes)?
        } else {
            fallback.clone()
        };
        let distance = if region_outline {
            self.allocate(
                count
                    .checked_mul(size_of::<[f32; 2]>())
                    .ok_or("3D outline size overflow")?,
            )?
        } else {
            fallback.clone()
        };
        let mut uniforms = plan.uniforms;
        let denoise_buffer = || {
            if denoising {
                self.buffer(float_bytes, false, oidn)
            } else {
                Ok(fallback.clone())
            }
        };
        let beauty = denoise_buffer()?;
        let denoise_background = denoise_buffer()?;
        let alpha = denoise_buffer()?;
        let depth_normal = denoise_buffer()?;
        let filtered = denoise_buffer()?;
        let filtered_background = denoise_buffer()?;
        let albedo = if oidn {
            denoise_buffer()?
        } else {
            fallback.clone()
        };
        uniforms.pbr.denoising = u32::from(denoising);
        uniforms.transmission_background[1] = u32::from(background.is_some()) as f32;
        let scene = self.upload(&pack_scene_uniforms(&uniforms))?;
        let uploaded = self.object.as_ref().expect("uploaded 3D geometry");
        let state = SoftwareResources {
            scene_data: scene.address(),
            output_size_data: OutputSize {
                width: plan.width,
                height: plan.height,
                denoising: u32::from(denoising),
                guides: u32::from(oidn),
            },
            positions_data: uploaded.positions.address(),
            normals_data: uploaded.normals.address(),
            tangents_data: uploaded.tangents.address(),
            tex_coords_0_data: uploaded.tex_coords_0.address(),
            tex_coords_1_data: uploaded.tex_coords_1.address(),
            vertex_colors_data: uploaded.colors.address(),
            materials_data: uploaded.materials.address(),
            mesh_instances_data: uploaded.instances.address(),
            bvh_nodes: uploaded.nodes.address(),
            bvh_triangles: uploaded.triangles.address(),
            material_texture_data: uploaded.material_texture,
            environment_texture_data: environment_texture,
            composite_texture_data: composite_texture,
            output_pixels_data: output.address(),
            outline_guide_data: guide.address(),
            outline_distance_data: distance.address(),
            denoiser_beauty_data: beauty.address(),
            denoiser_background_data: denoise_background.address(),
            denoiser_alpha_data: alpha.address(),
            denoiser_depth_normal_data: depth_normal.address(),
            denoiser_albedo_data: albedo.address(),
            denoised_texture_data: filtered.address(),
            denoised_background_texture_data: filtered_background.address(),
            denoised_alpha_texture_data: alpha.address(),
            transport_work_data: 0,
            sample_work_data: 0,
            accumulation_data: 0,
            pass_count_data: 0,
            wavefront_origin: [0; 2],
            wavefront_extent: [0; 2],
        };
        let data = state;
        let state = self.upload(bytes(std::slice::from_ref(&data)))?;
        let mut retained = [
            &uploaded.positions,
            &uploaded.normals,
            &uploaded.tangents,
            &uploaded.tex_coords_0,
            &uploaded.tex_coords_1,
            &uploaded.colors,
            &uploaded.materials,
            &uploaded.instances,
            &uploaded.nodes,
            &uploaded.triangles,
            &uploaded.texture,
        ]
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
        retained.extend([
            scene,
            fallback,
            output.clone(),
            guide,
            distance,
            composite_buffer,
            environment_buffer,
            state.clone(),
            beauty.clone(),
            denoise_background.clone(),
            alpha,
            depth_normal.clone(),
            filtered.clone(),
            filtered_background.clone(),
            albedo.clone(),
        ]);
        let transport = plan.params.path_tracing != shader::PathTracingMode::Off
            || plan.params.transmission > 0.0
            || plan
                .session
                .materials()
                .iter()
                .any(|material| material.flags[3] != 0 && material.pbr.transmission > 0.0);
        let specialization = plan.params.shading_model as u32
            | if transport { TRANSPORT } else { 0 }
            | if plan.params.grounds.is_empty() {
                0
            } else {
                GROUNDS
            }
            | if plan
                .params
                .grounds
                .iter()
                .any(|ground| ground.reflection > 0.0 && ground.opacity > 0.0)
            {
                GROUND_REFLECTIONS
            } else {
                0
            };
        if self.device.word_storage {
            self.object_wavefront(plan, data, &retained, specialization, submissions)?;
        } else {
            let mut args = self.arguments("obj_compute")?;
            args.specialization = specialization;
            args.set("data", &state.address().to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    retained.clone(),
                    [plan.width as usize, plan.height as usize, 1],
                )
            }?);
        }
        for name in if region_outline {
            &["obj_outline_distance_compute", "obj_outline_compute"][..]
        } else {
            &[][..]
        } {
            let mut args = self.arguments(name)?;
            args.specialization = specialization;
            args.set("data", &state.address().to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    retained.clone(),
                    [plan.width as usize, plan.height as usize, 1],
                )
            }?);
        }
        if denoising {
            #[cfg(target_os = "linux")]
            if oidn {
                self.denoise_oidn(
                    super::oidn::Guides {
                        colors: [
                            (&beauty, &filtered),
                            (&denoise_background, &filtered_background),
                        ],
                        albedo: &albedo,
                        depth_normal: &depth_normal,
                        size: (plan.width, plan.height),
                        quality: match plan.params.render_quality {
                            shader::RenderQuality::Interactive => {
                                shrimply_denoise_oidn::Quality::Interactive
                            }
                            shader::RenderQuality::Final => shrimply_denoise_oidn::Quality::Final,
                        },
                    },
                    submissions,
                )?;
            }
            if !oidn {
                for (input, output) in [
                    (&beauty, &filtered),
                    (&denoise_background, &filtered_background),
                ] {
                    self.denoise_atrous(
                        input,
                        output,
                        &depth_normal,
                        (plan.width, plan.height),
                        submissions,
                    )?;
                }
            }
            let mut args = self.arguments("obj_denoise_composite_compute")?;
            args.specialization = specialization;
            args.set("data", &state.address().to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    retained,
                    [plan.width as usize, plan.height as usize, 1],
                )
            }?);
        }
        Ok(output)
    }

    fn object_wavefront(
        &mut self,
        plan: &PreparedFrame,
        mut data: SoftwareResources,
        retained: &[Buffer],
        options: u32,
        submissions: &mut Vec<Submission>,
    ) -> Result<(), String> {
        let uniforms = &plan.uniforms;
        let key = SamplingKey {
            anti_aliasing: uniforms.toon.anti_aliasing as u32,
            path_tracing: uniforms.pbr.path_tracing as u32,
            quality: uniforms.pbr.render_quality as u32,
            depth_of_field: uniforms.camera_lens[1] > 0.0,
            transmission: uniforms.pbr.transmission > 0.0,
            light_sampling: uniforms.pbr.light_sampling_quality as u32,
            options,
        };
        if self
            .object_sampling
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            let budget = self.allocate(2 * size_of::<u32>())?;
            data.pass_count_data = budget.address();
            let state = self.upload(bytes(std::slice::from_ref(&data)))?;
            let mut resources = retained.to_vec();
            resources.extend([budget.clone(), state.clone()]);
            let mut args = self.arguments("obj_sample_budget_compute")?;
            args.specialization = options;
            args.set("data", &state.address().to_ne_bytes())?;
            let _submission = unsafe { self.dispatch(args, resources, [1, 1, 1]) }?;
            let bytes = self.readback(&budget)?;
            let samples = u32::from_le_bytes(bytes[..4].try_into().expect("sample budget word"));
            let transport_steps =
                u32::from_le_bytes(bytes[4..8].try_into().expect("transport budget word"));
            if samples == 0 || transport_steps == 0 {
                return Err("GPU returned an empty object sampling budget".into());
            }
            self.object_sampling = Some(SamplingCache {
                key,
                samples,
                transport_steps,
            });
        }
        let budget = self
            .object_sampling
            .as_ref()
            .expect("resolved sampling budget");
        let samples = budget.samples;
        let transport_steps = budget.transport_steps;
        let bytes_per_ray = TRANSPORT_BYTES
            .checked_add(SAMPLE_BYTES)
            .and_then(|value| value.checked_add(ACCUMULATION_BYTES))
            .ok_or("object scratch size overflow")?;
        let scratch_limit =
            MAX_OBJECT_SCRATCH_BYTES.min(self.device.limits.max_storage_buffer_range as usize);
        let max_rays = scratch_limit / bytes_per_ray;
        let tile =
            shrimply_render_core::math::bounded_tile_extent((plan.width, plan.height), max_rays)
                .ok_or("GPU cannot allocate object scratch")?;
        let rays = tile.0 as usize * tile.1 as usize;
        let transport = self.allocate(rays * TRANSPORT_BYTES)?;
        let sample = self.allocate(rays * SAMPLE_BYTES)?;
        let accumulation = self.allocate(rays * ACCUMULATION_BYTES)?;
        data.transport_work_data = transport.address();
        data.sample_work_data = sample.address();
        data.accumulation_data = accumulation.address();
        data.pass_count_data = 0;
        let mut resources = retained.to_vec();
        resources.extend([transport, sample, accumulation]);
        let mut cycle = vec!["obj_surface_prepare_compute"];
        if plan.params.shading_model != shader::ShadingModel::Depth {
            cycle.extend([
                "obj_surface_direct_compute",
                "obj_surface_environment_compute",
            ]);
            if plan.params.shading_model == shader::ShadingModel::Pbr && options & TRANSPORT != 0 {
                cycle.extend([
                    "obj_surface_reservoir_compute",
                    "obj_surface_visibility_compute",
                ]);
            }
            if plan.params.shading_model == shader::ShadingModel::Toon {
                cycle.push("obj_surface_toon_finish_compute");
            }
        }
        cycle.push("obj_surface_finish_compute");
        if options & TRANSPORT != 0 {
            cycle.extend([
                "obj_transport_scatter_compute",
                "obj_transport_trace_compute",
            ]);
            if options & GROUNDS != 0 {
                cycle.extend([
                    "obj_ground_prepare_compute",
                    "obj_ground_shadow_compute",
                    "obj_ground_push_compute",
                ]);
                if options & GROUND_REFLECTIONS != 0 {
                    cycle.push("obj_transport_trace_compute");
                }
            }
            cycle.extend(["obj_transport_miss_compute", "obj_transport_unwind_compute"]);
        } else {
            cycle.extend([
                "obj_transport_scatter_compute",
                "obj_transport_unwind_compute",
            ]);
        }
        let mut schedule = vec!["obj_sample_init_compute"];
        for _ in 0..samples {
            schedule.push("obj_sample_begin_compute");
            for _ in 0..transport_steps {
                schedule.extend_from_slice(&cycle);
            }
            schedule.push("obj_sample_primary_finish_compute");
            if options & GROUNDS != 0 {
                schedule.extend([
                    "obj_sample_ground_begin_compute",
                    "obj_ground_shadow_compute",
                    "obj_sample_reflection_begin_compute",
                ]);
                if options & GROUND_REFLECTIONS != 0 {
                    schedule.push("obj_transport_trace_compute");
                    for _ in 0..transport_steps {
                        schedule.extend_from_slice(&cycle);
                    }
                }
                schedule.push("obj_sample_ground_finish_compute");
            }
            schedule.push("obj_sample_accumulate_compute");
        }
        schedule.push("obj_sample_finalize_compute");
        let mut names = schedule.clone();
        names.sort_unstable();
        names.dedup();
        for y in (0..plan.height).step_by(tile.1 as usize) {
            for x in (0..plan.width).step_by(tile.0 as usize) {
                data.wavefront_origin = [x, y];
                data.wavefront_extent = [tile.0.min(plan.width - x), tile.1.min(plan.height - y)];
                let state = self.upload(bytes(std::slice::from_ref(&data)))?;
                let arguments = names
                    .iter()
                    .map(|name| {
                        let mut args = self.arguments(name)?;
                        args.specialization = options;
                        args.set("data", &state.address().to_ne_bytes())?;
                        Ok(args)
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let stages = schedule.iter().map(|name| {
                    &arguments[names.binary_search(name).expect("prepared object stage")]
                });
                let mut retained = resources.clone();
                retained.push(state);
                submissions.push(unsafe {
                    self.dispatch_batch(
                        stages,
                        retained,
                        [
                            data.wavefront_extent[0] as usize,
                            data.wavefront_extent[1] as usize,
                            1,
                        ],
                    )
                }?);
            }
        }
        Ok(())
    }

    fn upload_object(&mut self, plan: &PreparedFrame) -> Result<(), String> {
        if self
            .object
            .as_ref()
            .is_some_and(|uploaded| &uploaded.identity == plan.session.identity())
        {
            return Ok(());
        }
        let session = &plan.session;
        let bvh = shrimply_render_3d_core::software_bvh(session)?;
        let materials = session
            .materials()
            .iter()
            .map(ComputeMaterial::from)
            .collect::<Vec<_>>();
        let colors = session
            .colors()
            .iter()
            .map(|color| color.to_array())
            .collect::<Vec<_>>();
        let atlas = session.texture_atlas();
        let pixels = atlas
            .pixels
            .iter()
            .map(|color| {
                color
                    .to_array()
                    .map(|channel| f32::from(channel) / f32::from(u8::MAX))
            })
            .collect::<Vec<_>>();
        let (material_texture, texture) =
            self.float_texture((atlas.width, atlas.height), Some(&pixels))?;
        // Static source textures are uploaded once; mip data is produced on the GPU.
        let mut submissions = Vec::new();
        self.float_mips(&texture, (atlas.width, atlas.height), &mut submissions)?;
        if let Some(last) = submissions.last() {
            last.wait()?;
        }
        self.object = Some(Uploaded {
            identity: session.identity().clone(),
            positions: self.upload(bytes(session.positions()))?,
            normals: self.upload(bytes(session.normals()))?,
            tangents: self.upload(bytes(session.tangents()))?,
            tex_coords_0: self.upload(bytes(session.tex_coords_0()))?,
            tex_coords_1: self.upload(bytes(session.tex_coords_1()))?,
            colors: self.upload(bytes(&colors))?,
            materials: self.upload(bytes(&materials))?,
            instances: self.upload(bytes(session.mesh_instances()))?,
            nodes: self.upload(bytes(&bvh.nodes))?,
            triangles: self.upload(bytes(&bvh.triangles))?,
            texture,
            material_texture,
        });
        Ok(())
    }

    fn object_environment(
        &mut self,
        plan: &PreparedFrame,
        empty: SoftwareTexture,
        fallback: &Buffer,
        submissions: &mut Vec<Submission>,
    ) -> Result<(SoftwareTexture, Buffer), String> {
        let Some(identity) = plan.environment.as_ref() else {
            self.object_environment = None;
            return Ok((empty, fallback.clone()));
        };
        identity.verify_current()?;
        if !self
            .object_environment
            .as_ref()
            .is_some_and(|environment| &environment.identity == identity)
        {
            let decoded = shrimply_render_3d_core::load_environment(identity.path())
                .map_err(|error| error.to_string())?;
            identity.verify_current()?;
            let pixels = decoded
                .pixels
                .iter()
                .map(|color| color.to_array())
                .collect::<Vec<_>>();
            let (texture, buffer) =
                self.float_texture((decoded.width, decoded.height), Some(&pixels))?;
            self.float_mips(&buffer, (decoded.width, decoded.height), submissions)?;
            self.object_environment = Some(Environment {
                identity: identity.clone(),
                texture,
                buffer,
            });
        }
        let environment = self
            .object_environment
            .as_ref()
            .expect("uploaded 3D environment");
        Ok((environment.texture, environment.buffer.clone()))
    }

    fn float_texture(
        &self,
        size: (u32, u32),
        pixels: Option<&[[f32; 4]]>,
    ) -> Result<(SoftwareTexture, Buffer), String> {
        let (mut width, mut height) = size;
        if width == 0 || height == 0 {
            return Err("3D texture dimensions must be nonzero".into());
        }
        let mut count = 0_usize;
        let mut levels = 0;
        loop {
            count = count
                .checked_add(
                    (width as usize)
                        .checked_mul(height as usize)
                        .ok_or("3D mip size overflow")?,
                )
                .ok_or("3D texture size overflow")?;
            levels += 1;
            if width == 1 && height == 1 {
                break;
            }
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }
        let length = count
            .checked_mul(size_of::<[f32; 4]>())
            .ok_or("3D texture size overflow")?;
        let buffer = if let Some(pixels) = pixels {
            if (size.0 as usize).checked_mul(size.1 as usize) != Some(pixels.len()) {
                return Err("3D texture pixels do not match its dimensions".into());
            }
            let mut data = vec![0; length];
            data[..size_of_val(pixels)].copy_from_slice(bytes(pixels));
            self.upload(&data)?
        } else {
            self.allocate(length)?
        };
        Ok((
            SoftwareTexture {
                pixels: buffer.address(),
                width: size.0,
                height: size.1,
                levels,
                padding: 0,
            },
            buffer,
        ))
    }

    fn float_mips(
        &mut self,
        buffer: &Buffer,
        size: (u32, u32),
        submissions: &mut Vec<Submission>,
    ) -> Result<(), String> {
        let (mut width, mut height) = size;
        let mut offset = 0_usize;
        while width > 1 || height > 1 {
            let output_width = (width / 2).max(1);
            let output_height = (height / 2).max(1);
            let next = offset + width as usize * height as usize * size_of::<[f32; 4]>();
            let mut args = self.arguments("float_mip")?;
            args.set("input", &(buffer.address() + offset as u64).to_ne_bytes())?
                .set("output", &(buffer.address() + next as u64).to_ne_bytes())?
                .set("width", &width.to_ne_bytes())?
                .set("height", &height.to_ne_bytes())?
                .set("output_width", &output_width.to_ne_bytes())?
                .set("output_height", &output_height.to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    vec![buffer.clone()],
                    [output_width as usize * output_height as usize, 1, 1],
                )
            }?);
            offset = next;
            width = output_width;
            height = output_height;
        }
        Ok(())
    }
}

// All callers pass the reflected, padding-free ABI structs or scalar/vector slices.
fn bytes<T>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), size_of_val(values)) }
}
