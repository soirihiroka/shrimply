//! Camera recipes stay independent of GPU resources and output resolution.
mod math;
pub mod path;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    #[default]
    Isometric,
}

impl View {
    pub const ALL: [Self; 7] = [
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::Isometric,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Isometric => "isometric",
        }
    }
}
impl fmt::Display for View {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
impl FromStr for View {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|view| view.name() == value)
            .ok_or_else(|| {
                format!(
                    "unknown voxel view `{value}`; use {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Projection {
    Orthographic { height: f32 },
    Perspective { vertical_fov_degrees: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub projection: Projection,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraRig {
    pub view: View,
    pub yaw_degrees: f32,
    pub pitch_degrees: f32,
    pub zoom: f32,
    /// None selects an orthographic camera; Some selects perspective.
    pub vertical_fov_degrees: Option<f32>,
}

impl Default for CameraRig {
    fn default() -> Self {
        Self {
            view: View::Isometric,
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
            zoom: 1.0,
            vertical_fov_degrees: None,
        }
    }
}

/// Basis values shared by CPU inspection and GPU ray generation.
#[derive(Clone, Copy, Debug)]
pub struct CameraBasis {
    pub position: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub half_height: f32,
    pub aspect: f32,
    pub perspective: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
}
