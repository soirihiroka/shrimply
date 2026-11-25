use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shrimply_project_protocol::{
    LEGACY_CHECKPOINT_FIELD, ProjectVersion, PublishRequest, SyncResponse, valid_digest,
};
use uuid::Uuid;

use crate::media::BlobStore;

const LEGACY_HEADER: &[u8] = b"SHRIMP\0\x01\x07\0\0\0";

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Presence {
    id: Uuid,
    name: String,
    #[serde(flatten)]
    state: serde_json::Map<String, Value>,
}

const PRESENCE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(super) enum Failure {
    BadRequest(String),
    Conflict(u64),
    Storage(String),
    Missing,
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

struct Session {
    revision: u64,
    document: Arc<Value>,
    participants: BTreeMap<Uuid, (Instant, Presence)>,
}

impl Session {
    fn new(revision: u64, project: Value) -> Self {
        Self {
            revision,
            document: Arc::new(project),
            participants: BTreeMap::new(),
        }
    }

    fn version(&self) -> ProjectVersion<Value> {
        ProjectVersion {
            revision: self.revision,
            project: self.document.clone(),
        }
    }

    fn update_presence(
        &mut self,
        presence: impl IntoIterator<Item = Presence>,
    ) -> Result<Vec<Presence>, Failure> {
        let now = Instant::now();
        self.participants
            .retain(|_, (updated, _)| now.duration_since(*updated) < PRESENCE_TTL);
        for presence in presence {
            if presence.name.trim().is_empty() || presence.name.len() > 128 {
                return Err(Failure::BadRequest(
                    "presence name must be between 1 and 128 bytes".into(),
                ));
            }
            self.participants.insert(presence.id, (now, presence));
        }
        Ok(self
            .participants
            .values()
            .map(|(_, presence)| presence.clone())
            .collect())
    }
}

pub(super) struct Store {
    connection: Connection,
    sessions: BTreeMap<Uuid, Session>,
    blobs: Arc<BlobStore>,
}

impl Store {
    pub(super) fn open(path: &Path, blobs: Arc<BlobStore>) -> Result<Self, Failure> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS projects (
                 id TEXT PRIMARY KEY,
                 revision INTEGER NOT NULL,
                 document BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS operation_receipts (
                 project TEXT NOT NULL REFERENCES projects(id),
                 operation TEXT NOT NULL,
                 request BLOB NOT NULL,
                 revision INTEGER NOT NULL,
                 PRIMARY KEY (project, operation)
             );",
        )?;
        Ok(Self {
            connection,
            sessions: BTreeMap::new(),
            blobs,
        })
    }

    pub(super) fn create(
        &mut self,
        id: Uuid,
        project: Value,
    ) -> Result<ProjectVersion<Value>, Failure> {
        let bytes = encode(&project)?;
        let created = self.connection.execute(
            "INSERT INTO projects VALUES (?1, 0, ?2) ON CONFLICT(id) DO NOTHING",
            params![id.to_string(), bytes],
        )?;
        if created == 0 {
            return Err(Failure::Conflict(self.session(id)?.revision));
        }
        let session = Session::new(0, project);
        let version = session.version();
        self.sessions.insert(id, session);
        Ok(version)
    }

    pub(super) fn exists(&mut self, id: Uuid) -> Result<(), Failure> {
        self.session(id).map(|_| ())
    }

    pub(super) fn get(&mut self, id: Uuid) -> Result<ProjectVersion<Value>, Failure> {
        Ok(self.session(id)?.version())
    }

    pub(super) fn sync(
        &mut self,
        id: Uuid,
        revision: u64,
        presence: impl IntoIterator<Item = Presence>,
    ) -> Result<SyncResponse<Value, Presence>, Failure> {
        let session = self.session(id)?;
        let participants = session.update_presence(presence)?;
        Ok(SyncResponse {
            version: (revision != session.revision).then(|| session.version()),
            participants,
        })
    }

    fn operation(&self, id: Uuid, operation: Uuid) -> Result<Option<(u64, Vec<u8>)>, Failure> {
        let row = self.connection.query_row(
            "SELECT revision, request FROM operation_receipts WHERE project = ?1 AND operation = ?2",
            params![id.to_string(), operation.to_string()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        ).optional()?;
        row.map(|(revision, digest)| Ok((from_disk_revision(revision)?, digest)))
            .transpose()
    }

    pub(super) fn receipt(&self, id: Uuid, operation: Uuid) -> Result<Option<u64>, Failure> {
        Ok(self.operation(id, operation)?.map(|(revision, _)| revision))
    }

    pub(super) fn publish(
        &mut self,
        id: Uuid,
        request: PublishRequest<Value>,
    ) -> Result<u64, Failure> {
        if !valid_digest(&request.operation_digest) {
            return Err(Failure::BadRequest("invalid operation digest".into()));
        }
        if let Some((revision, recorded)) = self.operation(id, request.operation_id)? {
            // Older databases stored the legacy command JSON in this column.
            if !recorded.starts_with(b"{") && recorded != request.operation_digest.as_bytes() {
                return Err(Failure::BadRequest(
                    "operation ID was used for a different edit".into(),
                ));
            }
            return Ok(revision);
        }
        self.session(id)?;
        let session = self.sessions.get_mut(&id).expect("session was opened");
        let current_revision = session.revision;
        if request.base_revision != current_revision {
            return Err(Failure::Conflict(current_revision));
        }
        for digest in &request.media {
            if !valid_digest(digest) || !self.blobs.is_complete(id, digest)? {
                return Err(Failure::BadRequest(format!(
                    "shared project media {digest} has not finished uploading"
                )));
            }
        }
        let bytes = encode(&request.project)?;
        let key = id.to_string();
        let operation = request.operation_id.to_string();
        let previous_revision = i64::try_from(current_revision)
            .map_err(|_| Failure::Storage("project revision exhausted".into()))?;
        let revision = previous_revision
            .checked_add(1)
            .ok_or_else(|| Failure::Storage("project revision exhausted".into()))?;
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE projects SET revision = ?1, document = ?2 WHERE id = ?3 AND revision = ?4",
            params![revision, bytes, key, previous_revision],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            self.sessions.remove(&id);
            return Err(Failure::Conflict(self.session(id)?.revision));
        }
        transaction.execute(
            "INSERT INTO operation_receipts VALUES (?1, ?2, ?3, ?4)",
            params![
                key,
                operation,
                request.operation_digest.as_bytes(),
                revision
            ],
        )?;
        transaction.commit()?;
        let revision = from_disk_revision(revision)?;
        session.document = Arc::new(request.project);
        session.revision = revision;
        Ok(revision)
    }

    fn session(&mut self, id: Uuid) -> Result<&mut Session, Failure> {
        use std::collections::btree_map::Entry;
        match self.sessions.entry(id) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let (revision, bytes) = self
                    .connection
                    .query_row(
                        "SELECT revision, document FROM projects WHERE id = ?1",
                        [id.to_string()],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
                    )
                    .optional()?
                    .ok_or(Failure::Missing)?;
                let project = if bytes.starts_with(LEGACY_HEADER) {
                    serde_json::json!({ (LEGACY_CHECKPOINT_FIELD): STANDARD.encode(bytes) })
                } else {
                    serde_json::from_slice(&bytes)
                        .map_err(|error| Failure::Storage(error.to_string()))?
                };
                Ok(entry.insert(Session::new(from_disk_revision(revision)?, project)))
            }
        }
    }
}

fn encode(project: &Value) -> Result<Vec<u8>, Failure> {
    if !project.is_object() || project.get(LEGACY_CHECKPOINT_FIELD).is_some() {
        return Err(Failure::BadRequest(
            "project document must be a JSON object".into(),
        ));
    }
    serde_json::to_vec(project).map_err(|error| Failure::BadRequest(error.to_string()))
}

fn from_disk_revision(value: i64) -> Result<u64, Failure> {
    u64::try_from(value).map_err(|_| Failure::Storage("invalid stored project revision".into()))
}
