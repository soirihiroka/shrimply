//! The editor and headless media loader share bounded HTTP image responses.
use reqwest::{blocking::Client, header};
use shrimply_resource_pipeline::cache::Cache;
use std::{io::Read, sync::{Arc, Mutex, OnceLock}, time::{Duration, Instant}};

const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(30);
const MAX_MAX_AGE: u64 = 86_400;

#[derive(Clone)]
pub struct Download {
    pub bytes: Arc<Vec<u8>>,
    pub fingerprint: String,
    pub content_type: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    expires: Instant,
    max_age: Duration,
}
struct Slot(Mutex<Option<Arc<Download>>>);
type Responses = Cache<String, Slot>;
fn cache() -> &'static Mutex<Responses> {
    static CACHE: OnceLock<Mutex<Responses>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_CACHE_BYTES, MAX_ENTRIES)))
}
pub fn is_url(path: &str) -> bool { path.starts_with("http://") || path.starts_with("https://") }
pub fn canonical_url(path: &str) -> Result<String, String> {
    let mut url=reqwest::Url::parse(path).map_err(|error| format!("invalid image URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") { return Err("remote images require http:// or https:// URLs".into()); }
    url.set_fragment(None);
    Ok(url.to_string())
}
pub fn load(url: &str, max_bytes: usize) -> Result<Arc<Download>, String> { request(url,max_bytes,false) }
/// Hosts can explicitly revalidate; remote resources are never polled by file watchers.
pub fn refresh(url: &str, max_bytes: usize) -> Result<Arc<Download>, String> { request(url,max_bytes,true) }
/// Expire an observed URL without network I/O. The next preparation performs a
/// conditional request using the existing validators.
pub fn invalidate(url: &str) -> Result<(), String> {
    let url=canonical_url(url)?;
    let slot=cache().lock().expect("HTTP asset cache poisoned").get(&url);
    if let Some(slot)=slot && let Some(previous)=slot.0.lock().expect("HTTP image response poisoned").as_mut() {
        Arc::make_mut(previous).expires=Instant::now();
    }
    Ok(())
}
fn request(url: &str, max_bytes: usize, refresh: bool) -> Result<Arc<Download>, String> {
    static CLIENT: OnceLock<Result<Client,String>> = OnceLock::new();
    let client=CLIENT.get_or_init(|| Client::builder().user_agent(concat!("shrimply/",env!("CARGO_PKG_VERSION"))).timeout(TIMEOUT).build().map_err(|error|error.to_string())).as_ref().map_err(Clone::clone)?;
    let url=canonical_url(url)?;
    let slot={
        let mut cache=cache().lock().expect("HTTP asset cache poisoned");
        if let Some(slot)=cache.get(&url) {slot} else {
            let slot=Arc::new(Slot(Mutex::new(None)));
            cache.insert(url.clone(),slot.clone(),std::mem::size_of::<Slot>()+url.len());
            slot
        }
    };
    // Only requests for this URL serialize. Other assets load independently.
    let mut retained=slot.0.lock().expect("HTTP image response poisoned");
    if refresh && let Some(previous)=retained.as_mut() {
        Arc::make_mut(previous).expires=Instant::now();
    }
    if let Some(previous)=retained.as_ref() && !refresh && Instant::now()<previous.expires {
        if previous.bytes.len()>max_bytes { return Err("remote image exceeds its byte budget".into()); }
        return Ok(previous.clone());
    }
    let mut request=client.get(&url).header(header::ACCEPT,"image/*");
    if let Some(previous)=retained.as_ref() {
        if let Some(etag)=&previous.etag {request=request.header(header::IF_NONE_MATCH,etag);}
        else if let Some(modified)=&previous.last_modified {request=request.header(header::IF_MODIFIED_SINCE,modified);}
    }
    let response=request.send().map_err(|error|format!("could not download image '{url}': {error}"))?;
    let status=response.status();
    let headers=response.headers();
    let value=|name| headers.get(name).and_then(|value|value.to_str().ok()).map(str::to_owned);
    let policy=value(header::CACHE_CONTROL).unwrap_or_default();
    let no_store=policy.split(',').any(|part|part.trim().eq_ignore_ascii_case("no-store"));
    let no_cache=policy.split(',').any(|part|part.trim().eq_ignore_ascii_case("no-cache"));
    let max_age=if no_store || no_cache { Duration::ZERO } else {
        policy.split(',').find_map(|part|part.trim().strip_prefix("max-age=").and_then(|value|value.trim_matches('"').parse::<u64>().ok())).map_or_else(||if status==reqwest::StatusCode::NOT_MODIFIED {retained.as_ref().map_or(DEFAULT_MAX_AGE,|previous|previous.max_age)} else {DEFAULT_MAX_AGE},|seconds|Duration::from_secs(seconds.min(MAX_MAX_AGE)))
    };
    let expires=Instant::now()+max_age;
    let content_type=value(header::CONTENT_TYPE);
    let etag=value(header::ETAG);
    let last_modified=value(header::LAST_MODIFIED);
    let downloaded=if status==reqwest::StatusCode::NOT_MODIFIED {
        let previous=retained.as_ref().ok_or("server returned HTTP304 without a cached image")?;
        if previous.bytes.len()>max_bytes { return Err("remote image exceeds its byte budget".into()); }
        Arc::new(Download {bytes:previous.bytes.clone(),fingerprint:previous.fingerprint.clone(),content_type:content_type.or_else(||previous.content_type.clone()),etag:etag.or_else(||previous.etag.clone()),last_modified:last_modified.or_else(||previous.last_modified.clone()),expires,max_age})
    } else {
        if !status.is_success() { return Err(format!("could not download image '{url}': HTTP {status}")); }
        if response.content_length().is_some_and(|length|length>max_bytes as u64) {return Err("remote image exceeds its byte budget".into());}
        let mut bytes=Vec::new();
        response.take(max_bytes.saturating_add(1) as u64).read_to_end(&mut bytes).map_err(|error|format!("could not read remote image '{url}': {error}"))?;
        if bytes.len()>max_bytes {return Err("remote image exceeds its byte budget".into());}
        Arc::new(Download {fingerprint:shrimply_resource_pipeline::fingerprint::digest(&bytes),bytes:Arc::new(bytes),content_type,etag,last_modified,expires,max_age})
    };
    if no_store { *retained=None; }
    else {
        *retained=Some(downloaded.clone());
        let size=downloaded.bytes.len()+2*url.len()+downloaded.content_type.as_ref().map_or(0,String::len)+downloaded.etag.as_ref().map_or(0,String::len)+downloaded.last_modified.as_ref().map_or(0,String::len)+std::mem::size_of::<Download>()+std::mem::size_of::<Slot>();
        cache().lock().expect("HTTP asset cache poisoned").insert(url,slot.clone(),size);
    }
    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, sync::atomic::{AtomicUsize,Ordering}};
    #[test]
    fn remote_images_share_downloads_and_revalidate_etags_without_polling() {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let url=format!("http://{}/image.svg",listener.local_addr().unwrap());
        let version=Arc::new(AtomicUsize::new(1));
        let current=version.clone();
        let requests=Arc::new(AtomicUsize::new(0));
        let seen=requests.clone();
        let server=std::thread::spawn(move|| {
            for stream in listener.incoming().take(3) {
                let mut stream=stream.unwrap();
                let mut request=String::new();
                {
                    use std::io::BufRead;
                    let mut input=std::io::BufReader::new(&mut stream);
                    loop {
                        let mut line=String::new();
                        let count=input.read_line(&mut line).unwrap();
                        request.push_str(&line);
                        if count==0 || line=="\r\n" {break;}
                    }
                }
                seen.fetch_add(1,Ordering::SeqCst);
                let version=current.load(Ordering::SeqCst);
                let etag=format!("\"v{version}\"");
                if request.to_ascii_lowercase().contains(&format!("if-none-match: {etag}")) {
                    write!(stream,"HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let body=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{version}\" height=\"1\"/>");
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nETag: {etag}\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            }
        });
        let first=load(&url,4096).unwrap();
        let again=load(&url,4096).unwrap();
        assert!(Arc::ptr_eq(&first,&again));
        assert_eq!(requests.load(Ordering::SeqCst),1);
        assert!(load(&url,1).is_err(),"cached bodies still obey each caller's limit");
        let unchanged=refresh(&url,4096).unwrap();
        assert!(Arc::ptr_eq(&first.bytes,&unchanged.bytes));
        version.store(2,Ordering::SeqCst);
        let changed=refresh(&url,4096).unwrap();
        assert_ne!(changed.fingerprint,first.fingerprint);
        assert_eq!(requests.load(Ordering::SeqCst),3);
        server.join().unwrap();
        let obsolete=Arc::downgrade(&first.bytes);
        drop(first);drop(again);drop(unchanged);
        assert!(obsolete.upgrade().is_none(), "revalidation keeps only the current body, not an ETag history");
    }
    #[test]
    fn remote_image_protocols_are_explicit() {
        assert!(canonical_url("ftp://example.com/image.png").unwrap_err().contains("http"));
        assert_eq!(canonical_url("https://example.com/image.svg#view").unwrap(),"https://example.com/image.svg");
    }
}
