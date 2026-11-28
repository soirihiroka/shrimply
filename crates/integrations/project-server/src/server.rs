use std::{
    convert::Infallible,
    io,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
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
use ring::hmac;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::{
    net::TcpListener,
    sync::{RwLock, Semaphore},
};
use uuid::Uuid;

use crate::media::BlobStore;
use crate::store::{Failure, Presence, Store};
use shrimply_project_protocol::{
    ErrorResponse, PROJECT_SERVER_NAME_HEADER, ProjectVersionReceipt, PublishRequest, SignIn,
    SyncRequest, UPLOAD_CHUNK_BYTES,
};

pub(super) type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, io::Error>;

const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

struct Authority {
    version: shrimply_project_protocol::ServerVersion,
    store: Mutex<Store>,
    blobs: Arc<BlobStore>,
    media_lifecycle: Arc<RwLock<()>>,
    signing_key: hmac::Key,
    password: hmac::Tag,
    session: hmac::Tag,
    sign_in: SignIn,
    login_attempts: Semaphore,
}

pub async fn serve(
    address: SocketAddr,
    database: &Path,
    username: String,
    password: String,
) -> Result<(), String> {
    if username.trim().is_empty() || username.contains(':') || password.is_empty() {
        return Err("set a username without a colon and a nonempty password".into());
    }
    let blobs = Arc::new(BlobStore::new(database));
    let store = Store::open(database, blobs.clone()).map_err(|error| format!("{error:?}"))?;
    let signing_key = store.signing_key().map_err(|error| format!("{error:?}"))?;
    let password_tag = hmac::sign(
        &signing_key,
        format!(
            "Basic {}",
            STANDARD.encode(format!("{username}:{password}"))
        )
        .as_bytes(),
    );
    drop(password);
    let token = URL_SAFE_NO_PAD.encode(password_tag.as_ref());
    let session = hmac::sign(&signing_key, format!("Bearer {token}").as_bytes());
    let authority = Arc::new(Authority {
        version: shrimply_project_protocol::ServerVersion {
            protocol: shrimply_project_protocol::SERVER_PROTOCOL,
            version: Some(env!("CARGO_PKG_VERSION").into()),
            name: std::env::var("SHRIMPLY_PROJECT_SERVER_NAME")
                .ok()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| {
                    let hostname = std::process::Command::new("hostname").output().ok()?;
                    hostname
                        .status
                        .success()
                        .then(|| String::from_utf8_lossy(&hostname.stdout).into_owned())
                })
                .map(|name| {
                    name.trim()
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(80)
                        .collect::<String>()
                })
                .filter(|name| !name.is_empty()),
        },
        store: Mutex::new(store),
        blobs,
        media_lifecycle: Arc::new(RwLock::new(())),
        signing_key,
        password: password_tag,
        session,
        sign_in: SignIn { token },
        login_attempts: Semaphore::new(4),
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
    let path = request.uri().path();
    if request.method() == Method::GET && path == "/v1" {
        return Ok(json(StatusCode::OK, &authority.version));
    }
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .map_or(&[][..], header::HeaderValue::as_bytes);
    let login = request.method() == Method::POST && path == "/v1/login";
    let attempt = if login {
        match authority.login_attempts.try_acquire() {
            Ok(attempt) => Some(attempt),
            Err(_) => {
                return Ok(json(
                    StatusCode::TOO_MANY_REQUESTS,
                    ErrorResponse {
                        message: "Too many sign-in attempts. Try again in a moment".into(),
                        revision: None,
                    },
                ));
            }
        }
    } else {
        None
    };
    let credential = if login {
        &authority.password
    } else {
        &authority.session
    };
    if hmac::verify(&authority.signing_key, authorization, credential.as_ref()).is_err() {
        if login {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        return Ok(json(
            StatusCode::UNAUTHORIZED,
            ErrorResponse {
                message: if login {
                    "Incorrect project server username or password"
                } else {
                    "Sign in to this project server again"
                }
                .into(),
                revision: None,
            },
        ));
    }
    if login {
        let mut response = json(StatusCode::OK, &authority.sign_in);
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        return Ok(response);
    }
    drop(attempt);
    if request.method() == Method::GET && path == "/v1/projects" {
        let projects = transact(authority.clone(), |store| store.list()).await?;
        let mut response = json(StatusCode::OK, projects);
        if let Some(name) = &authority.version.name
            && let Ok(name) = header::HeaderValue::from_bytes(name.as_bytes())
        {
            response
                .headers_mut()
                .insert(PROJECT_SERVER_NAME_HEADER, name);
        }
        return Ok(response);
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
        (&Method::DELETE, "") => {
            let guard = authority.media_lifecycle.clone().write_owned().await;
            let blobs = authority.blobs.clone();
            transact(authority, move |store| {
                let _guard = guard;
                store.delete(id)?;
                blobs.remove_project(id)
            })
            .await?;
            Ok(json(StatusCode::OK, true))
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
            let mut sync = transact(authority.clone(), move |store| {
                store.sync(
                    id,
                    request.revision,
                    request.presence.into_iter().chain(request.participants),
                )
            })
            .await?;
            sync.server_version = Some(authority.version.clone());
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
    if request.method() != Method::PATCH {
        transact(authority.clone(), move |store| store.exists(project)).await?;
    }
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
            let guard = authority.media_lifecycle.clone().read_owned().await;
            transact(authority.clone(), move |store| store.exists(project)).await?;
            let receipt = tokio::task::spawn_blocking(move || {
                let _guard = guard;
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
