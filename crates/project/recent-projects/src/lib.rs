use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEY: &str = "recent_projects";
const LIMIT: usize = 50;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecentProject {
    pub name: String,
    pub path: PathBuf,
    opened_at: u64,
}

pub fn load() -> Result<Vec<RecentProject>, String> {
    let conn = open()?;
    let value = conn
        .query_row(
            "SELECT value FROM key_values WHERE key = ?1",
            [KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("could not read recent projects: {error}"))?;
    let mut projects: Vec<RecentProject> = value
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .inspect_err(|error| tracing::warn!("Could not parse recent projects: {error}"))
        .unwrap_or_default()
        .unwrap_or_default();
    let previous_len = projects.len();
    projects.retain(|project: &RecentProject| project.path.is_file());
    projects.sort_by_key(|project| std::cmp::Reverse(project.opened_at));
    projects.truncate(LIMIT);
    if projects.len() != previous_len {
        store(&conn, &projects)?;
    }
    Ok(projects)
}

pub fn settings_db_path() -> PathBuf {
    shrimply_path_core::config_directory().join("settings.sqlite")
}

pub fn touch(path: &Path, name: &str) -> Result<(), String> {
    let conn = open()?;
    let mut projects = load_from(&conn)?;
    let path = absolute_path(path);
    projects.retain(|project| project.path != path);
    projects.insert(
        0,
        RecentProject {
            name: name.to_string(),
            path,
            opened_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    projects.truncate(LIMIT);
    store(&conn, &projects)
}

pub fn remove(path: &Path) -> Result<(), String> {
    let conn = open()?;
    let path = absolute_path(path);
    let mut projects = load_from(&conn)?;
    projects.retain(|project| project.path != path);
    store(&conn, &projects)
}

pub fn clear() -> Result<(), String> {
    open()?
        .execute("DELETE FROM key_values WHERE key = ?1", [KEY])
        .map(|_| ())
        .map_err(|error| format!("could not clear recent projects: {error}"))
}

fn open() -> Result<Connection, String> {
    let path = settings_db_path();
    let directory = path
        .parent()
        .expect("settings database should have a parent directory");
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let conn = Connection::open(&path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS key_values (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )
    .map_err(|error| format!("could not initialize settings: {error}"))?;
    Ok(conn)
}

fn load_from(conn: &Connection) -> Result<Vec<RecentProject>, String> {
    let value = conn
        .query_row(
            "SELECT value FROM key_values WHERE key = ?1",
            [KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("could not read recent projects: {error}"))?;
    Ok(value
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .inspect_err(|error| tracing::warn!("Could not parse recent projects: {error}"))
        .unwrap_or_default()
        .unwrap_or_default())
}

fn store(conn: &Connection, projects: &[RecentProject]) -> Result<(), String> {
    let value = serde_json::to_string(projects)
        .map_err(|error| format!("could not serialize recent projects: {error}"))?;
    conn.execute(
        "INSERT INTO key_values (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![KEY, value],
    )
    .map(|_| ())
    .map_err(|error| format!("could not save recent projects: {error}"))
}

fn absolute_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}
