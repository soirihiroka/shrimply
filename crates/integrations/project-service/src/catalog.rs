use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shrimply_resource_pipeline::fingerprint;
use uuid::Uuid;

use crate::{ProjectClient, ProjectLink, SharedWorkspace};

#[derive(Clone, Deserialize, Serialize)]
pub struct ProjectEntry {
    pub name: String,
    pub tags: Vec<String>,
    pub path: Option<PathBuf>,
    pub remote: Option<ProjectLink>,
    #[serde(default)]
    pub opened_at: Option<i64>,
}

impl ProjectEntry {
    pub fn key(&self) -> String {
        match &self.remote {
            Some(link) => format!("{}#{}", link.server, link.project_id),
            None => format!("{:?}", self.path),
        }
    }

    pub fn delete(&self) -> Result<(), String> {
        if let Some(link) = &self.remote {
            return ProjectClient::new(&link.server)?
                .delete(link.project_id)
                .map_err(|error| error.to_string());
        }
        let path = self.path.as_deref().ok_or("Project file is missing")?;
        use shrimply_project_document::project::{acquire_project_lock, release_project_lock};
        acquire_project_lock(path)
            .map_err(|error| format!("Could not acquire the project for deletion: {error:?}"))?;
        let result = std::fs::remove_file(path).map_err(|error| error.to_string());
        release_project_lock(path);
        result
    }

    pub fn destination(&self, directory: Option<&Path>) -> Result<PathBuf, String> {
        if let Some(path) = &self.path {
            return Ok(path.clone());
        }
        let link = self
            .remote
            .as_ref()
            .ok_or("catalog entry has no project location")?;
        let directory = directory
            .map(Path::to_path_buf)
            .unwrap_or_else(|| shrimply_path_core::config_directory().join("project-workspaces"))
            .join(fingerprint::digest(link.server.as_bytes()))
            .join(link.project_id.to_string());
        let canonical = directory.join("project.shrimp");
        if canonical.try_exists().map_err(|error| error.to_string())?
            && !SharedWorkspace::open(&canonical)?.is_some_and(|workspace| workspace.link() == link)
        {
            return Ok(directory
                .join(Uuid::new_v4().to_string())
                .join("project.shrimp"));
        }
        Ok(canonical)
    }

    pub fn open(&self, directory: Option<&Path>) -> Result<PathBuf, String> {
        let path = self.destination(directory)?;
        if self.path.is_none()
            && !path.is_file()
            && let Some(link) = &self.remote
        {
            std::fs::create_dir_all(
                path.parent()
                    .ok_or("project destination has no directory")?,
            )
            .map_err(|error| error.to_string())?;
            SharedWorkspace::join(&path, &link.server, link.project_id)?;
        }
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
            opened_at: i64::try_from(local.opened_at).ok(),
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

#[derive(Serialize)]
pub struct ProjectSource {
    pub url: String,
    pub name: String,
}

impl ProjectSource {
    fn new(url: String) -> Self {
        let name = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_owned();
        Self { url, name }
    }
}

fn project_sources() -> Vec<ProjectSource> {
    let preferences = shrimply_preferences_core::open_with_defaults();
    shrimply_preferences_core::snapshot(&preferences)
        .project_server_urls
        .into_iter()
        .map(ProjectSource::new)
        .collect()
}

pub struct ProjectCatalog {
    pub projects: Vec<ProjectEntry>,
    pub servers: Vec<ProjectSource>,
}

pub fn local_catalog() -> Result<ProjectCatalog, String> {
    let servers = project_sources();
    let mut projects = local_projects()?;
    projects.retain(|entry| {
        entry
            .remote
            .as_ref()
            .is_none_or(|link| servers.iter().any(|source| source.url == link.server))
    });
    Ok(ProjectCatalog { projects, servers })
}

pub fn list() -> Result<ProjectCatalog, String> {
    let ProjectCatalog {
        mut projects,
        servers,
    } = local_catalog()?;
    let cached: HashMap<_, _> = projects
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.remote.clone().map(|link| (link, index)))
        .collect();
    let remote = std::thread::scope(|scope| {
        servers.into_iter().map(|mut source| {
                scope.spawn(move || {
                    let projects = ProjectClient::new(&source.url)
                        .and_then(|client| client.list_projects().map_err(|error| error.to_string()))
                        .map(|(projects, name)| {
                            if let Some(name) = name {
                                source.name = name;
                            }
                            projects
                        })
                        .inspect_err(
                            |error| tracing::warn!(server = %source.url, %error, "Could not list projects"),
                        );
                    (source, projects)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|task| task.join().expect("project listing worker panicked"))
            .collect::<Vec<_>>()
    });
    let mut available = HashSet::new();
    let mut listed = HashSet::new();
    let mut servers = Vec::with_capacity(remote.len());
    for (source, entries) in remote {
        let server = source.url.clone();
        servers.push(source);
        let Ok(entries) = entries else { continue };
        for entry in entries {
            let link = ProjectLink {
                server: server.clone(),
                project_id: entry.id,
            };
            listed.insert(link.clone());
            if let Some(&index) = cached.get(&link) {
                projects[index].name = entry.name;
                projects[index].opened_at = projects[index].opened_at.max(entry.opened_at);
            } else {
                projects.push(ProjectEntry {
                    name: entry.name,
                    tags: Vec::new(),
                    path: None,
                    remote: Some(link),
                    opened_at: entry.opened_at,
                });
            }
        }
        available.insert(server);
    }
    projects.retain(|entry| {
        entry
            .remote
            .as_ref()
            .is_none_or(|link| !available.contains(&link.server) || listed.contains(link))
    });
    projects.sort_by_cached_key(|entry| {
        (
            std::cmp::Reverse(entry.opened_at),
            entry.name.to_lowercase(),
        )
    });
    Ok(ProjectCatalog { projects, servers })
}
