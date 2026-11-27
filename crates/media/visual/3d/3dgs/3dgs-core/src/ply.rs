use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use glam::Vec3;
use rayon::prelude::*;

pub(crate) const SH_COEFFICIENTS: usize = 16;
const SH_CHANNELS: usize = 3;
const COLOR_CHANNEL_MAXIMUM: f32 = u8::MAX as f32;

#[derive(Clone, Debug)]
pub struct Gaussian {
    pub position: Vec3,
    pub spherical_harmonics: [f32; SH_CHANNELS],
    pub opacity: f32,
    pub scale: Vec3,
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct GaussianCloud {
    pub gaussians: Vec<Gaussian>,
    pub higher_order_spherical_harmonics: Vec<f32>,
    pub source_center: Vec3,
    pub source_radius: f32,
    pub spherical_harmonic_degree: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlyError(String);

impl fmt::Display for PlyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlyError {}

#[derive(Clone, Copy)]
enum Format {
    Ascii,
    LittleEndian,
    BigEndian,
}

#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

#[derive(Clone)]
enum Property {
    Scalar { name: String, ty: Scalar },
    List { count: Scalar, item: Scalar },
}

struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
}

struct GaussianLayout {
    position: [usize; 3],
    spherical_harmonics: [[Option<usize>; SH_CHANNELS]; SH_COEFFICIENTS],
    opacity: usize,
    scale: [usize; 3],
    rotation: [usize; 4],
    spherical_harmonic_degree: u32,
}

struct PointCloudLayout {
    position: [usize; 3],
    color: [usize; 3],
}

enum VertexLayout {
    Gaussian(Box<GaussianLayout>),
    PointCloud(PointCloudLayout),
}

struct Point {
    position: Vec3,
    color: [f32; 3],
}

pub fn load_gaussian_ply(path: impl AsRef<Path>) -> Result<GaussianCloud, PlyError> {
    let path = path.as_ref();
    let file = File::open(path)
        .map_err(|error| PlyError(format!("failed to open {}: {error}", path.display())))?;
    parse_gaussian_ply(BufReader::new(file))
}

fn parse_gaussian_ply(mut reader: impl BufRead) -> Result<GaussianCloud, PlyError> {
    let (format, elements) = parse_header(&mut reader)?;
    let vertex = elements
        .iter()
        .find(|element| element.name == "vertex")
        .ok_or_else(|| PlyError("PLY contains no vertex element".into()))?;
    if vertex
        .properties
        .iter()
        .any(|property| matches!(property, Property::List { .. }))
    {
        return Err(PlyError(
            "3D Gaussian PLY vertex properties must all be scalar".into(),
        ));
    }
    let layout = VertexLayout::new(vertex)?;

    let mut gaussians = Vec::new();
    let mut points = Vec::new();
    match &layout {
        VertexLayout::Gaussian(_) => gaussians
            .try_reserve_exact(vertex.count)
            .map_err(|_| PlyError("unable to allocate Gaussian data".into()))?,
        VertexLayout::PointCloud(_) => points
            .try_reserve_exact(vertex.count)
            .map_err(|_| PlyError("unable to allocate point cloud data".into()))?,
    }
    let spherical_harmonic_degree = match &layout {
        VertexLayout::Gaussian(layout) => layout.spherical_harmonic_degree,
        VertexLayout::PointCloud(_) => 0,
    };
    let higher_order_count = ((spherical_harmonic_degree as usize + 1).pow(2) - 1)
        .checked_mul(SH_CHANNELS)
        .and_then(|count| count.checked_mul(vertex.count))
        .ok_or_else(|| PlyError("higher-order SH data is too large".into()))?;
    let mut higher_order_spherical_harmonics = Vec::new();
    higher_order_spherical_harmonics
        .try_reserve_exact(higher_order_count)
        .map_err(|_| PlyError("unable to allocate higher-order SH data".into()))?;
    match format {
        Format::Ascii => parse_ascii_body(
            &mut reader,
            &elements,
            &layout,
            &mut gaussians,
            &mut points,
            &mut higher_order_spherical_harmonics,
        )?,
        Format::LittleEndian | Format::BigEndian => parse_binary_body(
            &mut reader,
            &elements,
            format,
            &layout,
            &mut gaussians,
            &mut points,
            &mut higher_order_spherical_harmonics,
        )?,
    }
    if gaussians.is_empty() && points.is_empty() {
        return Err(PlyError("PLY contains no vertices".into()));
    }
    let (minimum, maximum) = gaussians
        .par_iter()
        .map(|gaussian| gaussian.position)
        .chain(points.par_iter().map(|point| point.position))
        .map(|position| (position, position))
        .reduce_with(|(minimum, maximum), (other_minimum, other_maximum)| {
            (minimum.min(other_minimum), maximum.max(other_maximum))
        })
        .expect("Gaussian PLY was checked for vertices");
    let source_center = (minimum + maximum) * 0.5;
    let source_radius = gaussians
        .par_iter()
        .map(|gaussian| gaussian.position)
        .chain(points.par_iter().map(|point| point.position))
        .map(|position| position.distance(source_center))
        .reduce(|| 0.0, f32::max);
    if !source_radius.is_finite() || source_radius <= 0.0 {
        return Err(PlyError("PLY has a zero-radius bounding sphere".into()));
    }
    if !points.is_empty() {
        let scale = crate::math::point_cloud_log_scale(source_radius, points.len());
        let opacity = crate::math::point_cloud_opacity_logit();
        gaussians = points
            .into_par_iter()
            .map(|point| Gaussian {
                position: point.position,
                spherical_harmonics: crate::math::point_cloud_sh_dc(point.color),
                opacity,
                scale: Vec3::splat(scale),
                rotation: [1.0, 0.0, 0.0, 0.0],
            })
            .collect();
    }
    Ok(GaussianCloud {
        gaussians,
        higher_order_spherical_harmonics,
        source_center,
        source_radius,
        spherical_harmonic_degree,
    })
}

impl VertexLayout {
    fn new(element: &Element) -> Result<Self, PlyError> {
        let gaussian_properties = element.properties.iter().any(|property| {
            matches!(property, Property::Scalar { name, .. }
                if name == "f_dc_0" || name == "scale_0" || name == "rot_0"
                    || name.starts_with("f_rest_"))
        });
        if gaussian_properties {
            GaussianLayout::new(element)
                .map(Box::new)
                .map(Self::Gaussian)
        } else {
            PointCloudLayout::new(element).map(Self::PointCloud)
        }
    }
}

impl PointCloudLayout {
    fn new(element: &Element) -> Result<Self, PlyError> {
        let index = |name: &str| {
            element.properties.iter().position(
                |property| matches!(property, Property::Scalar { name: candidate, .. } if candidate == name),
            )
        };
        let required = |name: &str| {
            index(name).ok_or_else(|| {
                PlyError(format!(
                    "PLY is neither 3D Gaussian Splatting data nor a colored point cloud: missing `{name}`"
                ))
            })
        };
        Ok(Self {
            position: [required("x")?, required("y")?, required("z")?],
            color: [required("red")?, required("green")?, required("blue")?],
        })
    }
}

impl GaussianLayout {
    fn new(element: &Element) -> Result<Self, PlyError> {
        let index = |name: &str| {
            element.properties.iter().position(
                |property| matches!(property, Property::Scalar { name: candidate, .. } if candidate == name),
            )
        };
        let required = |name: &str| {
            index(name).ok_or_else(|| {
                PlyError(format!(
                    "PLY is not a 3D Gaussian Splatting file: missing `{name}`"
                ))
            })
        };
        let rest_count = (0..SH_COEFFICIENTS.saturating_sub(1) * SH_CHANNELS)
            .take_while(|value| index(&format!("f_rest_{value}")).is_some())
            .count();
        let spherical_harmonic_degree = match rest_count {
            0 => 0,
            9 => 1,
            24 => 2,
            45 => 3,
            _ => {
                return Err(PlyError(format!(
                    "3D Gaussian PLY has {rest_count} contiguous higher-order SH values; expected 0, 9, 24, or 45"
                )));
            }
        };
        let mut spherical_harmonics = [[None; SH_CHANNELS]; SH_COEFFICIENTS];
        for (channel, value) in spherical_harmonics[0].iter_mut().enumerate() {
            *value = Some(required(&format!("f_dc_{channel}"))?);
        }
        let coefficients_per_channel = rest_count / SH_CHANNELS;
        for (coefficient, channels) in spherical_harmonics.iter_mut().enumerate().skip(1) {
            for (channel, value) in channels.iter_mut().enumerate() {
                if coefficient <= coefficients_per_channel {
                    *value = index(&format!(
                        "f_rest_{}",
                        channel * coefficients_per_channel + coefficient - 1
                    ));
                }
            }
        }
        Ok(Self {
            position: [required("x")?, required("y")?, required("z")?],
            spherical_harmonics,
            opacity: required("opacity")?,
            scale: [
                required("scale_0")?,
                required("scale_1")?,
                required("scale_2")?,
            ],
            rotation: [
                required("rot_0")?,
                required("rot_1")?,
                required("rot_2")?,
                required("rot_3")?,
            ],
            spherical_harmonic_degree,
        })
    }
}

fn parse_header(reader: &mut impl BufRead) -> Result<(Format, Vec<Element>), PlyError> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| PlyError(format!("read PLY header: {error}")))?;
    if line.trim_end() != "ply" {
        return Err(PlyError(
            "file does not start with the PLY signature".into(),
        ));
    }
    let mut format = None;
    let mut elements = Vec::<Element>::new();
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|error| PlyError(format!("read PLY header: {error}")))?
            == 0
        {
            return Err(PlyError("PLY header is missing `end_header`".into()));
        }
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else { continue };
        match kind {
            "comment" | "obj_info" => {}
            "format" => {
                format = Some(match fields.next() {
                    Some("ascii") => Format::Ascii,
                    Some("binary_little_endian") => Format::LittleEndian,
                    Some("binary_big_endian") => Format::BigEndian,
                    Some(value) => {
                        return Err(PlyError(format!("unsupported PLY format `{value}`")));
                    }
                    None => return Err(PlyError("missing PLY format".into())),
                });
                if fields.next() != Some("1.0") {
                    return Err(PlyError("only PLY version 1.0 is supported".into()));
                }
            }
            "element" => {
                let name = fields
                    .next()
                    .ok_or_else(|| PlyError("missing PLY element name".into()))?;
                let count = fields
                    .next()
                    .ok_or_else(|| PlyError("missing PLY element count".into()))?
                    .parse()
                    .map_err(|_| PlyError("invalid PLY element count".into()))?;
                elements.push(Element {
                    name: name.into(),
                    count,
                    properties: Vec::new(),
                });
            }
            "property" => {
                let element = elements
                    .last_mut()
                    .ok_or_else(|| PlyError("PLY property appears before an element".into()))?;
                let first = fields
                    .next()
                    .ok_or_else(|| PlyError("missing PLY property type".into()))?;
                if first == "list" {
                    element.properties.push(Property::List {
                        count: scalar(fields.next())?,
                        item: scalar(fields.next())?,
                    });
                    fields
                        .next()
                        .ok_or_else(|| PlyError("missing PLY list property name".into()))?;
                } else {
                    element.properties.push(Property::Scalar {
                        ty: scalar(Some(first))?,
                        name: fields
                            .next()
                            .ok_or_else(|| PlyError("missing PLY property name".into()))?
                            .into(),
                    });
                }
            }
            "end_header" => break,
            _ => {
                return Err(PlyError(format!(
                    "unsupported PLY header directive `{kind}`"
                )));
            }
        }
    }
    Ok((
        format.ok_or_else(|| PlyError("PLY header has no format".into()))?,
        elements,
    ))
}

fn scalar(value: Option<&str>) -> Result<Scalar, PlyError> {
    match value {
        Some("char" | "int8") => Ok(Scalar::I8),
        Some("uchar" | "uint8") => Ok(Scalar::U8),
        Some("short" | "int16") => Ok(Scalar::I16),
        Some("ushort" | "uint16") => Ok(Scalar::U16),
        Some("int" | "int32") => Ok(Scalar::I32),
        Some("uint" | "uint32") => Ok(Scalar::U32),
        Some("float" | "float32") => Ok(Scalar::F32),
        Some("double" | "float64") => Ok(Scalar::F64),
        Some(value) => Err(PlyError(format!("unsupported PLY scalar type `{value}`"))),
        None => Err(PlyError("missing PLY scalar type".into())),
    }
}

fn parse_ascii_body(
    reader: &mut impl BufRead,
    elements: &[Element],
    layout: &VertexLayout,
    gaussians: &mut Vec<Gaussian>,
    points: &mut Vec<Point>,
    higher_order: &mut Vec<f32>,
) -> Result<(), PlyError> {
    let mut line = String::new();
    for element in elements {
        let mut values = Vec::with_capacity(element.properties.len());
        for record in 0..element.count {
            line.clear();
            if reader
                .read_line(&mut line)
                .map_err(|error| PlyError(format!("read PLY body: {error}")))?
                == 0
            {
                return Err(PlyError(format!(
                    "unexpected end of PLY {} data at record {record}",
                    element.name
                )));
            }
            let mut fields = line.split_whitespace();
            values.clear();
            for property in &element.properties {
                match property {
                    Property::Scalar { .. } => {
                        values.push(parse_ascii_scalar(fields.next().ok_or_else(|| {
                            PlyError(format!("short PLY {} record {record}", element.name))
                        })?)?)
                    }
                    Property::List { .. } => {
                        let count = parse_ascii_count(fields.next().ok_or_else(|| {
                            PlyError(format!("short PLY {} list record {record}", element.name))
                        })?)?;
                        for _ in 0..count {
                            fields.next().ok_or_else(|| {
                                PlyError(format!("short PLY {} list record {record}", element.name))
                            })?;
                        }
                    }
                }
            }
            if element.name == "vertex" {
                push_vertex(&values, record, layout, gaussians, points, higher_order)?;
            }
        }
    }
    Ok(())
}

fn parse_binary_body(
    reader: &mut impl Read,
    elements: &[Element],
    format: Format,
    layout: &VertexLayout,
    gaussians: &mut Vec<Gaussian>,
    points: &mut Vec<Point>,
    higher_order: &mut Vec<f32>,
) -> Result<(), PlyError> {
    for element in elements {
        let mut values = Vec::with_capacity(element.properties.len());
        for record in 0..element.count {
            values.clear();
            for property in &element.properties {
                match property {
                    Property::Scalar { ty, .. } => values.push(read_scalar(reader, *ty, format)?),
                    Property::List { count, item } => {
                        let count = read_scalar(reader, *count, format)?;
                        if !count.is_finite() || count < 0.0 || count.fract() != 0.0 {
                            return Err(PlyError("invalid PLY list count".into()));
                        }
                        for _ in 0..count as usize {
                            read_scalar(reader, *item, format)?;
                        }
                    }
                }
            }
            if element.name == "vertex" {
                push_vertex(&values, record, layout, gaussians, points, higher_order)?;
            }
        }
    }
    Ok(())
}

fn push_vertex(
    values: &[f64],
    record: usize,
    layout: &VertexLayout,
    gaussians: &mut Vec<Gaussian>,
    points: &mut Vec<Point>,
    higher_order: &mut Vec<f32>,
) -> Result<(), PlyError> {
    match layout {
        VertexLayout::Gaussian(layout) => {
            gaussians.push(gaussian(values, record, layout, higher_order)?)
        }
        VertexLayout::PointCloud(layout) => points.push(point(values, record, layout)?),
    }
    Ok(())
}

fn point(values: &[f64], record: usize, layout: &PointCloudLayout) -> Result<Point, PlyError> {
    let value = |index: usize, name: &str| -> Result<f32, PlyError> {
        let value = values[index] as f32;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(PlyError(format!(
                "point record {record} has non-finite `{name}`"
            )))
        }
    };
    Ok(Point {
        position: Vec3::new(
            value(layout.position[0], "x")?,
            value(layout.position[1], "y")?,
            value(layout.position[2], "z")?,
        ),
        color: [
            value(layout.color[0], "red")? / COLOR_CHANNEL_MAXIMUM,
            value(layout.color[1], "green")? / COLOR_CHANNEL_MAXIMUM,
            value(layout.color[2], "blue")? / COLOR_CHANNEL_MAXIMUM,
        ],
    })
}

fn gaussian(
    values: &[f64],
    record: usize,
    layout: &GaussianLayout,
    higher_order: &mut Vec<f32>,
) -> Result<Gaussian, PlyError> {
    let value = |index: usize, name: &str| -> Result<f32, PlyError> {
        let value = values[index] as f32;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(PlyError(format!(
                "Gaussian record {record} has non-finite `{name}`"
            )))
        }
    };
    let opacity = values[layout.opacity];
    if opacity.is_nan() {
        return Err(PlyError(format!(
            "Gaussian record {record} has non-finite `opacity`"
        )));
    }
    let opacity = opacity.clamp(f32::MIN as f64, f32::MAX as f64) as f32;
    let rotation = [
        value(layout.rotation[0], "rot_0")?,
        value(layout.rotation[1], "rot_1")?,
        value(layout.rotation[2], "rot_2")?,
        value(layout.rotation[3], "rot_3")?,
    ];
    if rotation.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON {
        return Err(PlyError(format!(
            "Gaussian record {record} has a zero rotation quaternion"
        )));
    }
    let mut spherical_harmonics = [0.0; SH_CHANNELS];
    for (channel, coefficient) in spherical_harmonics.iter_mut().enumerate() {
        *coefficient = value(
            layout.spherical_harmonics[0][channel].expect("DC SH index is required"),
            &format!("f_dc_{channel}"),
        )?;
    }
    let degree = layout.spherical_harmonic_degree as usize;
    let coefficients_per_channel = (degree + 1).pow(2) - 1;
    let higher_order_indices = &layout.spherical_harmonics[1..coefficients_per_channel + 1];
    for (channel, _) in spherical_harmonics.iter().enumerate() {
        for (coefficient, indices) in higher_order_indices.iter().enumerate() {
            if let Some(index) = indices[channel] {
                let name = format!(
                    "f_rest_{}",
                    channel * coefficients_per_channel + coefficient
                );
                higher_order.push(value(index, &name)?);
            }
        }
    }
    Ok(Gaussian {
        position: Vec3::new(
            value(layout.position[0], "x")?,
            value(layout.position[1], "y")?,
            value(layout.position[2], "z")?,
        ),
        spherical_harmonics,
        opacity,
        scale: Vec3::new(
            value(layout.scale[0], "scale_0")?,
            value(layout.scale[1], "scale_1")?,
            value(layout.scale[2], "scale_2")?,
        ),
        rotation,
    })
}

fn parse_ascii_scalar(value: &str) -> Result<f64, PlyError> {
    value
        .parse()
        .map_err(|_| PlyError(format!("invalid PLY scalar `{value}`")))
}

fn parse_ascii_count(value: &str) -> Result<usize, PlyError> {
    value
        .parse()
        .map_err(|_| PlyError(format!("invalid PLY list count `{value}`")))
}

fn read_scalar(reader: &mut impl Read, ty: Scalar, format: Format) -> Result<f64, PlyError> {
    let little = matches!(format, Format::LittleEndian);
    macro_rules! read {
        ($size:literal, $ty:ty) => {{
            let mut bytes = [0; $size];
            reader
                .read_exact(&mut bytes)
                .map_err(|error| PlyError(format!("read PLY body: {error}")))?;
            if little {
                <$ty>::from_le_bytes(bytes) as f64
            } else {
                <$ty>::from_be_bytes(bytes) as f64
            }
        }};
    }
    Ok(match ty {
        Scalar::I8 => read!(1, i8),
        Scalar::U8 => read!(1, u8),
        Scalar::I16 => read!(2, i16),
        Scalar::U16 => read!(2, u16),
        Scalar::I32 => read!(4, i32),
        Scalar::U32 => read!(4, u32),
        Scalar::F32 => read!(4, f32),
        Scalar::F64 => read!(8, f64),
    })
}
