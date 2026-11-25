use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    Response, StatusCode,
    body::{Bytes, Frame},
    header,
};
use shrimply_resource_pipeline::fingerprint::Writer;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{UploadReceipt, server::Body, store::Failure};

pub(super) struct BlobStore {
    directory: PathBuf,
    uploads: Vec<Mutex<()>>,
}

impl BlobStore {
    pub(super) fn new(database: &Path) -> Self {
        let mut directory = database.as_os_str().to_os_string();
        directory.push(".media");
        Self {
            directory: directory.into(),
            uploads: (0..16).map(|_| Mutex::new(())).collect(),
        }
    }

    pub(super) fn blob(&self, project: Uuid, digest: &str) -> Result<PathBuf, Failure> {
        if !crate::valid_digest(digest) {
            return Err(Failure::BadRequest(
                "invalid SHA-256 media identifier".into(),
            ));
        }
        Ok(self.directory.join(project.to_string()).join(digest))
    }

    pub(super) fn is_complete(&self, project: Uuid, digest: &str) -> Result<bool, Failure> {
        Ok(file_length(&self.blob(project, digest)?)?.is_some())
    }

    pub(super) fn progress(&self, project: Uuid, digest: &str) -> Result<UploadReceipt, Failure> {
        let path = self.blob(project, digest)?;
        let _upload = self.lock(digest);
        Self::progress_at_path(&path)
    }

    fn progress_at_path(path: &Path) -> Result<UploadReceipt, Failure> {
        if let Some(offset) = file_length(path)? {
            return Ok(UploadReceipt {
                offset,
                complete: true,
            });
        }
        let offset = file_length(&path.with_extension("upload"))?.unwrap_or(0);
        Ok(UploadReceipt {
            offset,
            complete: false,
        })
    }

    pub(super) fn append(
        &self,
        project: Uuid,
        digest: &str,
        offset: u64,
        total: u64,
        bytes: &[u8],
    ) -> Result<UploadReceipt, Failure> {
        let path = self.blob(project, digest)?;
        let _upload = self.lock(digest);
        let receipt = Self::progress_at_path(&path)?;
        if receipt.offset > total {
            return Err(Failure::BadRequest("upload size changed".into()));
        }
        if receipt.complete || receipt.offset != offset {
            return Ok(receipt);
        }
        let next = offset
            .checked_add(bytes.len() as u64)
            .filter(|next| *next <= total)
            .ok_or_else(|| Failure::BadRequest("upload chunk exceeds media length".into()))?;
        if next == offset && total != offset {
            return Err(Failure::BadRequest("upload chunk is empty".into()));
        }
        let parent = path.parent().expect("asset path has a parent");
        fs::create_dir_all(parent).map_err(storage)?;
        let partial = path.with_extension("upload");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&partial)
            .map_err(storage)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_data())
            .map_err(storage)?;
        drop(file);
        if next != total {
            return Ok(UploadReceipt {
                offset: next,
                complete: false,
            });
        }
        let mut source = fs::File::open(&partial).map_err(storage)?;
        let mut checksum = Writer::default();
        io::copy(&mut source, &mut checksum).map_err(storage)?;
        drop(source);
        if checksum.finish() != digest {
            fs::remove_file(&partial).map_err(storage)?;
            return Err(Failure::BadRequest(
                "media bytes do not match their SHA-256 identifier".into(),
            ));
        }
        fs::rename(&partial, &path).map_err(storage)?;
        #[cfg(unix)]
        if let Err(error) = fs::File::open(parent).and_then(|directory| directory.sync_all()) {
            eprintln!("Could not sync project media directory: {error}");
        }
        Ok(UploadReceipt {
            offset: next,
            complete: true,
        })
    }

    fn lock(&self, digest: &str) -> MutexGuard<'_, ()> {
        let index = usize::from_str_radix(&digest[..1], 16).expect("validated digest");
        self.uploads[index]
            .lock()
            .expect("project upload lock poisoned")
    }

    pub(super) async fn read(
        &self,
        project: Uuid,
        digest: &str,
        range: Option<&header::HeaderValue>,
        headers_only: bool,
    ) -> Result<Response<Body>, Failure> {
        let path = self.blob(project, digest)?;
        let mut file = tokio::fs::File::open(path).await.map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Failure::Missing
            } else {
                storage(error)
            }
        })?;
        let total = file.metadata().await.map_err(storage)?.len();
        let mut response = Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::ETAG, format!("\"{digest}\""))
            .header(header::ACCEPT_RANGES, "bytes");
        let (start, length) = if let Some(range) = range {
            let Some((start, end)) = range
                .to_str()
                .ok()
                .and_then(|value| byte_range(value, total))
            else {
                return Ok(response
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(crate::server::full(Bytes::new()))
                    .expect("media response headers"));
            };
            response = response.status(StatusCode::PARTIAL_CONTENT).header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{total}"),
            );
            (start, end - start + 1)
        } else {
            (0, total)
        };
        let body = if headers_only {
            crate::server::full(Bytes::new())
        } else {
            file.seek(io::SeekFrom::Start(start))
                .await
                .map_err(storage)?;
            StreamBody::new(ReaderStream::new(file.take(length)).map_ok(Frame::data)).boxed_unsync()
        };
        Ok(response
            .header(header::CONTENT_LENGTH, length)
            .body(body)
            .expect("media response headers"))
    }
}

fn file_length(path: &Path) -> Result<Option<u64>, Failure> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage(error)),
    }
}

fn storage(error: io::Error) -> Failure {
    Failure::Storage(error.to_string())
}

fn byte_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let last = total.checked_sub(1)?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(total);
        return (suffix > 0).then_some((total - suffix, last));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        last
    } else {
        end.parse::<u64>().ok()?.min(last)
    };
    (start <= end).then_some((start, end))
}
