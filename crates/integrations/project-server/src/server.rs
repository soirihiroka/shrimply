use std::{
    convert::Infallible,
    io,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header,
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::media::BlobStore;
use crate::store::{Failure, Presence, Store};
use shrimply_project_protocol::{
    ErrorResponse, ProjectVersionReceipt, PublishRequest, SyncRequest, UPLOAD_CHUNK_BYTES,
};

pub(super) type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, io::Error>;

const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

struct Authority {
    store: Mutex<Store>,
    blobs: Arc<BlobStore>,
    credential: String,
}

pub async fn serve(address: SocketAddr, database: &Path, token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("a project server token is required".into());
    }
    let blobs = Arc::new(BlobStore::new(database));
    let authority = Arc::new(Authority {
        store: Mutex::new(
            Store::open(database, blobs.clone()).map_err(|error| format!("{error:?}"))?,
        ),
        blobs,
        credential: format!("Bearer {token}"),
    });
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| error.to_string())?;
    eprintln!(
        "Project server listening on {}",
        listener.local_addr().map_err(|error| error.to_string())?
    );
    loop {
        let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let authority = authority.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let authority = authority.clone();
                async move {
                    Ok::<_, Infallible>(dispatch(authority, request).await.unwrap_or_else(failure))
                }
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

async fn dispatch(
    authority: Arc<Authority>,
    request: Request<Incoming>,
) -> Result<Response<Body>, Failure> {
    let authorization = request.headers().get(header::AUTHORIZATION);
    if authorization.is_none_or(|value| value.as_bytes() != authority.credential.as_bytes()) {
        return Ok(json(
            StatusCode::UNAUTHORIZED,
            ErrorResponse {
                message: "invalid project server credentials".into(),
                revision: None,
            },
        ));
    }
    let path = request.uri().path();
    if request.method() == Method::GET && path == "/v1" {
        return Ok(json(StatusCode::OK, serde_json::json!({"protocol":2})));
    }
    if request.method() == Method::GET && path == "/v1/projects" {
        let projects = transact(authority, |store| store.list()).await?;
        return Ok(json(StatusCode::OK, projects));
    }
    let relative = path.strip_prefix("/v1/projects/").ok_or(Failure::Missing)?;
    let (id, action) = relative.split_once('/').unwrap_or((relative, ""));
    let id = Uuid::parse_str(id).map_err(|_| Failure::Missing)?;
    if let Some(digest) = action.strip_prefix("blobs/") {
        let digest = digest.to_owned();
        return media(authority, request, id, digest).await;
    }
    match (request.method(), action) {
        (&Method::POST, "") => {
            let project: Value = read_json(request).await?;
            let created = transact(authority, move |store| store.create(id, project)).await?;
            Ok(json(StatusCode::CREATED, created))
        }
        (&Method::GET, "") => {
            let version = transact(authority, move |store| store.get(id)).await?;
            Ok(json(StatusCode::OK, version))
        }
        (&Method::GET, path) if path.starts_with("operations/") => {
            let operation = Uuid::parse_str(path.trim_start_matches("operations/"))
                .map_err(|_| Failure::Missing)?;
            let revision = transact(authority, move |store| store.receipt(id, operation))
                .await?
                .ok_or(Failure::Missing)?;
            Ok(json(StatusCode::OK, ProjectVersionReceipt { revision }))
        }
        (&Method::POST, "snapshots") => {
            let request: PublishRequest<Value> = read_json(request).await?;
            let revision = transact(authority, move |store| store.publish(id, request)).await?;
            Ok(json(StatusCode::OK, ProjectVersionReceipt { revision }))
        }
        (&Method::POST, "sync") => {
            let request: SyncRequest<Presence> = read_json(request).await?;
            let sync = transact(authority, move |store| {
                store.sync(
                    id,
                    request.revision,
                    request.presence.into_iter().chain(request.participants),
                )
            })
            .await?;
            Ok(json(StatusCode::OK, sync))
        }
        _ => Err(Failure::Missing),
    }
}

async fn read_body(request: Request<Incoming>, limit: usize) -> Result<Bytes, Failure> {
    Ok(Limited::new(request.into_body(), limit)
        .collect()
        .await
        .map_err(|error| Failure::BadRequest(error.to_string()))?
        .to_bytes())
}

async fn read_json<T: DeserializeOwned>(request: Request<Incoming>) -> Result<T, Failure> {
    serde_json::from_slice(&read_body(request, MAX_DOCUMENT_BYTES).await?)
        .map_err(|error| Failure::BadRequest(error.to_string()))
}

async fn media(
    authority: Arc<Authority>,
    request: Request<Incoming>,
    project: Uuid,
    digest: String,
) -> Result<Response<Body>, Failure> {
    transact(authority.clone(), move |store| store.exists(project)).await?;
    if let Some(progress) = digest.strip_suffix("/upload")
        && request.method() == Method::GET
    {
        let progress = progress.to_owned();
        let receipt =
            tokio::task::spawn_blocking(move || authority.blobs.progress(project, &progress))
                .await
                .map_err(|error| Failure::Storage(error.to_string()))??;
        return Ok(json(StatusCode::OK, receipt));
    }
    match *request.method() {
        Method::GET | Method::HEAD => {
            authority
                .blobs
                .read(
                    project,
                    &digest,
                    request.headers().get(header::RANGE),
                    request.method() == Method::HEAD,
                )
                .await
        }
        Method::PATCH => {
            let number = |key| {
                request
                    .headers()
                    .get(key)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(|| Failure::BadRequest(format!("invalid {key} header")))
            };
            let offset = number("upload-offset")?;
            let length = number("upload-length")?;
            let bytes = read_body(request, UPLOAD_CHUNK_BYTES).await?;
            let receipt = tokio::task::spawn_blocking(move || {
                authority
                    .blobs
                    .append(project, &digest, offset, length, &bytes)
            })
            .await
            .map_err(|error| Failure::Storage(error.to_string()))??;
            Ok(json(StatusCode::OK, receipt))
        }
        _ => Err(Failure::Missing),
    }
}

async fn transact<T: Send + 'static>(
    authority: Arc<Authority>,
    work: impl FnOnce(&mut Store) -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(move || {
        let mut store = authority
            .store
            .lock()
            .expect("project server store poisoned");
        work(&mut store)
    })
    .await
    .map_err(|error| Failure::Storage(error.to_string()))?
}

fn failure(error: Failure) -> Response<Body> {
    let (status, message, revision) = match error {
        Failure::BadRequest(message) => (StatusCode::BAD_REQUEST, message, None),
        Failure::Conflict(revision) => (
            StatusCode::CONFLICT,
            "project revision conflict".into(),
            Some(revision),
        ),
        Failure::Missing => (
            StatusCode::NOT_FOUND,
            "project or route not found".into(),
            None,
        ),
        Failure::Storage(error) => {
            eprintln!("Project server storage error: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "project storage error".into(),
                None,
            )
        }
    };
    json(status, ErrorResponse { message, revision })
}

fn json(status: StatusCode, value: impl Serialize) -> Response<Body> {
    let bytes = serde_json::to_vec(&value).expect("project protocol value must be serializable");
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(full(Bytes::from(bytes)))
        .expect("static project response headers")
}

pub(super) fn full(bytes: Bytes) -> Body {
    Full::new(bytes)
        .map_err(|error| match error {})
        .boxed_unsync()
}
