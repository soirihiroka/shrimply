use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use rusqlite::{Connection, OptionalExtension, params};
use shrimply_editing_api::edit::apply_shared_edit;
use shrimply_project_document::project::{ChangeError, Project, ProjectEncoding, ProjectSession};
use uuid::Uuid;

use crate::{
    ChangeRequest, CommitRequest, Presence, ProjectVersion, blob_digest, media::BlobStore,
};

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
    document: ProjectSession,
    participants: BTreeMap<Uuid, (Instant, Presence)>,
}

impl Session {
    fn new(revision: u64, project: Project) -> Self {
        Self {
            document: ProjectSession::at_revision(Arc::new(project), revision, 1),
            participants: BTreeMap::new(),
        }
    }

    fn version(&self) -> ProjectVersion {
        ProjectVersion {
            revision: self.document.revision(),
            project: self.document.current().clone(),
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
        mut project: Project,
    ) -> Result<ProjectVersion, Failure> {
        project.ensure_ids();
        project.clear_view_state();
        project.validate().map_err(Failure::BadRequest)?;
        verify_media(&self.blobs, id, &project)?;
        let bytes = ProjectEncoding::Shrimp
            .encode(&project)
            .map_err(Failure::BadRequest)?;
        let created = self.connection.execute(
            "INSERT INTO projects VALUES (?1, 0, ?2) ON CONFLICT(id) DO NOTHING",
            params![id.to_string(), bytes],
        )?;
        if created == 0 {
            return Err(Failure::Conflict(self.session(id)?.document.revision()));
        }
        let session = Session::new(0, project);
        let version = session.version();
        self.sessions.insert(id, session);
        Ok(version)
    }

    pub(super) fn exists(&mut self, id: Uuid) -> Result<(), Failure> {
        self.session(id).map(|_| ())
    }

    pub(super) fn get(&mut self, id: Uuid) -> Result<ProjectVersion, Failure> {
        Ok(self.session(id)?.version())
    }

    pub(super) fn sync(
        &mut self,
        id: Uuid,
        revision: u64,
        presence: impl IntoIterator<Item = Presence>,
    ) -> Result<crate::SyncResponse, Failure> {
        let session = self.session(id)?;
        let participants = session.update_presence(presence)?;
        Ok(crate::SyncResponse {
            version: (revision != session.document.revision()).then(|| session.version()),
            participants,
        })
    }

    pub(super) fn commit(&mut self, id: Uuid, request: CommitRequest) -> Result<u64, Failure> {
        self.publish(id, request.operation_id, &request, |project, revision| {
            if request.base_revision != revision {
                return Err(Failure::Conflict(revision));
            }
            apply_shared_edit(project, &request.edit).map_err(Failure::BadRequest)
        })
    }

    pub(super) fn changes(&mut self, id: Uuid, request: ChangeRequest) -> Result<u64, Failure> {
        self.publish(id, request.operation_id, &request, |project, revision| {
            if request.base_revision > revision {
                return Err(Failure::Conflict(revision));
            }
            if request.changes.is_empty() {
                return Err(Failure::BadRequest(
                    "an edit must change the project".into(),
                ));
            }
            request.changes.apply(project).map_err(|error| match error {
                ChangeError::Conflict(_) => Failure::Conflict(revision),
                ChangeError::Invalid(_) if request.base_revision != revision => {
                    Failure::Conflict(revision)
                }
                ChangeError::Invalid(error) => Failure::BadRequest(error),
            })
        })
    }

    fn publish(
        &mut self,
        id: Uuid,
        operation: Uuid,
        receipt: &impl serde::Serialize,
        apply: impl FnOnce(&Project, u64) -> Result<Project, Failure>,
    ) -> Result<u64, Failure> {
        self.session(id)?;
        let session = self.sessions.get_mut(&id).expect("session was opened");
        let key = id.to_string();
        let operation = operation.to_string();
        let serialized =
            serde_json::to_vec(receipt).map_err(|error| Failure::BadRequest(error.to_string()))?;
        let previous = self.connection.query_row(
            "SELECT request, revision FROM operation_receipts WHERE project = ?1 AND operation = ?2",
            params![key, operation],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
        ).optional()?;
        if let Some((previous, revision)) = previous {
            return if previous == serialized {
                from_disk_revision(revision)
            } else {
                Err(Failure::BadRequest(
                    "operation ID was used for a different edit".into(),
                ))
            };
        }
        let current_revision = session.document.revision();
        let candidate = apply(session.document.current(), current_revision)?;
        verify_media(&self.blobs, id, &candidate)?;
        let previous_revision = i64::try_from(current_revision)
            .map_err(|_| Failure::Storage("project revision exhausted".into()))?;
        let revision = previous_revision
            .checked_add(1)
            .ok_or_else(|| Failure::Storage("project revision exhausted".into()))?;
        let bytes = ProjectEncoding::Shrimp
            .encode(&candidate)
            .map_err(Failure::BadRequest)?;
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE projects SET revision = ?1, document = ?2 WHERE id = ?3 AND revision = ?4",
            params![revision, bytes, key, previous_revision],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            self.sessions.remove(&id);
            return Err(Failure::Conflict(self.session(id)?.document.revision()));
        }
        transaction.execute(
            "INSERT INTO operation_receipts VALUES (?1, ?2, ?3, ?4)",
            params![key, operation, serialized, revision],
        )?;
        transaction.commit()?;
        let revision = from_disk_revision(revision)?;
        session.document.commit(Arc::new(candidate), None);
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
                let project = ProjectEncoding::Shrimp
                    .decode(&bytes)
                    .map_err(Failure::Storage)?;
                Ok(entry.insert(Session::new(from_disk_revision(revision)?, project)))
            }
        }
    }
}

fn verify_media(blobs: &BlobStore, project_id: Uuid, project: &Project) -> Result<(), Failure> {
    for asset in project.assets() {
        let digest = blob_digest(asset.path()).ok_or_else(|| {
            Failure::BadRequest(
                "shared project assets must reference immutable project media".into(),
            )
        })?;
        if !blobs.is_complete(project_id, digest)? {
            return Err(Failure::BadRequest(format!(
                "shared project media {digest} has not finished uploading"
            )));
        }
    }
    Ok(())
}

fn from_disk_revision(value: i64) -> Result<u64, Failure> {
    u64::try_from(value).map_err(|_| Failure::Storage("invalid stored project revision".into()))
}
