use std::{error::Error, fmt, path::Path};

use glam::{Mat3, Mat4, Vec2, Vec3};
use gltf::{
    image::{Data as ImageData, Format},
    material::AlphaMode,
    mesh::Mode,
    texture::{MagFilter, WrappingMode},
};
use rayon::prelude::*;
use shrimply_math_color::Color;

use crate::obj::{MeshMaterial, ObjMesh, TextureAtlas, TextureMapping};

const TEXTURE_GUTTER: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlbError(String);

impl fmt::Display for GlbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for GlbError {}

pub fn load_glb(path: impl AsRef<Path>) -> Result<ObjMesh, GlbError> {
    let path = path.as_ref();
    let (document, buffers, images) = gltf::import(path)
        .map_err(|error| GlbError(format!("failed to import {}: {error}", path.display())))?;
    let (texture_atlas, atlas_regions) = pack_images(&images)?;
    let mut mesh = ObjMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        tex_coords_0: Vec::new(),
        tex_coords_1: Vec::new(),
        colors: Vec::new(),
        materials: Vec::new(),
        texture_atlas,
        face_normals: Vec::new(),
        triangles: Vec::new(),
        source_center: Vec3::ZERO,
        source_radius: 0.0,
    };
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or_else(|| GlbError("GLB contains no scene".to_string()))?;
    for node in scene.nodes() {
        append_node(node, Mat4::IDENTITY, &buffers, &atlas_regions, &mut mesh)?;
    }
    if mesh.triangles.is_empty() {
        return Err(GlbError("GLB contains no triangle primitives".to_string()));
    }
    normalize_positions(&mut mesh)?;
    Ok(mesh)
}

fn append_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    atlas_regions: &[[f32; 4]],
    output: &mut ObjMesh,
) -> Result<(), GlbError> {
    let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            append_primitive(primitive, transform, buffers, atlas_regions, output)?;
        }
    }
    for child in node.children() {
        append_node(child, transform, buffers, atlas_regions, output)?;
    }
    Ok(())
}

fn append_primitive(
    primitive: gltf::Primitive<'_>,
    transform: Mat4,
    buffers: &[gltf::buffer::Data],
    atlas_regions: &[[f32; 4]],
    output: &mut ObjMesh,
) -> Result<(), GlbError> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()].0));
    let positions: Vec<_> = reader
        .read_positions()
        .ok_or_else(|| GlbError("GLB primitive is missing POSITION data".to_string()))?
        .collect();
    let indices: Vec<_> = reader
        .read_indices()
        .map(|indices| indices.into_u32().collect())
        .unwrap_or_else(|| (0..positions.len() as u32).collect());
    let triangles = triangle_indices(primitive.mode(), &indices)?;
    let normals = reader.read_normals().map(Iterator::collect::<Vec<_>>);
    let tangents = reader.read_tangents().map(Iterator::collect::<Vec<_>>);
    let tex_coords_0 = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect::<Vec<_>>());
    let tex_coords_1 = reader
        .read_tex_coords(1)
        .map(|coords| coords.into_f32().collect::<Vec<_>>());
    let colors = reader
        .read_colors(0)
        .map(|colors| colors.into_rgba_f32().collect::<Vec<_>>());
    let material = material(primitive.material(), atlas_regions)?;
    let linear = Mat3::from_mat4(transform);
    let determinant = linear.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return Err(GlbError("GLB node has a singular transform".to_string()));
    }
    let normal_transform = linear.inverse().transpose();

    for triangle in triangles {
        let first = u32::try_from(output.positions.len())
            .map_err(|_| GlbError("GLB vertex count exceeds GPU limits".to_string()))?;
        let source = triangle.map(|index| {
            usize::try_from(index)
                .ok()
                .filter(|index| *index < positions.len())
                .ok_or_else(|| GlbError(format!("GLB index {index} is out of range")))
        });
        let source = [source[0].clone()?, source[1].clone()?, source[2].clone()?];
        let transformed =
            source.map(|index| transform.transform_point3(Vec3::from(positions[index])));
        let face_normal = (transformed[1] - transformed[0])
            .cross(transformed[2] - transformed[0])
            .try_normalize();
        let Some(face_normal) = face_normal else {
            continue;
        };
        let generated_tangent = triangle_tangent(
            transformed,
            source.map(|index| {
                tex_coords_0
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .copied()
                    .map(Vec2::from)
                    .unwrap_or(Vec2::ZERO)
            }),
            face_normal,
        );
        for corner in 0..3 {
            let index = source[corner];
            let normal = normals
                .as_ref()
                .and_then(|values| values.get(index))
                .map(|normal| normal_transform * Vec3::from(*normal))
                .and_then(Vec3::try_normalize)
                .unwrap_or(face_normal);
            let tangent = tangents
                .as_ref()
                .and_then(|values| values.get(index))
                .map(|tangent| {
                    let direction = (linear * Vec3::from_array(tangent[..3].try_into().unwrap()))
                        .normalize_or_zero();
                    [
                        direction.x,
                        direction.y,
                        direction.z,
                        tangent[3] * determinant.signum(),
                    ]
                })
                .unwrap_or(generated_tangent);
            output
                .positions
                .push(transformed[corner].extend(1.0).to_array());
            output.normals.push(normal.extend(0.0).to_array());
            output.tangents.push(tangent);
            output.tex_coords_0.push(
                tex_coords_0
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .map(|uv| [uv[0], uv[1], 0.0, 0.0])
                    .unwrap_or([0.0; 4]),
            );
            output.tex_coords_1.push(
                tex_coords_1
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .map(|uv| [uv[0], uv[1], 0.0, 0.0])
                    .unwrap_or([0.0; 4]),
            );
            output.colors.push(
                colors
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .copied()
                    .map(Color::from)
                    .unwrap_or(Color::WHITE),
            );
        }
        output.face_normals.push(face_normal.extend(0.0).to_array());
        output.triangles.push([first, first + 1, first + 2, 0]);
        output.materials.push(material);
    }
    Ok(())
}

fn triangle_tangent(positions: [Vec3; 3], uv: [Vec2; 3], normal: Vec3) -> [f32; 4] {
    let edge_0 = positions[1] - positions[0];
    let edge_1 = positions[2] - positions[0];
    let delta_0 = uv[1] - uv[0];
    let delta_1 = uv[2] - uv[0];
    let determinant = delta_0.perp_dot(delta_1);
    let tangent = if determinant.abs() > f32::EPSILON {
        (edge_0 * delta_1.y - edge_1 * delta_0.y) / determinant
    } else {
        normal.any_orthonormal_vector()
    }
    .normalize_or_zero();
    let bitangent = if determinant.abs() > f32::EPSILON {
        (edge_1 * delta_0.x - edge_0 * delta_1.x) / determinant
    } else {
        normal.cross(tangent)
    };
    [
        tangent.x,
        tangent.y,
        tangent.z,
        normal.cross(tangent).dot(bitangent).signum(),
    ]
}

fn triangle_indices(mode: Mode, indices: &[u32]) -> Result<Vec<[u32; 3]>, GlbError> {
    let triangles = match mode {
        Mode::Triangles => indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect(),
        Mode::TriangleStrip => (2..indices.len())
            .map(|index| {
                if index % 2 == 0 {
                    [indices[index - 2], indices[index - 1], indices[index]]
                } else {
                    [indices[index - 1], indices[index - 2], indices[index]]
                }
            })
            .collect(),
        Mode::TriangleFan => (2..indices.len())
            .map(|index| [indices[0], indices[index - 1], indices[index]])
            .collect(),
        _ => {
            return Err(GlbError(format!(
                "unsupported GLB primitive mode {mode:?}; expected triangles"
            )));
        }
    };
    Ok(triangles)
}

fn material(
    material: gltf::Material<'_>,
    atlas_regions: &[[f32; 4]],
) -> Result<MeshMaterial, GlbError> {
    let pbr = material.pbr_metallic_roughness();
    let base_color_texture = pbr
        .base_color_texture()
        .map(|info| texture_mapping(&info, atlas_regions))
        .transpose()?
        .unwrap_or_default();
    let metallic_roughness_texture = pbr
        .metallic_roughness_texture()
        .map(|info| texture_mapping(&info, atlas_regions))
        .transpose()?
        .unwrap_or_default();
    let normal_texture = material
        .normal_texture()
        .map(|info| {
            texture_mapping_parts(
                info.texture(),
                info.tex_coord(),
                Vec2::ZERO,
                Vec2::ONE,
                0.0,
                atlas_regions,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let normal_scale = material
        .normal_texture()
        .map(|texture| texture.scale())
        .unwrap_or(1.0);
    let alpha_cutoff = matches!(material.alpha_mode(), AlphaMode::Mask)
        .then(|| material.alpha_cutoff().unwrap_or(0.5))
        .unwrap_or(0.0);
    Ok(MeshMaterial {
        base_color_factor: Color::from(pbr.base_color_factor()),
        metallic_roughness_normal_alpha: [
            pbr.metallic_factor(),
            pbr.roughness_factor(),
            normal_scale,
            alpha_cutoff,
        ],
        flags: [
            1,
            match material.alpha_mode() {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask => 1,
                AlphaMode::Blend => 2,
            },
            material.double_sided() as u32,
            0,
        ],
        base_color_texture,
        metallic_roughness_texture,
        normal_texture,
    })
}

fn texture_mapping(
    info: &gltf::texture::Info<'_>,
    atlas_regions: &[[f32; 4]],
) -> Result<TextureMapping, GlbError> {
    let transform = info.texture_transform();
    texture_mapping_parts(
        info.texture(),
        transform
            .as_ref()
            .and_then(|transform| transform.tex_coord())
            .unwrap_or_else(|| info.tex_coord()),
        transform
            .as_ref()
            .map(|value| value.offset())
            .unwrap_or([0.0; 2])
            .into(),
        transform
            .as_ref()
            .map(|value| value.scale())
            .unwrap_or([1.0; 2])
            .into(),
        transform
            .as_ref()
            .map(|value| value.rotation())
            .unwrap_or(0.0),
        atlas_regions,
    )
}

fn texture_mapping_parts(
    texture: gltf::Texture<'_>,
    tex_coord: u32,
    offset: Vec2,
    scale: Vec2,
    rotation: f32,
    atlas_regions: &[[f32; 4]],
) -> Result<TextureMapping, GlbError> {
    if tex_coord > 1 {
        return Err(GlbError(format!(
            "GLB texture uses unsupported TEXCOORD_{tex_coord}"
        )));
    }
    let image = texture.source().index();
    let atlas = *atlas_regions
        .get(image)
        .ok_or_else(|| GlbError(format!("GLB texture image {image} is missing")))?;
    let sampler = texture.sampler();
    let nearest = matches!(sampler.mag_filter(), Some(MagFilter::Nearest));
    Ok(TextureMapping {
        atlas,
        transform: [offset.x, offset.y, scale.x, scale.y],
        settings: [
            1,
            tex_coord,
            wrapping(sampler.wrap_s()),
            wrapping(sampler.wrap_t()),
        ],
        rotation_filter: [rotation, nearest as u32 as f32, 0.0, 0.0],
    })
}

fn wrapping(mode: WrappingMode) -> u32 {
    match mode {
        WrappingMode::ClampToEdge => 0,
        WrappingMode::MirroredRepeat => 1,
        WrappingMode::Repeat => 2,
    }
}

fn normalize_positions(mesh: &mut ObjMesh) -> Result<(), GlbError> {
    let first = Vec3::from_array(mesh.positions[0][..3].try_into().unwrap());
    let (minimum, maximum) =
        mesh.positions
            .iter()
            .skip(1)
            .fold((first, first), |(minimum, maximum), position| {
                let position = Vec3::new(position[0], position[1], position[2]);
                (minimum.min(position), maximum.max(position))
            });
    let center = (minimum + maximum) * 0.5;
    let radius = mesh
        .positions
        .iter()
        .map(|position| Vec3::new(position[0], position[1], position[2]).distance(center))
        .fold(0.0_f32, f32::max);
    if !radius.is_finite() || radius <= 0.0 {
        return Err(GlbError(
            "GLB has a zero-radius bounding sphere".to_string(),
        ));
    }
    for position in &mut mesh.positions {
        *position = ((Vec3::new(position[0], position[1], position[2]) - center) / radius)
            .extend(1.0)
            .to_array();
    }
    mesh.source_center = center;
    mesh.source_radius = radius;
    Ok(())
}

fn pack_images(images: &[ImageData]) -> Result<(TextureAtlas, Vec<[f32; 4]>), GlbError> {
    if images.is_empty() {
        return Ok((TextureAtlas::default(), Vec::new()));
    }
    let decoded: Vec<_> = images
        .par_iter()
        .map(rgba_pixels)
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<Result<_, _>>()?;
    let area = decoded
        .iter()
        .try_fold(0_u64, |area, (_, width, height)| {
            area.checked_add(
                u64::from(width + 2 * TEXTURE_GUTTER) * u64::from(height + 2 * TEXTURE_GUTTER),
            )
        })
        .ok_or_else(|| GlbError("GLB texture atlas dimensions overflow".to_string()))?;
    let widest = decoded
        .iter()
        .map(|(_, width, _)| width + 2 * TEXTURE_GUTTER)
        .max()
        .unwrap();
    let target_width = widest.max((area as f64).sqrt().ceil() as u32);
    let mut placements = Vec::with_capacity(decoded.len());
    let (mut x, mut y, mut row_height, mut used_width) = (0_u32, 0_u32, 0_u32, 0_u32);
    for (_, width, height) in &decoded {
        let padded_width = width + 2 * TEXTURE_GUTTER;
        let padded_height = height + 2 * TEXTURE_GUTTER;
        if x > 0 && x + padded_width > target_width {
            y += row_height;
            x = 0;
            row_height = 0;
        }
        placements.push((x + TEXTURE_GUTTER, y + TEXTURE_GUTTER));
        x += padded_width;
        used_width = used_width.max(x);
        row_height = row_height.max(padded_height);
    }
    let width = used_width.max(1);
    let height = (y + row_height).max(1);
    let pixel_count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| GlbError("GLB texture atlas is too large".to_string()))?;
    let mut pixels = vec![Color::TRANSPARENT; pixel_count];
    let mut regions = Vec::with_capacity(decoded.len());
    for ((source, image_width, image_height), (left, top)) in decoded.iter().zip(placements) {
        for target_y in top - TEXTURE_GUTTER..=top + image_height {
            for target_x in left - TEXTURE_GUTTER..=left + image_width {
                let source_x = target_x.saturating_sub(left).min(image_width - 1);
                let source_y = target_y.saturating_sub(top).min(image_height - 1);
                pixels[(target_y * width + target_x) as usize] =
                    source[(source_y * image_width + source_x) as usize];
            }
        }
        regions.push([
            left as f32 / width as f32,
            top as f32 / height as f32,
            *image_width as f32 / width as f32,
            *image_height as f32 / height as f32,
        ]);
    }
    Ok((
        TextureAtlas {
            pixels,
            width,
            height,
        },
        regions,
    ))
}

fn rgba_pixels(image: &ImageData) -> Result<(Vec<Color<u8>>, u32, u32), GlbError> {
    let count = usize::try_from(u64::from(image.width) * u64::from(image.height))
        .map_err(|_| GlbError("GLB image dimensions overflow".to_string()))?;
    let mut pixels = Vec::with_capacity(count);
    match image.format {
        Format::R8 => image
            .pixels
            .iter()
            .for_each(|value| pixels.push(Color::from_rgb(*value, *value, *value))),
        Format::R8G8 => image
            .pixels
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|value| pixels.push(Color::new(value[0], value[0], value[0], value[1]))),
        Format::R8G8B8 => image
            .pixels
            .as_chunks::<3>()
            .0
            .iter()
            .for_each(|value| pixels.push(Color::from_rgb(value[0], value[1], value[2]))),
        Format::R8G8B8A8 => image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .for_each(|value| pixels.push(Color::new(value[0], value[1], value[2], value[3]))),
        other => return Err(GlbError(format!("unsupported GLB image format {other:?}"))),
    }
    if pixels.len() != count {
        return Err(GlbError(
            "GLB image has an invalid pixel payload".to_string(),
        ));
    }
    Ok((pixels, image.width, image.height))
}
