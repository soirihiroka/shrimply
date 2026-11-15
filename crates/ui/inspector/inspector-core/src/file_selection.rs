use std::path::Path;

use crate::{InspectorControlAction, InspectorController, InspectorTarget};

#[derive(Clone, Copy, Debug)]
pub struct FileSelection {
    pub title: &'static str,
    pub extensions: &'static [&'static str],
}

impl InspectorControlAction {
    pub fn file_selection(self) -> Option<FileSelection> {
        Some(match self {
            Self::SelectVideoGenerationMedia { .. } => FileSelection {
                title: "Select generation media",
                extensions: &[
                    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "mp4", "mov", "mkv",
                    "webm", "wav", "mp3", "flac", "ogg", "opus", "m4a", "aac",
                ],
            },
            Self::SelectObject3dModel { .. } => FileSelection {
                title: "Select 3D model",
                extensions: &["obj", "glb"],
            },
            Self::SelectScene3dEnvironment => FileSelection {
                title: "Select environment image",
                extensions: &["png", "jpg", "jpeg", "webp", "avif", "hdr", "exr"],
            },
            Self::SelectPaintTexture { .. } => FileSelection {
                title: "Select paint texture",
                extensions: &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"],
            },
            _ => return None,
        })
    }
}

impl InspectorController {
    pub fn set_control_file(
        &self,
        target: &InspectorTarget,
        action: InspectorControlAction,
        path: &Path,
    ) -> Result<(), String> {
        match action {
            InspectorControlAction::SelectVideoGenerationMedia { model, input } => {
                self.add_video_generation_media(target, model, input, path)
            }
            InspectorControlAction::SelectObject3dModel { modifier_id } => {
                self.set_object_3d_model(target, modifier_id, path)
            }
            InspectorControlAction::SelectScene3dEnvironment => {
                self.set_scene_3d_environment(target, path)
            }
            InspectorControlAction::SelectPaintTexture { color_id } => {
                self.set_paint_texture(target, color_id, path)
            }
            _ => Err("inspector action does not select a file".into()),
        }
    }
}
