//! Overlay-aware analysis; bounded temporal sizing reads only analyzed source modules.
use crate::text::{self, Document};
use sanim_lang::{AnalyzedModule, Engine, FileSources, LintSeverity, Sources};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct OpenDocument {
    pub path: PathBuf,
    pub document: Document,
    pub syntax: std::cell::RefCell<Option<Syntax>>,
}
pub(crate) struct Syntax {
    version: i64,
    pub markup: Vec<sanim_lang::parser::MarkupOpening>,
    pub highlights: Vec<sanim_lang::core::code::Highlight>,
    pub type_references: Vec<sanim_lang::parser::TypeReference>,
    pub declarations: Vec<sanim_lang::parser::Declaration>,
}
impl OpenDocument {
    pub fn syntax(&self) -> std::cell::Ref<'_, Syntax> {
        if self
            .syntax
            .borrow()
            .as_ref()
            .is_none_or(|syntax| syntax.version != self.document.version)
        {
            self.parse_syntax();
        }
        std::cell::Ref::map(self.syntax.borrow(), |syntax| {
            syntax.as_ref().expect("computed parser metadata")
        })
    }
    fn parse_syntax(&self) -> sanim_lang::parser::Recovery {
        let (recovery, highlights) = sanim_lang::parser::parse_highlighted(
            "editor",
            &self.document.text,
            sanim_lang::Limits::default().max_parse_steps,
        );
        *self.syntax.borrow_mut() = Some(Syntax {
            version: self.document.version,
            markup: recovery.markup.clone(),
            type_references: recovery.type_references.clone(),
            declarations: recovery.declarations.clone(),
            highlights: sanim_lang::core::code::normalize_highlights(
                &highlights,
                &self.document.text,
            ),
        });
        recovery
    }
}

pub(crate) struct Analysis {
    pub root: PathBuf,
    pub entry: String,
    pub modules: BTreeMap<String, AnalyzedModule>,
    pub dependencies: BTreeSet<PathBuf>,
    pub pending_calls: Vec<sanim_lang::parser::PendingCall>,
    diagnostics: BTreeMap<String, Vec<Value>>,
    failed: bool,
}

#[derive(Default)]
pub(crate) struct Workspace {
    pub roots: Vec<PathBuf>,
    pub snippet_support: bool,
    pub documents: BTreeMap<String, OpenDocument>,
    pub analyses: BTreeMap<String, Analysis>,
    engines: BTreeMap<PathBuf, Engine>,
    layouts: sanim_lang::core::cache::Cache<String, StaticLayout>,
    published: BTreeSet<String>,
}

const STATIC_LAYOUT_BYTES: usize = 8 * 1024 * 1024;
const STATIC_LAYOUT_ENTRIES: usize = 64;
const STATIC_LAYOUT_OPERATIONS: u64 = 10_000;
pub(crate) struct StaticLayout {
    pub duration: Option<sanim_lang::core::plan::Extent>,
    pub receipts: Arc<[sanim_lang::core::plan::Receipt]>,
    pub constraint: Option<&'static str>,
}
struct LayoutSources<'a> {
    source: &'a dyn Sources,
    modules: &'a BTreeMap<String, AnalyzedModule>,
}
impl Sources for LayoutSources<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if !self.modules.contains_key(path) {
            return Err(format!("layout depends on external resource '{path}'"));
        }
        self.source.read(path)
    }
}

struct Overlay<'a> {
    files: FileSources,
    root: &'a Path,
    documents: &'a BTreeMap<String, OpenDocument>,
}
impl Sources for Overlay<'_> {
    fn read(&self, source: &str) -> Result<Vec<u8>, String> {
        let path = canonical(&self.root.join(source));
        self.documents
            .values()
            .find(|document| document.path == path)
            .map(|document| Ok(document.document.text.as_bytes().to_vec()))
            .unwrap_or_else(|| self.files.read(source))
    }
}
struct Prefix<'a> {
    sources: &'a dyn Sources,
    entry: &'a str,
    text: &'a str,
}
impl Sources for Prefix<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if path == self.entry {
            Ok(self.text.as_bytes().to_vec())
        } else {
            self.sources.read(path)
        }
    }
}
impl Workspace {
    pub fn root(&self, path: &Path) -> PathBuf {
        self.roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_owned())
    }
    pub fn open(&mut self, uri: &str, text: String, version: i64) -> Result<(), String> {
        let path = text::path(uri)?;
        let path = path
            .canonicalize()
            .or_else(|_| {
                let parent = path
                    .parent()
                    .ok_or_else(|| std::io::Error::other("document requires a parent directory"))?
                    .canonicalize()?;
                Ok::<_, std::io::Error>(
                    parent
                        .join(path.file_name().ok_or_else(|| {
                            std::io::Error::other("document requires a filename")
                        })?),
                )
            })
            .map_err(|error| error.to_string())?;
        self.documents.insert(
            uri.into(),
            OpenDocument {
                path,
                document: Document::new(text, version),
                syntax: Default::default(),
            },
        );
        Ok(())
    }
    pub fn close(&mut self, uri: &str) {
        self.documents.remove(uri);
        self.analyses.remove(uri);
        self.layouts.remove(&uri.to_owned());
        let used = self
            .documents
            .values()
            .map(|document| self.root(&document.path))
            .collect::<BTreeSet<_>>();
        self.engines.retain(|root, _| used.contains(root));
    }
    /// Refresh the changed entry and known import dependents. Failed analyses are
    /// retried too, allowing a newly created/missing import to recover immediately.
    pub fn refresh(&mut self, changed: Option<&str>) -> Vec<Value> {
        let changed_paths = changed
            .and_then(|uri| text::path(uri).ok())
            .map(|path| BTreeSet::from([canonical(&path), path]))
            .unwrap_or_default();
        let affected = self
            .documents
            .iter()
            .filter(|(uri, _)| {
                changed.is_none()
                    || changed == Some(uri.as_str())
                    || self.analyses.get(*uri).is_none_or(|analysis| {
                        analysis.failed
                            || changed_paths
                                .iter()
                                .any(|path| analysis.dependencies.contains(path))
                    })
            })
            .map(|(uri, _)| uri.clone())
            .collect::<Vec<_>>();
        for uri in affected {
            self.layouts.remove(&uri);
            if let Err(error) = self.analyze(&uri) {
                let document = &self.documents[&uri].document;
                let analysis = Analysis {
                    root: self.root(&self.documents[&uri].path),
                    entry: String::new(),
                    modules: BTreeMap::new(),
                    dependencies: BTreeSet::new(),
                    pending_calls: Vec::new(),
                    diagnostics: BTreeMap::from([(
                        uri.clone(),
                        vec![
                            json!({"range":document.range(Default::default()),"severity":1,"source":"sanim","code":"LSP001","message":error}),
                        ],
                    )]),
                    failed: true,
                };
                self.analyses.insert(uri, analysis);
            }
        }
        let mut combined: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for analysis in self.analyses.values() {
            for (uri, diagnostics) in &analysis.diagnostics {
                let target = combined.entry(uri.clone()).or_default();
                for diagnostic in diagnostics {
                    if !target.contains(diagnostic) {
                        target.push(diagnostic.clone());
                    }
                }
            }
        }
        let targets = self
            .published
            .iter()
            .cloned()
            .chain(combined.keys().cloned())
            .chain(self.documents.keys().cloned())
            .collect::<BTreeSet<_>>();
        self.published = combined.keys().cloned().collect();
        targets
            .into_iter()
            .map(|uri| {
                let mut params =
                    json!({"uri":uri,"diagnostics":combined.remove(&uri).unwrap_or_default()});
                if let Some(document) = self.documents.get(&uri) {
                    params["version"] = document.document.version.into();
                }
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":params})
            })
            .collect()
    }
    fn analyze(&mut self, uri: &str) -> Result<(), String> {
        let document = self.documents.get(uri).ok_or("document is not open")?;
        let root = self.root(&document.path);
        let entry = document
            .path
            .strip_prefix(&root)
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or("document path must be UTF-8")?
            .replace('\\', "/");
        let files = FileSources::new(&root)?;
        let sources = Overlay {
            files,
            root: &root,
            documents: &self.documents,
        };
        let result = self
            .engines
            .entry(root.clone())
            .or_default()
            .lint(&entry, &sources);
        let mut analysis = Analysis {
            root: root.clone(),
            entry: entry.clone(),
            modules: BTreeMap::new(),
            dependencies: BTreeSet::new(),
            pending_calls: Vec::new(),
            diagnostics: BTreeMap::new(),
            failed: false,
        };
        let mut source_documents = BTreeMap::new();
        match result {
            Ok(report) => {
                analysis.modules = report.modules;
                analysis.dependencies = report
                    .dependencies
                    .keys()
                    .flat_map(|source| {
                        let path = root.join(source);
                        [canonical(&path), path]
                    })
                    .collect();
                for diagnostic in report.diagnostics {
                    let diagnostic_uri = self.uri_for_path(&root.join(&diagnostic.source))?;
                    let range = self.diagnostic_range(
                        &diagnostic_uri,
                        diagnostic.span,
                        &mut source_documents,
                    );
                    analysis
                        .diagnostics
                        .entry(diagnostic_uri)
                        .or_default()
                        .push(json!({
                            "range":range,
                            "severity":if diagnostic.severity==LintSeverity::Error {1} else {2},
                            "source":"sanim","code":diagnostic.code,"message":diagnostic.message
                        }));
                }
            }
            Err(diagnostic) => {
                analysis.failed = true;
                let diagnostic_uri = self.uri_for_path(&root.join(&diagnostic.source))?;
                let range =
                    self.diagnostic_range(&diagnostic_uri, diagnostic.span, &mut source_documents);
                analysis.diagnostics.entry(diagnostic_uri).or_default().push(json!({
                    "range":range,"severity":1,"source":"sanim","code":diagnostic.code,"message":diagnostic.message
                }));
                // Canonical recovery retains completed statements and actual
                // parser call contexts. A typed prefix supports completion and
                // signatures during ordinary unfinished edits, without inventing
                // tokens, stale ranges, or an independent parser/checker.
                let recovery = document.parse_syntax();
                let prefix_end = if recovery.diagnostic.is_some() {
                    recovery.prefix_end
                } else if diagnostic.source == entry {
                    // Type errors also leave preceding declarations usable. The
                    // complete root statement containing the error is excluded;
                    // its enclosing function/block must never acquire fake types.
                    recovery
                        .module
                        .statements
                        .iter()
                        .take_while(|statement| statement.span.end <= diagnostic.span.start)
                        .last()
                        .map_or(0, |statement| statement.span.end)
                } else {
                    0
                };
                let recovered_types = if prefix_end > 0 {
                    let prefix = Prefix {
                        sources: &sources,
                        entry: &entry,
                        text: &document.document.text[..prefix_end],
                    };
                    self.engines
                        .entry(root.clone())
                        .or_default()
                        .lint(&entry, &prefix)
                        .ok()
                } else {
                    None
                };
                analysis.pending_calls = recovery.pending_calls;
                if let Some(report) = recovered_types {
                    analysis.modules = report.modules;
                    if let Some(module) = analysis.modules.get_mut(&entry) {
                        module.ast = Arc::new(recovery.module);
                    }
                    analysis.dependencies = report
                        .dependencies
                        .keys()
                        .flat_map(|source| {
                            let path = root.join(source);
                            [canonical(&path), path]
                        })
                        .collect();
                } else {
                    analysis.modules.insert(
                        entry.clone(),
                        AnalyzedModule {
                            ast: Arc::new(recovery.module),
                            checked: Arc::default(),
                        },
                    );
                }
            }
        }
        // Superseded utilities remain source metadata even though the parser
        // correctly removes them from the executable argument list.
        for opening in &document.syntax().markup {
            let explicit = opening
                .attributes
                .iter()
                .filter(|attribute| !attribute.utility)
                .map(|attribute| attribute.name.as_str())
                .collect::<BTreeSet<_>>();
            struct Override<'a> {
                previous: &'a sanim_lang::parser::MarkupAttribute,
                keys: Vec<String>,
                redundant: bool,
            }
            let mut properties =
                BTreeMap::<String, (&sanim_lang::parser::MarkupAttribute, String)>::new();
            for attribute in opening
                .attributes
                .iter()
                .filter(|attribute| attribute.utility)
            {
                let Ok(utility) = sanim_lang::utilities::describe(&opening.name, &attribute.name)
                else {
                    continue;
                };
                let mut replaced = BTreeMap::<(usize, usize), Override<'_>>::new();
                for (key, value) in utility.properties {
                    if explicit.contains(key.as_str()) {
                        continue;
                    }
                    if let Some((previous, old_value)) =
                        properties.insert(key.clone(), (attribute, value.clone()))
                    {
                        let entry = replaced
                            .entry((previous.span.start, previous.span.end))
                            .or_insert(Override {
                                previous,
                                keys: vec![],
                                redundant: true,
                            });
                        entry.keys.push(key);
                        entry.redundant &= old_value == value;
                    }
                }
                for Override {
                    previous,
                    keys,
                    redundant,
                } in replaced.into_values()
                {
                    analysis.diagnostics.entry(uri.into()).or_default().push(json!({
                        "range":document.document.range(previous.span),"severity":2,"source":"sanim","code":"U001",
                        "message":format!("utility '{}' is {} by later '{}' for {}; remove the unnecessary setting", previous.name, if redundant {"repeated"} else {"overridden"}, attribute.name, keys.join(", ")),
                        "relatedInformation":[{"location":{"uri":uri,"range":document.document.range(attribute.span)},"message":"Later utility takes precedence here"}],
                    }));
                }
            }
        }
        self.analyses.insert(uri.into(), analysis);
        Ok(())
    }
    pub fn document_text(&self, uri: &str) -> Option<String> {
        if let Some(document) = self.documents.get(uri) {
            return Some(document.document.text.clone());
        }
        let path = canonical(&text::path(uri).ok()?);
        if let Some(document) = self
            .documents
            .values()
            .find(|document| document.path == path)
        {
            return Some(document.document.text.clone());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        let max = sanim_lang::Limits::default().max_source_bytes;
        std::fs::File::open(path)
            .ok()?
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > max {
            return None;
        }
        String::from_utf8(bytes).ok()
    }
    fn diagnostic_range(
        &self,
        uri: &str,
        span: sanim_lang::core::Span,
        closed: &mut BTreeMap<String, Document>,
    ) -> Value {
        if let Some(document) = self.documents.get(uri) {
            return document.document.range(span);
        }
        closed
            .entry(uri.into())
            .or_insert_with(|| Document::new(self.document_text(uri).unwrap_or_default(), 0))
            .range(span)
    }
    pub fn uri_for_path(&self, path: &Path) -> Result<String, String> {
        let path = canonical(path);
        self.documents
            .iter()
            .find(|(_, document)| document.path == path)
            .map(|(uri, _)| Ok(uri.clone()))
            .unwrap_or_else(|| text::uri(&path))
    }
    pub(crate) fn animation_layout(&mut self, uri: &str) -> Option<Arc<StaticLayout>> {
        self.layouts
            .configure(STATIC_LAYOUT_BYTES, STATIC_LAYOUT_ENTRIES);
        if let Some(layout) = self.layouts.get(&uri.to_owned()) {
            return Some(layout);
        }
        let analysis = self.analyses.get(uri)?;
        if analysis.failed {
            return None;
        }
        let sources = Overlay {
            files: FileSources::new(&analysis.root).ok()?,
            root: &analysis.root,
            documents: &self.documents,
        };
        let sources = LayoutSources {
            source: &sources,
            modules: &analysis.modules,
        };
        let engine = self.engines.get_mut(&analysis.root)?;
        let limits = engine.limits.clone();
        let preparation = engine.preparation;
        let available = engine.available_duration;
        engine.preparation = sanim_lang::Preparation::Demand;
        engine.available_duration = None;
        engine.limits.max_operations = engine.limits.max_operations.min(STATIC_LAYOUT_OPERATIONS);
        engine.limits.max_iterations = engine
            .limits
            .max_iterations
            .min(STATIC_LAYOUT_OPERATIONS as usize);
        engine.limits.max_elements = engine
            .limits
            .max_elements
            .min(STATIC_LAYOUT_OPERATIONS as usize);
        engine.limits.max_time_map_bytes =
            engine.limits.max_time_map_bytes.min(STATIC_LAYOUT_BYTES);
        engine.limits.max_cache_bytes = engine.limits.max_cache_bytes.min(STATIC_LAYOUT_BYTES);
        engine.limits.data.max_bytes = engine.limits.data.max_bytes.min(STATIC_LAYOUT_BYTES);
        engine.limits.data.max_values = engine
            .limits
            .data
            .max_values
            .min(STATIC_LAYOUT_OPERATIONS as usize);
        engine.limits.data.max_sql_steps = engine
            .limits
            .data
            .max_sql_steps
            .min(STATIC_LAYOUT_OPERATIONS);
        let result = engine.duration(&analysis.entry, &sources, &BTreeMap::new());
        engine.limits = limits;
        engine.preparation = preparation;
        engine.available_duration = available;
        let layout = match result {
            Ok(report) => StaticLayout {
                duration: Some(if report.indefinite {
                    sanim_lang::core::plan::Extent::Infinite
                } else {
                    sanim_lang::core::plan::Extent::Finite(report.duration)
                }),
                receipts: report.temporal_layout,
                constraint: None,
            },
            Err(error) => StaticLayout {
                duration: None,
                receipts: Arc::from([]),
                constraint: Some(if matches!(error.code.as_str(), "S138" | "S139") {
                    "Duration: `match_parent`; the editor clip supplies the parent constraint."
                } else if error.message.contains("budget") || error.message.contains("limit") {
                    "Duration: unresolved within the static evaluation budget."
                } else {
                    "Duration: unresolved without runtime inputs."
                }),
            },
        };
        let bytes = std::mem::size_of::<StaticLayout>()
            + uri.len()
            + layout
                .receipts
                .iter()
                .map(|receipt| {
                    std::mem::size_of_val(receipt)
                        + receipt.origin.as_ref().map_or(0, |origin| {
                            origin.location.source.len()
                                + origin.binding.as_ref().map_or(0, |binding| binding.len())
                        })
                })
                .sum::<usize>();
        let layout = Arc::new(layout);
        self.layouts.insert(uri.to_owned(), layout.clone(), bytes);
        Some(layout)
    }
    pub fn module(&self, uri: &str) -> Option<(&Analysis, &AnalyzedModule)> {
        let analysis = self.analyses.get(uri)?;
        Some((analysis, analysis.modules.get(&analysis.entry)?))
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| path.to_owned())
    })
}
