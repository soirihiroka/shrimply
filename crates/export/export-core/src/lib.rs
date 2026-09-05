pub mod audio;
pub mod json;
pub mod output;
#[cfg(feature = "video")]
pub mod video;

#[cfg(feature = "video")]
use shrimply_math_media as math;
use shrimply_project::project::{AssetSnapshot, Project};

fn snapshot_assets(project: &Project) -> Result<Vec<AssetSnapshot>, String> {
    project
        .assets()
        .into_iter()
        .map(|asset| asset.snapshot())
        .collect()
}

fn ensure_assets_current(assets: &[AssetSnapshot]) -> Result<(), String> {
    assets.iter().try_for_each(AssetSnapshot::ensure_current)
}

fn verify_assets_current(assets: &[AssetSnapshot]) -> Result<(), String> {
    assets.iter().try_for_each(AssetSnapshot::verify_current)
}

pub fn ensure_output_is_not_an_asset(
    project: &Project,
    output: &std::path::Path,
) -> Result<(), String> {
    let output = output
        .canonicalize()
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                std::path::absolute(output)
            } else {
                Err(error)
            }
        })
        .map_err(|error| {
            format!(
                "Could not resolve export path {}: {error}",
                output.display()
            )
        })?;
    if let Some(asset) = project.assets().into_iter().find(|asset| {
        asset
            .path()
            .canonicalize()
            .or_else(|_| std::path::absolute(asset.path()))
            .is_ok_and(|asset| asset == output)
    }) {
        Err(format!(
            "Export destination is also a project asset: {}",
            asset.path().display()
        ))
    } else {
        Ok(())
    }
}

pub use shrimply_project::{caption, project, time_format};
