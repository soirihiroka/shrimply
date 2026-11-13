//! One immutable syntax result per content/grammar revision, shared across paths.
use super::{MAX_SYNTAX_CACHE_BYTES, MAX_SYNTAX_CACHE_ENTRIES};
use sanim_core::{
    cache::Cache,
    code::{Document, Language},
};
use std::sync::{Arc, Mutex, OnceLock};
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Identity {
    Native {
        language: tree_sitter::Language,
        queries: [u8; 32],
    },
    Provider(usize),
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    language: Language,
    parser: Identity,
    source: [u8; 32],
}
pub(super) struct Parsed {
    pub document: Arc<Document>,
    pub bytes: usize,
}
type Flight = OnceLock<Result<Arc<Parsed>, String>>;
type Contents = Mutex<Cache<Key, Flight>>;
type Paths = Mutex<Cache<(Language, Arc<str>), Parsed>>;
fn contents() -> &'static Contents {
    static CACHE: OnceLock<Contents> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_SYNTAX_CACHE_BYTES, MAX_SYNTAX_CACHE_ENTRIES)))
}
fn paths() -> &'static Paths {
    static CACHE: OnceLock<Paths> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_SYNTAX_CACHE_BYTES, MAX_SYNTAX_CACHE_ENTRIES)))
}
pub(super) fn previous(document: &Document) -> Option<Arc<Parsed>> {
    paths()
        .lock()
        .expect("code syntax path index poisoned")
        .get(&(document.language, document.path.clone()))
}
pub(super) fn bytes(document: &Document) -> usize {
    document.text.len()
        + document.path.len()
        + document.syntax.retained_bytes
        + std::mem::size_of_val(document.scopes.as_ref())
        + document
            .scopes
            .iter()
            .map(|scope| scope.id.len() + scope.label.len())
            .sum::<usize>()
        + std::mem::size_of_val(document.highlights.as_ref())
        + document
            .highlights
            .iter()
            .map(|span| span.kind.len())
            .sum::<usize>()
        + std::mem::size_of_val(document.diagnostics.as_ref())
        + document
            .diagnostics
            .iter()
            .map(|issue| issue.message.len())
            .sum::<usize>()
}
pub(super) fn memo(
    mut document: Document,
    parser: Identity,
    compute: impl FnOnce(Document) -> Result<Arc<Parsed>, String>,
) -> Result<Document, String> {
    let key = Key {
        language: document.language,
        parser,
        source: sanim_core::digest_bytes(document.text.as_bytes()),
    };
    let flight = {
        let mut cache = contents()
            .lock()
            .expect("code syntax content cache poisoned");
        if let Some(flight) = cache.get(&key) {
            flight
        } else {
            let flight = Arc::new(OnceLock::new());
            cache.insert(
                key.clone(),
                flight.clone(),
                document.text.len() + std::mem::size_of::<Flight>(),
            );
            flight
        }
    };
    // Separate keys can compute concurrently; the same revision is parsed once.
    let parsed = flight.get_or_init(|| compute(document.clone())).clone()?;
    contents()
        .lock()
        .expect("code syntax content cache poisoned")
        .insert(key, flight, parsed.bytes);
    paths()
        .lock()
        .expect("code syntax path index poisoned")
        .insert(
            (document.language, document.path.clone()),
            parsed.clone(),
            parsed.bytes,
        );
    document.scopes = parsed.document.scopes.clone();
    document.highlights = parsed.document.highlights.clone();
    document.diagnostics = parsed.document.diagnostics.clone();
    document.syntax = parsed.document.syntax.clone();
    document.syntax_parsed = parsed.document.syntax_parsed;
    Ok(document)
}
