use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Method, Url,
    blocking::{Client, RequestBuilder, Response},
    header,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use shrimply_project_document::project::{Project, ProjectEncoding};
use uuid::Uuid;

use shrimply_resource_pipeline::fingerprint::Writer;

use crate::{MediaCache, MediaReader, ProjectVersion, SyncRequest, SyncResponse};
use shrimply_project_protocol::{
    ErrorResponse, LEGACY_CHECKPOINT_FIELD, MediaBlob, ProjectSummary, ProjectVersionReceipt,
    PublishRequest, ServerVersion, SignIn, UPLOAD_CHUNK_BYTES, UploadReceipt, valid_digest,
};

#[derive(Debug)]
pub enum RequestError {
    Transport(reqwest::Error),
    Media(io::Error),
    InvalidProject(String),
    Rejected {
        message: String,
        revision: Option<u64>,
    },
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(formatter),
            Self::Media(error) => error.fmt(formatter),
            Self::InvalidProject(error) => formatter.write_str(error),
            Self::Rejected {
                message,
                revision: None,
            } => formatter.write_str(message),
            Self::Rejected {
                message,
                revision: Some(revision),
            } => write!(formatter, "{message}; shared revision is {revision}"),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Clone)]
pub struct ProjectClient {
    client: Client,
    base: Url,
    token: String,
}

impl ProjectClient {
    pub fn configured(base: &str) -> Result<Self, String> {
        let mut client = Self::new(base, String::new())?;
        client.token = std::fs::read_to_string(client.session_path()).map_err(
            |_| "Sign in to this project server in Project Servers or with shrimply project login",
        )?;
        Ok(client)
    }

    fn session_path(&self) -> std::path::PathBuf {
        shrimply_path_core::config_directory()
            .join("project-server-sessions")
            .join(shrimply_resource_pipeline::fingerprint::digest(
                self.base.as_str().trim_end_matches('/').as_bytes(),
            ))
    }

    pub fn sign_in(base: &str, username: &str, password: &str) -> Result<(), String> {
        use std::io::Write as _;
        if username.trim().is_empty() || username.contains(':') || password.is_empty() {
            return Err("Enter a username without a colon and a password".into());
        }
        let client = Self::new(base, String::new())?;
        let mut url = client.api_url();
        url.path_segments_mut()
            .expect("project server URL has a path")
            .push("login");
        let SignIn { token } = response(
            client
                .client
                .post(url)
                .basic_auth(username, Some(password))
                .send(),
        )
        .map_err(|error| error.to_string())?;
        if token.is_empty()
            || token.chars().any(char::is_whitespace)
            || header::HeaderValue::from_str(&token).is_err()
        {
            return Err("The project server returned an invalid session".into());
        }
        let path = client.session_path();
        let directory = path.parent().expect("project server session directory");
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let mut staged =
            tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
        staged
            .write_all(token.as_bytes())
            .map_err(|error| error.to_string())?;
        staged
            .persist(path)
            .map_err(|error| error.error.to_string())?;
        Ok(())
    }

    pub(crate) fn forget(base: &str) -> Result<(), String> {
        match std::fs::remove_file(Self::new(base, String::new())?.session_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn new(base: &str, token: String) -> Result<Self, String> {
        let base = Url::parse(base).map_err(|error| error.to_string())?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("project server URL must be a plain HTTP or HTTPS base URL".into());
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            base,
            token,
        })
    }

    pub fn create(&self, id: Uuid, project: &Project) -> Result<ProjectVersion, RequestError> {
        if !project.assets().is_empty() {
            return Err(invalid_media(
                "A shared project must be created before uploading media",
            ));
        }
        let mut project = project.clone();
        project.clear_view_state();
        project.ensure_ids();
        project.validate().map_err(RequestError::InvalidProject)?;
        response(
            self.request(Method::POST, Some(id), &[])
                .json(&project)
                .send(),
        )
        .and_then(decode_version)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>, RequestError> {
        response(self.request(Method::GET, None, &[]).send())
    }

    pub fn version(&self) -> Result<ServerVersion, RequestError> {
        response(
            self.client
                .get(self.api_url())
                .timeout(Duration::from_secs(3))
                .send(),
        )
    }

    pub fn get(&self, id: Uuid) -> Result<ProjectVersion, RequestError> {
        response(self.request(Method::GET, Some(id), &[]).send()).and_then(decode_version)
    }

    pub fn delete(&self, id: Uuid) -> Result<(), RequestError> {
        response::<bool>(self.request(Method::DELETE, Some(id), &[]).send()).map(|_| ())
    }

    pub(super) fn publish(
        &self,
        id: Uuid,
        request: &PublishRequest<&Project>,
    ) -> Result<ProjectVersionReceipt, RequestError> {
        response(
            self.request(Method::POST, Some(id), &["snapshots"])
                .json(request)
                .send(),
        )
    }

    pub(super) fn receipt(
        &self,
        id: Uuid,
        operation: Uuid,
    ) -> Result<Option<ProjectVersionReceipt>, RequestError> {
        let operation = operation.to_string();
        let reply = self
            .request(Method::GET, Some(id), &["operations", &operation])
            .send()
            .map_err(RequestError::Transport)?;
        if reply.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        response(Ok(reply)).map(Some)
    }

    pub(super) fn sync(
        &self,
        id: Uuid,
        request: &SyncRequest,
    ) -> Result<SyncResponse, RequestError> {
        let reply: shrimply_project_protocol::SyncResponse<Value, crate::Presence> = response(
            self.request(Method::POST, Some(id), &["sync"])
                .json(request)
                .send(),
        )?;
        Ok(SyncResponse {
            version: reply.version.map(decode_version).transpose()?,
            participants: reply.participants,
        })
    }

    pub fn media_blob(&self, id: Uuid, digest: &str) -> Result<MediaBlob, RequestError> {
        if !valid_digest(digest) {
            return Err(invalid_media("invalid media identifier"));
        }
        let response = self
            .request(Method::HEAD, Some(id), &["blobs", digest])
            .send()
            .map_err(RequestError::Transport)?;
        let etag = format!("\"{digest}\"");
        if !response.status().is_success()
            || response
                .headers()
                .get(header::ETAG)
                .map(|value| value.as_bytes())
                != Some(etag.as_bytes())
        {
            return Err(invalid_media(
                "project server did not identify the requested media revision",
            ));
        }
        let length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| invalid_media("project server did not report the media length"))?;
        Ok(MediaBlob {
            digest: digest.to_owned(),
            length,
        })
    }

    pub fn open_media(
        &self,
        id: Uuid,
        blob: MediaBlob,
        cache: MediaCache,
    ) -> Result<MediaReader, RequestError> {
        MediaReader::new(self.clone(), self.base.as_str(), id, blob, cache)
    }

    pub(super) fn keep_uploaded(
        &self,
        id: Uuid,
        blob: &MediaBlob,
        source: &Path,
        cache: &MediaCache,
    ) -> io::Result<()> {
        cache.keep_uploaded(self.base.as_str(), id, blob, source)
    }
    pub fn upload_file(&self, id: Uuid, path: &Path) -> Result<MediaBlob, RequestError> {
        let mut file = File::open(path).map_err(RequestError::Media)?;
        let mut hash = Writer::default();
        let length = io::copy(&mut file, &mut hash).map_err(RequestError::Media)?;
        let blob = MediaBlob {
            digest: hash.finish(),
            length,
        };
        self.upload_blob(id, path, &blob)?;
        Ok(blob)
    }

    pub(crate) fn upload_blob(
        &self,
        id: Uuid,
        path: &Path,
        blob: &MediaBlob,
    ) -> Result<(), RequestError> {
        let mut file = None;
        let length = blob.length;
        let segments = ["blobs", &blob.digest];
        let mut progress: UploadReceipt = response(
            self.request(Method::GET, Some(id), &["blobs", &blob.digest, "upload"])
                .send(),
        )?;
        while !progress.complete {
            let offset = progress.offset;
            let chunk = blob
                .length
                .checked_sub(offset)
                .ok_or_else(|| invalid_media("upload progress exceeds the file length"))?
                .min(UPLOAD_CHUNK_BYTES as u64) as usize;
            let file = match &mut file {
                Some(file) => file,
                slot @ None => slot.insert(File::open(path).map_err(RequestError::Media)?),
            };
            file.seek(SeekFrom::Start(offset))
                .map_err(RequestError::Media)?;
            let mut bytes = vec![0; chunk];
            file.read_exact(&mut bytes).map_err(RequestError::Media)?;
            progress = response(
                self.request(Method::PATCH, Some(id), &segments)
                    .header("Upload-Offset", offset)
                    .header("Upload-Length", length)
                    .body(bytes)
                    .send(),
            )?;
            if progress.offset <= offset && !progress.complete {
                return Err(invalid_media("project server did not advance the upload"));
            }
        }
        if progress.offset != blob.length {
            return Err(invalid_media(
                "uploaded media length does not match the local file",
            ));
        }
        Ok(())
    }

    /// Fetch one bounded range of the immutable remote media revision.
    pub fn read_range(
        &self,
        id: Uuid,
        blob: &MediaBlob,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, RequestError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let end = offset
            .checked_add(length as u64 - 1)
            .filter(|end| *end < blob.length && length <= UPLOAD_CHUNK_BYTES)
            .ok_or_else(|| invalid_media("invalid media byte range"))?;
        let response = self
            .request(Method::GET, Some(id), &["blobs", &blob.digest])
            .header(header::RANGE, format!("bytes={offset}-{end}"))
            .send()
            .map_err(RequestError::Transport)?;
        let content_range = format!("bytes {offset}-{end}/{}", blob.length);
        let etag = format!("\"{}\"", blob.digest);
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(header::CONTENT_RANGE)
                .map(|value| value.as_bytes())
                != Some(content_range.as_bytes())
            || response
                .headers()
                .get(header::ETAG)
                .map(|value| value.as_bytes())
                != Some(etag.as_bytes())
        {
            return Err(invalid_media(
                "project server returned a different media range or revision",
            ));
        }
        let mut bytes = Vec::with_capacity(length);
        response
            .take(length as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(RequestError::Media)?;
        if bytes.len() != length {
            return Err(invalid_media(
                "project server returned an incomplete media range",
            ));
        }
        Ok(bytes)
    }

    fn request(&self, method: Method, id: Option<Uuid>, action: &[&str]) -> RequestBuilder {
        let mut url = self.api_url();
        {
            let mut path = url
                .path_segments_mut()
                .expect("project server URL has a path");
            path.push("projects");
            if let Some(id) = id {
                path.push(&id.to_string());
            }
            path.extend(action.iter().copied());
        }
        self.client.request(method, url).bearer_auth(&self.token)
    }

    fn api_url(&self) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("project server URL has a path")
            .pop_if_empty()
            .push("v1");
        url
    }
}

fn decode_version(
    version: shrimply_project_protocol::ProjectVersion<Value>,
) -> Result<ProjectVersion, RequestError> {
    let value = std::sync::Arc::unwrap_or_clone(version.project);
    let project: Project = if let Some(legacy) = value.get(LEGACY_CHECKPOINT_FIELD) {
        let bytes = STANDARD
            .decode(legacy.as_str().ok_or_else(|| {
                RequestError::InvalidProject("invalid legacy project checkpoint".into())
            })?)
            .map_err(|error| RequestError::InvalidProject(error.to_string()))?;
        ProjectEncoding::Shrimp
            .decode(&bytes)
            .map_err(RequestError::InvalidProject)?
    } else {
        serde_json::from_value(value)
            .map_err(|error| RequestError::InvalidProject(error.to_string()))?
    };
    project.validate().map_err(RequestError::InvalidProject)?;
    Ok(ProjectVersion {
        revision: version.revision,
        project: std::sync::Arc::new(project),
    })
}

fn response<T: DeserializeOwned>(
    response: Result<Response, reqwest::Error>,
) -> Result<T, RequestError> {
    let response = response.map_err(RequestError::Transport)?;
    let status = response.status();
    if status.is_success() {
        return response.json().map_err(RequestError::Transport);
    }
    let rejected = response.json::<ErrorResponse>();
    Err(match rejected {
        Ok(error) => RequestError::Rejected {
            message: error.message,
            revision: error.revision,
        },
        Err(_) => RequestError::Rejected {
            message: format!("project server returned {status}"),
            revision: None,
        },
    })
}

fn invalid_media(message: &'static str) -> RequestError {
    RequestError::Media(io::Error::new(io::ErrorKind::InvalidData, message))
}
