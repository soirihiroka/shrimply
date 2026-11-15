use glam::Mat3;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use glam::UVec2;
use opencv::core::Mat;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use opencv::core::{MatTraitConst, Point2f, Vector};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use opencv::prelude::MatTraitConstManual;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use shrimply_math_geometry::optical_flow::OpticalFlowField;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub struct Estimator {
    renderer: shrimply_render_vulkan::Renderer,
    previous: shrimply_render_vulkan::Buffer,
}

#[cfg(target_os = "macos")]
pub struct Estimator {
    previous: Vec<u8>,
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
pub struct Estimator {
    previous: Mat,
}

impl Estimator {
    pub fn new(previous: &Mat) -> Result<Self, String> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let renderer = shrimply_render_vulkan::Renderer::new()?;
            let previous = renderer.upload(&rgba(previous)?)?;
            Ok(Self { renderer, previous })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                previous: rgba(previous)?,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
        {
            Ok(Self {
                previous: previous.clone(),
            })
        }
    }

    pub fn estimate(&mut self, current: &Mat) -> Result<Mat3, String> {
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        {
            let tracks = self.tracks(current)?;
            let previous = Vector::<Point2f>::from_iter(
                tracks.iter().map(|track| Point2f::new(track[0], track[1])),
            );
            let current = Vector::<Point2f>::from_iter(
                tracks.iter().map(|track| Point2f::new(track[2], track[3])),
            );
            if current.len() < super::MINIMUM_TRACKED_FEATURES {
                return Err(format!(
                    "GPU optical flow retained only {} reliable affine tracks",
                    current.len()
                ));
            }
            super::estimate_affine_from_points(&current, &previous)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
        {
            let motion = super::estimate_affine(&self.previous, current)?;
            self.previous = current.clone();
            Ok(motion)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    pub fn tracks(&mut self, current: &Mat) -> Result<Vec<[f32; 6]>, String> {
        let size = UVec2::new(current.cols() as u32, current.rows() as u32);
        Ok(self.estimate_flow(current, size)?.reliable_tracks(size))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn estimate_flow(&mut self, current: &Mat, size: UVec2) -> Result<OpticalFlowField, String> {
        let current = self.renderer.upload(&rgba(current)?)?;
        let mut submissions = Vec::new();
        let field = self.renderer.optical_flow(
            self.previous.clone(),
            current.clone(),
            (size.x, size.y),
            &mut submissions,
        )?;
        submissions
            .last()
            .ok_or("GPU optical flow submitted no work")?
            .wait()?;
        let read = |buffer: &shrimply_render_vulkan::Buffer| -> Result<_, String> {
            Ok(self
                .renderer
                .readback(buffer)?
                .chunks_exact(size_of::<[f32; 2]>())
                .map(|bytes| {
                    glam::Vec2::new(
                        f32::from_ne_bytes(bytes[..4].try_into().expect("flow x component")),
                        f32::from_ne_bytes(bytes[4..].try_into().expect("flow y component")),
                    )
                })
                .collect())
        };
        let flow = OpticalFlowField::new(
            UVec2::new(field.grid.0, field.grid.1),
            read(&field.forward)?,
            read(&field.backward)?,
        )?;
        self.previous = current;
        Ok(flow)
    }

    #[cfg(target_os = "macos")]
    fn estimate_flow(&mut self, current: &Mat, size: UVec2) -> Result<OpticalFlowField, String> {
        let current = rgba(current)?;
        let flow = super::flow_metal::estimate_rgba(&self.previous, &current, size.x, size.y)?;
        self.previous = current;
        Ok(flow)
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn rgba(frame: &Mat) -> Result<Vec<u8>, String> {
    let mut rgba = Mat::default();
    opencv::imgproc::cvt_color_def(frame, &mut rgba, opencv::imgproc::COLOR_BGR2RGBA)
        .map_err(|error| error.to_string())?;
    let bytes = rgba.data_bytes().map_err(|error| error.to_string())?;
    if bytes.len() != frame.cols() as usize * frame.rows() as usize * size_of::<u32>() {
        return Err("GPU optical flow input is not tightly packed".into());
    }
    Ok(bytes.to_vec())
}
