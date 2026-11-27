use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEY: &str = "recent_projects";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecentProject {
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub path: PathBuf,
    opened_at: u64,
}

pub fn load() -> Result<Vec<RecentProject>, String> {
    let mut projects = load_from(&open()?)?;
    projects.retain(|project| project.path.is_file());
    projects.sort_by_key(|project| std::cmp::Reverse(project.opened_at));
    Ok(projects)
}

pub fn settings_db_path() -> PathBuf {
    shrimply_path_core::config_directory().join("settings.sqlite")
}

pub fn touch(path: &Path, name: &str, tags: &[String]) -> Result<(), String> {
    let mut conn = open()?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut projects = load_from(&transaction)?;
    let path = absolute_path(path);
    projects.retain(|project| project.path != path);
    projects.insert(
        0,
        RecentProject {
            name: name.to_string(),
            tags: tags.to_vec(),
            path,
            opened_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    store(&transaction, &projects)?;
    transaction.commit().map_err(|error| error.to_string())
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
