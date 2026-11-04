use crate::{Buffer, Renderer, Submission};
use shrimply_asset::AssetSnapshot;
use shrimply_render_3d_core::{PreparedFrame, SceneIdentity, SoftwareBvhNode, obj as shader};

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
struct OutputSize {
    width: u32,
    height: u32,
    denoising: u32,
    padding: u32,
}
#[repr(C)]
struct ComputePbr {
    modes: [u32; 4],
    surface: [f32; 4],
    ior: f32,
    padding: [f32; 3],
}
#[repr(C)]
struct ComputeMaterial {
    base_color: [f32; 4],
    metallic_roughness_normal_alpha: [f32; 4],
    flags: [u32; 4],
    base_color_texture: shader::TextureMapping,
    metallic_roughness_texture: shader::TextureMapping,
    normal_texture: shader::TextureMapping,
    surface: [u32; 4],
    pbr: ComputePbr,
}
#[repr(C)]
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
    denoised_texture_data: u64,
    denoised_background_texture_data: u64,
    denoised_alpha_texture_data: u64,
}
include!(concat!(env!("OUT_DIR"), "/software_abi.rs"));
include!(concat!(env!("OUT_DIR"), "/software_uniforms.rs"));

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
            let mut args = self.arguments("rgba_to_float")?;
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
        let denoising =
            plan.params.denoising && plan.params.shading_model == shader::ShadingModel::Pbr;
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
        let mut denoiser = Vec::new();
        for _ in 0..7 {
            denoiser.push(if denoising {
                self.allocate(float_bytes)?
            } else {
                fallback.clone()
            });
        }
        let mut uniforms = plan.uniforms;
        uniforms.transmission_background[1] = u32::from(background.is_some()) as f32;
        let scene = self.upload(&pack_scene_uniforms(&uniforms))?;
        let uploaded = self.object.as_ref().expect("uploaded 3D geometry");
        let state = SoftwareResources {
            scene_data: scene.address(),
            output_size_data: OutputSize {
                width: plan.width,
                height: plan.height,
                denoising: u32::from(denoising),
                padding: 0,
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
            denoiser_beauty_data: denoiser[0].address(),
            denoiser_background_data: denoiser[1].address(),
            denoiser_alpha_data: denoiser[2].address(),
            denoiser_depth_normal_data: denoiser[3].address(),
            denoised_texture_data: denoiser[4].address(),
            denoised_background_texture_data: denoiser[5].address(),
            denoised_alpha_texture_data: denoiser[6].address(),
        };
        let state = self.upload(bytes(std::slice::from_ref(&state)))?;
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
        ]);
        retained.extend(denoiser);
        let mut kernels = vec!["obj_compute"];
        if region_outline {
            kernels.extend(["obj_outline_distance_compute", "obj_outline_compute"]);
        }
        if denoising {
            kernels.extend(["obj_software_denoise", "obj_denoise_composite_compute"]);
        }
        for name in kernels {
            let mut args = self.arguments(name)?;
            args.set("data", &state.address().to_ne_bytes())?;
            submissions.push(unsafe {
                self.dispatch(
                    args,
                    retained.clone(),
                    [plan.width as usize, plan.height as usize, 1],
                )
            }?);
        }
        Ok(output)
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
            .map(|material| ComputeMaterial {
                base_color: material.base_color_factor,
                metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                flags: material.flags,
                base_color_texture: material.base_color_texture,
                metallic_roughness_texture: material.metallic_roughness_texture,
                normal_texture: material.normal_texture,
                surface: material.surface,
                pbr: ComputePbr {
                    modes: [
                        material.pbr.path_tracing as u32,
                        material.pbr.light_sampling_quality as u32,
                        material.pbr.render_quality as u32,
                        material.pbr.denoising,
                    ],
                    surface: [
                        material.pbr.subsurface,
                        material.pbr.clearcoat,
                        material.pbr.sheen,
                        material.pbr.transmission,
                    ],
                    ior: material.pbr.ior,
                    padding: [0.0; 3],
                },
            })
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
