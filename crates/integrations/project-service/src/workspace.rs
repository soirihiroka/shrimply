use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, params};
use shrimply_editing_api::protocol::MediaRewrites;
use shrimply_project_document::project::{self, Project, ProjectChanges, ProjectEncoding};
use shrimply_resource_pipeline::fingerprint;
use uuid::Uuid;

use shrimply_project_protocol::PublishRequest;

use crate::{
    ChangeRequest, MediaBlob, MediaCache, Presence, ProjectClient, ProjectVersion, RequestError,
    SyncRequest,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLink {
    pub server: String,
    pub project_id: Uuid,
}

impl ProjectLink {
    pub fn from_url(url: &str, servers: &[String]) -> Result<Self, String> {
        let (server, id) = url
            .trim()
            .rsplit_once("/v1/projects/")
            .ok_or("Enter a Shrimply project link")?;
        let server = shrimply_preferences_core::normalize_project_server_url(server)?;
        if !servers.contains(&server) {
            return Err(format!(
                "Add {server} in Project Servers before opening its project link"
            ));
        }
        let project_id = Uuid::parse_str(id).map_err(|_| "The project link has an invalid ID")?;
        Ok(Self { server, project_id })
    }
}

impl std::fmt::Display for ProjectLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}/v1/projects/{}",
            self.server.trim_end_matches('/'),
            self.project_id
        )
    }
}

pub struct WorkspaceUpdate {
    pub revision: u64,
    pub draft_changed: bool,
    pub participants: Vec<Presence>,
    pub conflict: Option<String>,
}

/// Durable working state for a shared project on this device.
pub struct SharedWorkspace {
    sqlite: Connection,
    link: ProjectLink,
    upload_directory: PathBuf,
    key: String,
}

impl SharedWorkspace {
    pub fn database(project: &Path) -> PathBuf {
        let path = project::normalized_project_path(project);
        let key = fingerprint::digest(path.as_os_str().as_encoded_bytes());
        shrimply_path_core::config_directory()
            .join("shared-projects")
            .join(format!("{key}.sqlite"))
    }

    pub fn managed_path(filename: &str) -> Result<PathBuf, String> {
        Self::managed_path_in(
            &shrimply_path_core::config_directory().join("project-workspaces"),
            filename,
        )
    }

    pub fn managed_path_in(directory: &Path, filename: &str) -> Result<PathBuf, String> {
        let directory = directory.join(Uuid::new_v4().to_string());
        fs::create_dir_all(&directory).map_err(text)?;
        Ok(directory.join(filename))
    }

    pub fn open(project: &Path) -> Result<Option<Self>, String> {
        let path = Self::database(project);
        if !path.try_exists().map_err(text)? {
            return Ok(None);
        }
        let sqlite = Connection::open(&path).map_err(text)?;
        configure(&sqlite)?;
        let link = sqlite
            .query_row(
                "SELECT server, project_id FROM shared_project WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(text)?;
        Ok(Some(Self {
            sqlite,
            upload_directory: path.with_extension("uploads"),
            key: path
                .file_stem()
                .expect("workspace has a key")
                .to_string_lossy()
                .into_owned(),
            link: ProjectLink {
                server: link.0,
                project_id: Uuid::parse_str(&link.1).map_err(text)?,
            },
        }))
    }

    pub fn create(
        project_file: &Path,
        server: &str,
        initial: &Project,
    ) -> Result<(Self, Arc<Project>), String> {
        Self::check_destination(project_file)?;
        let client = ProjectClient::configured(server)?;
        let workspace = match Self::open(project_file)? {
            Some(workspace) if workspace.link.server == server => workspace,
            Some(_) => {
                return Err("this destination has a pending project on another server".into());
            }
            None => {
                let project_id = Uuid::new_v4();
                Self::seed(project_file, server, project_id, 0, initial)?
            }
        };
        let project_id = workspace.link.project_id;
        let initial = workspace.draft()?;
        let version = match client.create(project_id, &initial) {
            Ok(version) => version,
            Err(RequestError::Rejected {
                revision: Some(_), ..
            }) => client.get(project_id).map_err(text)?,
            Err(error) => return Err(text(error)),
        };
        workspace.finish(project_file, &client, version)
    }

    pub fn join(
        project_file: &Path,
        server: &str,
        project_id: Uuid,
    ) -> Result<(Self, Arc<Project>), String> {
        Self::check_destination(project_file)?;
        let client = ProjectClient::configured(server)?;
        let version = client.get(project_id).map_err(text)?;
        let workspace = match Self::open(project_file)? {
            Some(workspace)
                if workspace.link.server == server && workspace.link.project_id == project_id =>
            {
                workspace
            }
            Some(_) => return Err("this destination belongs to a different shared project".into()),
            None => Self::seed(
                project_file,
                server,
                project_id,
                version.revision,
                &version.project,
            )?,
        };
        workspace.finish(project_file, &client, version)
    }

    fn check_destination(project_file: &Path) -> Result<(), String> {
        if !project_file
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("shrimp"))
            || !project_file.parent().is_some_and(Path::is_dir)
        {
            return Err("shared workspace needs a .shrimp path in an existing directory".into());
        }
        if project_file.try_exists().map_err(text)? {
            return Err("this destination already has a project".into());
        }
        Ok(())
    }

    fn seed(
        project_file: &Path,
        server: &str,
        project_id: Uuid,
        revision: u64,
        project: &Project,
    ) -> Result<Self, String> {
        let path = Self::database(project_file);
        let parent = path.parent().expect("workspace database has a parent");
        fs::create_dir_all(parent).map_err(text)?;
        let temporary = tempfile::NamedTempFile::new_in(parent).map_err(text)?;
        {
            let mut sqlite = Connection::open(temporary.path()).map_err(text)?;
            let transaction = sqlite.transaction().map_err(text)?;
            transaction
                .execute_batch(
                    "CREATE TABLE shared_project (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    server TEXT NOT NULL,
                    project_id TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    document BLOB NOT NULL,
                    draft BLOB NOT NULL
                 );
                 CREATE TABLE pending_edits (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    operation BLOB NOT NULL
                 );
                 CREATE TABLE project_media (
                    digest TEXT PRIMARY KEY,
                    length INTEGER NOT NULL,
                    pending INTEGER NOT NULL
                 );",
                )
                .map_err(text)?;
            transaction
                .execute(
                    "INSERT INTO shared_project VALUES (1, ?1, ?2, ?3, ?4, ?4)",
                    params![
                        server,
                        project_id.to_string(),
                        i64::try_from(revision).map_err(text)?,
                        ProjectEncoding::Shrimp.encode(project)?,
                    ],
                )
                .map_err(text)?;
            transaction.commit().map_err(text)?;
        }
        temporary.persist_noclobber(&path).map_err(text)?;
        Self::open(project_file)?.ok_or("shared workspace was not written".into())
    }

    fn finish(
        self,
        project_file: &Path,
        client: &ProjectClient,
        version: ProjectVersion,
    ) -> Result<(Self, Arc<Project>), String> {
        self.record_media(client, &version.project)?;
        let document = ProjectEncoding::Shrimp.encode(&version.project)?;
        self.sqlite
            .execute(
                "UPDATE shared_project SET revision = ?1, document = ?2, draft = ?2 WHERE id = 1",
                params![i64::try_from(version.revision).map_err(text)?, document],
            )
            .map_err(text)?;
        project::create_new_project_file(project_file, &version.project)?;
        Ok((self, version.project))
    }

    pub fn link(&self) -> &ProjectLink {
        &self.link
    }

    pub fn draft(&self) -> Result<Project, String> {
        draft(&self.sqlite)
    }

    pub fn upload_directory(&self) -> &Path {
        &self.upload_directory
    }

    pub fn queue(
        &self,
        before: &Project,
        after: &Project,
        media: &[MediaBlob],
        receipt: Option<(Uuid, &MediaRewrites)>,
    ) -> Result<(), String> {
        let changes = ProjectChanges::between(before, after).map_err(text)?;
        if changes.is_empty() {
            return Ok(());
        }
        let transaction = self.sqlite.unchecked_transaction().map_err(text)?;
        let request = ChangeRequest {
            operation_id: Uuid::new_v4(),
            changes,
        };
        let project = request.changes.apply(&draft(&transaction)?).map_err(text)?;
        let document = ProjectEncoding::Shrimp.encode(&project)?;
        transaction
            .execute(
                "INSERT INTO pending_edits (operation) VALUES (?1)",
                [serde_json::to_vec(&request).map_err(text)?],
            )
            .map_err(text)?;
        transaction
            .execute(
                "UPDATE shared_project SET draft = ?1 WHERE id = 1",
                [document],
            )
            .map_err(text)?;
        for blob in media {
            transaction
                .execute(
                    "INSERT INTO project_media VALUES (?1, ?2, 1) ON CONFLICT(digest) DO NOTHING",
                    params![blob.digest, i64::try_from(blob.length).map_err(text)?],
                )
                .map_err(text)?;
        }
        if let Some((id, rewrites)) = receipt {
            transaction
                .execute(
                    "INSERT INTO attached_receipts (operation, rewrites) VALUES (?1, ?2)",
                    params![id.to_string(), serde_json::to_vec(rewrites).map_err(text)?],
                )
                .map_err(text)?;
            transaction.execute(
                "DELETE FROM attached_receipts WHERE rowid <= (SELECT max(rowid) - 1024 FROM attached_receipts)",
                [],
            ).map_err(text)?;
        }
        transaction.commit().map_err(text)
    }

    pub fn attached_receipt(&self, id: Uuid) -> Result<Option<MediaRewrites>, String> {
        self.sqlite
            .query_row(
                "SELECT rewrites FROM attached_receipts WHERE operation = ?1",
                [id.to_string()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(text)?
            .map(|receipt| serde_json::from_slice(&receipt).map_err(text))
            .transpose()
    }

    pub fn synchronize(
        &mut self,
        client: &ProjectClient,
        participants: Vec<Presence>,
    ) -> Result<WorkspaceUpdate, String> {
        let previous_revision = self.confirmed_revision()?;
        let id = self.link.project_id;
        let mut update = client
            .sync(
                id,
                &SyncRequest {
                    revision: previous_revision,
                    presence: None,
                    participants,
                },
            )
            .map_err(text)?;
        if let Some(version) = &update.version {
            require_revision(version.revision, previous_revision)?;
        }
        let mut sent = Vec::new();
        let mut conflicts = BTreeMap::new();
        if self.has_pending_uploads()? {
            if let Some(version) = &mut update.version {
                for (sequence, request) in pending(&self.sqlite)? {
                    if let Some(receipt) = client.receipt(id, request.operation_id).map_err(text)? {
                        if receipt.revision > version.revision {
                            refresh_head(client, id, version, receipt.revision)?;
                        }
                        sent.push(sequence);
                    }
                }
            }
        } else {
            let edits = pending(&self.sqlite)?;
            let mut head = match (update.version.take(), edits.is_empty()) {
                (Some(version), _) => Some(version),
                (None, true) => None,
                (None, false) => Some(ProjectVersion {
                    revision: previous_revision,
                    project: Arc::new(confirmed(&self.sqlite)?),
                }),
            };
            for (sequence, request) in edits {
                match publish_edit(
                    client,
                    id,
                    head.as_mut().expect("pending edits have a base"),
                    &request,
                )? {
                    Ok(()) => sent.push(sequence),
                    Err(conflict) => {
                        conflicts.insert(sequence, conflict);
                    }
                }
            }
            update.version = head.filter(|head| head.revision != previous_revision);
        }
        let revision = if let Some(version) = &update.version {
            self.record_media(client, &version.project)?;
            version.revision
        } else {
            previous_revision
        };
        let draft_changed = !sent.is_empty() || !conflicts.is_empty() || update.version.is_some();
        if draft_changed {
            let document = update
                .version
                .as_ref()
                .map(|version| ProjectEncoding::Shrimp.encode(&version.project))
                .transpose()?;
            let transaction = self.sqlite.transaction().map_err(text)?;
            if let Some(document) = document {
                transaction
                    .execute(
                        "UPDATE shared_project SET revision = ?1, document = ?2 WHERE id = 1",
                        params![i64::try_from(revision).map_err(text)?, document],
                    )
                    .map_err(text)?;
            }
            for sequence in sent {
                transaction
                    .execute("DELETE FROM pending_edits WHERE sequence = ?1", [sequence])
                    .map_err(text)?;
            }
            let rebased = rebase(&transaction, &mut conflicts)?;
            if let Some(reason) = conflicts.values().next() {
                let directory = self.upload_directory.with_extension("conflicts");
                fs::create_dir_all(&directory).map_err(text)?;
                let path = directory.join(format!("{}.shrimp", Uuid::new_v4()));
                let bytes = ProjectEncoding::Shrimp.encode(&draft(&transaction)?)?;
                let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(text)?;
                file.write_all(&bytes)
                    .and_then(|()| file.as_file().sync_all())
                    .map_err(text)?;
                file.persist_noclobber(&path).map_err(text)?;
                let message = format!(
                    "{reason}. Conflicting edits were preserved in {}",
                    path.display()
                );
                transaction.execute(
                    "INSERT INTO last_sync_conflict VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET message = excluded.message",
                    [message],
                ).map_err(text)?;
                for sequence in conflicts.into_keys() {
                    transaction
                        .execute("DELETE FROM pending_edits WHERE sequence = ?1", [sequence])
                        .map_err(text)?;
                }
            }
            transaction
                .execute(
                    "UPDATE shared_project SET draft = ?1 WHERE id = 1",
                    [rebased],
                )
                .map_err(text)?;
            transaction.commit().map_err(text)?;
        }
        let conflict = self
            .sqlite
            .query_row(
                "SELECT message FROM last_sync_conflict WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(text)?;
        Ok(WorkspaceUpdate {
            revision,
            draft_changed,
            participants: update.participants,
            conflict,
        })
    }

    pub fn upload_pending(&self, client: &ProjectClient, cache: &MediaCache) -> Result<(), String> {
        let mut query = self
            .sqlite
            .prepare("SELECT digest, length FROM project_media WHERE pending = 1")
            .map_err(text)?;
        let rows = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(text)?;
        let uploads: Vec<_> = rows.collect::<Result<_, _>>().map_err(text)?;
        for (digest, length) in uploads {
            let blob = MediaBlob {
                digest,
                length: u64::try_from(length).map_err(text)?,
            };
            let path = self.upload_directory.join(&blob.digest);
            client
                .upload_blob(self.link.project_id, &path, &blob)
                .map_err(text)?;
            if let Err(error) = client.keep_uploaded(self.link.project_id, &blob, &path, cache) {
                eprintln!("Could not retain imported media in the local cache: {error}");
            }
            self.sqlite
                .execute(
                    "UPDATE project_media SET pending = 0 WHERE digest = ?1",
                    [&blob.digest],
                )
                .map_err(text)?;
            // The complete asset is now served from immutable remote storage.
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn has_pending_uploads(&self) -> Result<bool, String> {
        self.sqlite
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_media WHERE pending = 1)",
                [],
                |row| row.get(0),
            )
            .map_err(text)
    }

    fn record_media(&self, client: &ProjectClient, project: &Project) -> Result<(), String> {
        for asset in project.assets() {
            let digest =
                crate::blob_digest(asset.path()).ok_or("invalid shared media reference")?;
            if self.media_length(digest)?.is_none() {
                let blob = client
                    .media_blob(self.link.project_id, digest)
                    .map_err(text)?;
                self.sqlite.execute(
                    "INSERT INTO project_media VALUES (?1, ?2, 0) ON CONFLICT(digest) DO NOTHING",
                    params![blob.digest, i64::try_from(blob.length).map_err(text)?],
                ).map_err(text)?;
            }
        }
        Ok(())
    }

    fn media_length(&self, digest: &str) -> Result<Option<u64>, String> {
        self.sqlite
            .query_row(
                "SELECT length FROM project_media WHERE digest = ?1",
                [digest],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(text)?
            .map(|length| u64::try_from(length).map_err(text))
            .transpose()
    }

    pub fn media_cache(&self) -> Result<MediaCache, String> {
        const CACHE_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
        let path = shrimply_path_core::cache_directory()?
            .join("shared-media")
            .join(&self.key);
        MediaCache::new(path, CACHE_BUDGET).map_err(text)
    }

    pub fn media_client(&self) -> Result<(ProjectClient, Option<String>), String> {
        let server = &self.link.server;
        match ProjectClient::configured(server) {
            Ok(client) => Ok((client, None)),
            Err(error) => Ok((ProjectClient::new(server, String::new())?, Some(error))),
        }
    }

    pub fn bind_media(
        &self,
        project: &mut Project,
        client: &ProjectClient,
        cache: &MediaCache,
    ) -> Result<(), String> {
        let mut result = Ok(());
        project.visit_assets_mut(|asset| {
            if result.is_err() || asset.path().as_os_str().is_empty() {
                return;
            }
            result = (|| {
                let digest = crate::blob_digest(asset.path())
                    .ok_or("invalid shared media reference")?
                    .to_owned();
                let length = self
                    .media_length(&digest)?
                    .ok_or("shared media length is unavailable")?;
                let staged = self.upload_directory.join(&digest);
                let blob = MediaBlob { digest, length };
                let client = client.clone();
                let cache = cache.clone();
                let project_id = self.link.project_id;
                asset.bind_reader(length, move || match fs::File::open(&staged) {
                    Ok(file) => Ok(Box::new(file)),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        let reader = client
                            .open_media(project_id, blob.clone(), cache.clone())
                            .map_err(io::Error::other)?;
                        Ok(Box::new(reader))
                    }
                    Err(error) => Err(error),
                });
                Ok(())
            })();
        });
        result
    }

    pub fn confirmed_revision(&self) -> Result<u64, String> {
        confirmed_revision(&self.sqlite)
    }
}

/// Freeze imported data before its stable references are committed to the outbox.
pub fn prepare_uploads(
    directory: &Path,
    project: &mut Project,
) -> Result<(Vec<MediaBlob>, MediaRewrites), String> {
    let mut uploads = BTreeMap::<PathBuf, MediaBlob>::new();
    let mut rewrites = MediaRewrites::new();
    let mut result = Ok(());
    project.visit_assets_mut(|asset| {
        if result.is_err()
            || asset.path().as_os_str().is_empty()
            || crate::blob_digest(asset.path()).is_some()
        {
            return;
        }
        let source = asset.path().to_path_buf();
        if source
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                ["blend", "sanim", "py"]
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
        {
            result = Err(format!(
                "Shared source projects require a media export before importing {}",
                source.display()
            ));
            return;
        }
        let blob = uploads
            .get(&source)
            .cloned()
            .map_or_else(|| stage_media(directory, &source), Ok);
        match blob {
            Ok(blob) => {
                let reference = blob.reference_path(&source);
                *asset = reference.clone().into();
                rewrites.insert(source.clone(), reference);
                uploads.insert(source, blob);
            }
            Err(error) => result = Err(error),
        }
    });
    result?;
    Ok((uploads.into_values().collect(), rewrites))
}

fn stage_media(directory: &Path, source: &Path) -> Result<MediaBlob, String> {
    fs::create_dir_all(directory).map_err(text)?;
    let source = fs::File::open(source).map_err(text)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(text)?;
    let (length, digest) = fingerprint::copy(source, &mut temporary).map_err(text)?;
    temporary.as_file().sync_all().map_err(text)?;
    let destination = directory.join(&digest);
    if !destination.is_file() {
        temporary.persist(destination).map_err(text)?;
    }
    Ok(MediaBlob { digest, length })
}

fn confirmed_revision(connection: &Connection) -> Result<u64, String> {
    let stored = connection
        .query_row(
            "SELECT revision FROM shared_project WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(text)?;
    u64::try_from(stored).map_err(text)
}

fn draft(connection: &Connection) -> Result<Project, String> {
    let bytes = connection
        .query_row("SELECT draft FROM shared_project WHERE id = 1", [], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(text)?;
    ProjectEncoding::Shrimp.decode(&bytes)
}

fn confirmed(connection: &Connection) -> Result<Project, String> {
    let snapshot = connection
        .query_row(
            "SELECT document FROM shared_project WHERE id = 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(text)?;
    ProjectEncoding::Shrimp.decode(&snapshot)
}

fn publish_edit(
    client: &ProjectClient,
    project_id: Uuid,
    head: &mut ProjectVersion,
    request: &ChangeRequest,
) -> Result<Result<(), String>, String> {
    const MAX_ATTEMPTS: usize = 4;
    let operation_digest =
        fingerprint::digest(&serde_json::to_vec(&request.changes).map_err(text)?);
    for _ in 0..MAX_ATTEMPTS {
        let project = match request.changes.apply(&head.project) {
            Ok(project) => project,
            Err(error) => {
                // A response may have been lost after the server committed this edit.
                let Some(receipt) = client
                    .receipt(project_id, request.operation_id)
                    .map_err(text)?
                else {
                    return Ok(Err(text(error)));
                };
                if receipt.revision > head.revision {
                    refresh_head(client, project_id, head, receipt.revision)?;
                }
                return Ok(Ok(()));
            }
        };
        let media: BTreeSet<_> = project
            .assets()
            .into_iter()
            .map(|asset| {
                crate::blob_digest(asset.path())
                    .map(str::to_owned)
                    .ok_or("shared project assets must reference immutable project media")
            })
            .collect::<Result<_, _>>()?;
        let submission = PublishRequest {
            operation_id: request.operation_id,
            operation_digest: operation_digest.clone(),
            base_revision: head.revision,
            project: &project,
            media: media.into_iter().collect(),
        };
        match client.publish(project_id, &submission) {
            Ok(receipt) => {
                if head.revision.checked_add(1) == Some(receipt.revision) {
                    head.revision = receipt.revision;
                    head.project = Arc::new(project);
                } else if receipt.revision > head.revision {
                    refresh_head(client, project_id, head, receipt.revision)?;
                }
                return Ok(Ok(()));
            }
            Err(RequestError::Rejected {
                revision: Some(revision),
                ..
            }) => {
                refresh_head(client, project_id, head, revision)?;
            }
            Err(error) => return Err(text(error)),
        }
    }
    Err("Shared project changed during synchronization; pending edits will retry".into())
}

fn refresh_head(
    client: &ProjectClient,
    id: Uuid,
    head: &mut ProjectVersion,
    required_revision: u64,
) -> Result<(), String> {
    let latest = client.get(id).map_err(text)?;
    require_revision(latest.revision, required_revision.max(head.revision))?;
    *head = latest;
    Ok(())
}

fn require_revision(revision: u64, minimum: u64) -> Result<(), String> {
    if revision < minimum {
        return Err(format!(
            "Shared project server returned revision {revision}, expected at least {minimum}"
        ));
    }
    Ok(())
}

fn rebase(
    connection: &Connection,
    conflicts: &mut BTreeMap<i64, String>,
) -> Result<Vec<u8>, String> {
    let mut project = confirmed(connection)?;
    for (sequence, request) in pending(connection)? {
        if let std::collections::btree_map::Entry::Vacant(entry) = conflicts.entry(sequence) {
            match request.changes.apply(&project) {
                Ok(next) => project = next,
                Err(error) => {
                    entry.insert(text(error));
                }
            }
        }
    }
    ProjectEncoding::Shrimp.encode(&project)
}

fn pending(connection: &Connection) -> Result<Vec<(i64, ChangeRequest)>, String> {
    let mut query = connection
        .prepare("SELECT sequence, operation FROM pending_edits ORDER BY sequence")
        .map_err(text)?;
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(text)?;
    rows.map(|row| {
        let (sequence, operation) = row.map_err(text)?;
        Ok((sequence, serde_json::from_slice(&operation).map_err(text)?))
    })
    .collect()
}

fn configure(connection: &Connection) -> Result<(), String> {
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(text)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS attached_receipts (operation TEXT UNIQUE NOT NULL, rewrites BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS last_sync_conflict (id INTEGER PRIMARY KEY CHECK(id = 1), message TEXT NOT NULL);"
        )
        .map_err(text)
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
