use glam::Mat3;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use glam::UVec2;
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
use shrimply_math_geometry::optical_flow::OpticalFlowField;
use shrimply_vision::RgbaImage;

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
    previous: RgbaImage,
    tracker: super::sparse::Tracker,
}

impl Estimator {
    pub fn new(previous: &RgbaImage) -> Result<Self, String> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let renderer = shrimply_render_vulkan::Renderer::new()?;
            let previous = renderer.upload(&previous.data)?;
            Ok(Self { renderer, previous })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                previous: previous.data.clone(),
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
        {
            Ok(Self {
                previous: previous.clone(),
                tracker: super::sparse::Tracker::default(),
            })
        }
    }

    pub fn estimate(&mut self, current: &RgbaImage) -> Result<Mat3, String> {
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        {
            let tracks = self.tracks(current)?;
            let previous = tracks
                .iter()
                .map(|track| [track[0], track[1]])
                .collect::<Vec<_>>();
            let current = tracks
                .iter()
                .map(|track| [track[2], track[3]])
                .collect::<Vec<_>>();
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
            let motion = super::estimate_affine(&mut self.tracker, &self.previous, current)?;
            self.previous = current.clone();
            Ok(motion)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    pub fn tracks(&mut self, current: &RgbaImage) -> Result<Vec<[f32; 6]>, String> {
        let size = UVec2::new(current.width as u32, current.height as u32);
        Ok(self.estimate_flow(current, size)?.reliable_tracks(size))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn estimate_flow(
        &mut self,
        current: &RgbaImage,
        size: UVec2,
    ) -> Result<OpticalFlowField, String> {
        let current = self.renderer.upload(&current.data)?;
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
    fn estimate_flow(
        &mut self,
        current: &RgbaImage,
        size: UVec2,
    ) -> Result<OpticalFlowField, String> {
        let current = current.data.clone();
        let flow = super::flow_metal::estimate_rgba(&self.previous, &current, size.x, size.y)?;
        self.previous = current;
        Ok(flow)
    }
}
