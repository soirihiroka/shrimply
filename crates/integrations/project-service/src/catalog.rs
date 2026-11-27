use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shrimply_resource_pipeline::fingerprint;

use crate::{ProjectClient, ProjectLink, SharedWorkspace};

#[derive(Clone, Deserialize, Serialize)]
pub struct ProjectEntry {
    pub name: String,
    pub tags: Vec<String>,
    pub path: Option<PathBuf>,
    pub remote: Option<ProjectLink>,
}

impl ProjectEntry {
    pub fn open(&self, directory: Option<&Path>) -> Result<PathBuf, String> {
        if let Some(path) = &self.path {
            return Ok(path.clone());
        }
        let link = self
            .remote
            .as_ref()
            .expect("catalog entries have a location");
        let directory = directory
            .map(Path::to_path_buf)
            .unwrap_or_else(|| shrimply_path_core::config_directory().join("project-workspaces"))
            .join(fingerprint::digest(link.server.as_bytes()))
            .join(link.project_id.to_string());
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let canonical = directory.join("project.shrimp");
        if canonical.is_file()
            && SharedWorkspace::open(&canonical)?.is_some_and(|workspace| workspace.link() == link)
        {
            return Ok(canonical);
        }
        let path = if canonical.exists() {
            SharedWorkspace::managed_path_in(&directory, "project.shrimp")?
        } else {
            canonical
        };
        SharedWorkspace::join(&path, &link.server, link.project_id)?;
        Ok(path)
    }

    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.name.to_lowercase().contains(query)
            || self
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(query))
            || self
                .path
                .as_ref()
                .is_some_and(|path| path.to_string_lossy().to_lowercase().contains(query))
            || self
                .remote
                .as_ref()
                .is_some_and(|link| link.server.to_lowercase().contains(query))
    }
}

pub fn register(
    path: &Path,
    project: &shrimply_project_document::project::Project,
) -> Result<(), String> {
    if shrimply_path_core::cache_directory().is_ok_and(|cache| path.starts_with(cache)) {
        return Ok(());
    }
    shrimply_recent_projects::touch(path, &project.name, &project.tags)
}

pub fn local_projects() -> Result<Vec<ProjectEntry>, String> {
    let mut projects = Vec::new();
    let mut seen = HashSet::new();
    for local in shrimply_recent_projects::load()? {
        let remote = SharedWorkspace::open(&local.path)
            .inspect_err(|error| tracing::warn!(%error, path = %local.path.display(), "Could not read project location"))
            .ok()
            .flatten()
            .map(|workspace| workspace.link().clone());
        let entry = ProjectEntry {
            name: local.name,
            tags: local.tags,
            path: Some(local.path),
            remote,
        };
        if entry
            .remote
            .as_ref()
            .is_some_and(|link| !seen.insert(link.clone()))
        {
            continue;
        }
        projects.push(entry);
    }

    Ok(projects)
}

pub fn list() -> Result<Vec<ProjectEntry>, String> {
    let mut projects = local_projects()?;
    let cached: HashMap<_, _> = projects
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.remote.clone().map(|link| (link, index)))
        .collect();
    let preferences = shrimply_preferences_core::open_with_defaults();
    let servers = shrimply_preferences_core::snapshot(&preferences).project_server_urls;
    let remote = std::thread::scope(|scope| {
        servers
            .into_iter()
            .map(|server| {
                scope.spawn(move || {
                    let projects = ProjectClient::configured(&server)
                        .and_then(|client| {
                            client.list_projects().map_err(|error| error.to_string())
                        })
                        .inspect_err(
                            |error| tracing::warn!(%server, %error, "Could not list projects"),
                        )
                        .unwrap_or_default();
                    projects.into_iter().map(move |project| {
                        (
                            ProjectLink {
                                server: server.clone(),
                                project_id: project.id,
                            },
                            project.name,
                        )
                    })
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|task| task.join().expect("project listing worker panicked"))
            .collect::<Vec<_>>()
    });
    let mut discovered = Vec::new();
    for (link, name) in remote {
        if let Some(&index) = cached.get(&link) {
            projects[index].name = name;
        } else {
            discovered.push(ProjectEntry {
                name,
                tags: Vec::new(),
                path: None,
                remote: Some(link),
            });
        }
    }
    discovered.sort_by_cached_key(|project| project.name.to_lowercase());
    projects.extend(discovered);
    Ok(projects)
}
