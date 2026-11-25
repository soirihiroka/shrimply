use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

use reqwest::{
    Method, Url,
    blocking::{Client, RequestBuilder, Response},
    header,
};
use serde::de::DeserializeOwned;
use shrimply_project_document::project::Project;
use uuid::Uuid;

use shrimply_resource_pipeline::fingerprint::Writer;

use crate::{
    ChangeRequest, CommitRequest, ErrorResponse, MediaBlob, MediaCache, MediaReader,
    ProjectVersion, ProjectVersionReceipt, SyncRequest, SyncResponse, UPLOAD_CHUNK_BYTES,
    UploadReceipt,
};

#[derive(Debug)]
pub enum RequestError {
    Transport(reqwest::Error),
    Media(io::Error),
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
        let token = std::env::var("SHRIMPLY_PROJECT_SERVER_TOKEN")
            .map_err(|_| "set SHRIMPLY_PROJECT_SERVER_TOKEN to connect to a project server")?;
        Self::new(base, token)
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
        response(self.request(Method::POST, id, &[]).json(project).send())
    }

    pub fn get(&self, id: Uuid) -> Result<ProjectVersion, RequestError> {
        response(self.request(Method::GET, id, &[]).send())
    }

    pub fn commit(
        &self,
        id: Uuid,
        request: &CommitRequest,
    ) -> Result<ProjectVersionReceipt, RequestError> {
        response(
            self.request(Method::POST, id, &["edits"])
                .json(request)
                .send(),
        )
    }

    pub fn changes(
        &self,
        id: Uuid,
        request: &ChangeRequest,
    ) -> Result<ProjectVersionReceipt, RequestError> {
        response(
            self.request(Method::POST, id, &["changes"])
                .json(request)
                .send(),
        )
    }

    pub fn sync(&self, id: Uuid, request: &SyncRequest) -> Result<SyncResponse, RequestError> {
        response(
            self.request(Method::POST, id, &["sync"])
                .json(request)
                .send(),
        )
    }

    pub fn media_blob(&self, id: Uuid, digest: &str) -> Result<MediaBlob, RequestError> {
        if !crate::valid_digest(digest) {
            return Err(invalid_media("invalid media identifier"));
        }
        let response = self
            .request(Method::HEAD, id, &["blobs", digest])
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
            self.request(Method::GET, id, &["blobs", &blob.digest, "upload"])
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
                self.request(Method::PATCH, id, &segments)
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
            .request(Method::GET, id, &["blobs", &blob.digest])
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

    fn request(&self, method: Method, id: Uuid, action: &[&str]) -> RequestBuilder {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("project server URL has a path")
            .pop_if_empty()
            .extend(["v1", "projects", &id.to_string()])
            .extend(action.iter().copied());
        self.client.request(method, url).bearer_auth(&self.token)
    }
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
