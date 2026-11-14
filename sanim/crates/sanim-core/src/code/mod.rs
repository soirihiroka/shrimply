//! Immutable source documents, syntax evidence, and validated unified patches.
mod patch;
pub mod values;
pub mod view;
pub use patch::{FilePatch, Hunk, Patch, PatchLine};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

/// An immutable, language-native tree. It is derived from source bytes and is
/// intentionally absent from serialization and semantic value equality.
#[derive(Clone, Default)]
pub struct SyntaxArtifact {
    tree: Option<Arc<dyn std::any::Any + Send + Sync>>,
    pub retained_bytes: usize,
}
impl SyntaxArtifact {
    pub fn new<T: std::any::Any + Send + Sync>(tree: T, retained_bytes: usize) -> Self {
        Self {
            tree: Some(Arc::new(tree)),
            retained_bytes,
        }
    }
    pub fn shared<T: std::any::Any + Send + Sync>(tree: Arc<T>, retained_bytes: usize) -> Self {
        Self {
            tree: Some(tree),
            retained_bytes,
        }
    }
    pub fn get<T: std::any::Any>(&self) -> Option<&T> {
        self.tree.as_ref()?.downcast_ref()
    }
    pub fn is_present(&self) -> bool {
        self.tree.is_some()
    }
    pub fn shares_tree(&self, other: &Self) -> bool {
        match (&self.tree, &other.tree) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}
impl std::fmt::Debug for SyntaxArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxArtifact")
            .field("retained_bytes", &self.retained_bytes)
            .finish_non_exhaustive()
    }
}
impl PartialEq for SyntaxArtifact {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for SyntaxArtifact {}

pub const MAX_CODE_BYTES: usize = 1024 * 1024;
pub const MAX_CODE_LINES: usize = 16_384;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Language {
    #[default]
    Auto,
    Plain,
    Sanim,
    Python,
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Bash,
    Toml,
    Yaml,
    C,
    Cpp,
    Json,
    Sql,
}
impl Language {
    pub fn parse(name: &str) -> Result<Self, String> {
        Self::ALL.iter().find(|(registered, _)| registered.eq_ignore_ascii_case(name.trim()))
            .map(|(_, language)| *language)
            .or_else(|| match name.trim().to_ascii_lowercase().as_str() {
                "py"=>Some(Self::Python),"rs"=>Some(Self::Rust),"jsx"|"js" => Some(Self::JavaScript), "ts" => Some(Self::TypeScript),
                "tsx" => Some(Self::Tsx), "c++" => Some(Self::Cpp), "shell"|"zsh"|"sh" => Some(Self::Bash), "yml"=>Some(Self::Yaml),
                "text" => Some(Self::Plain), _ => None,
            })
            .ok_or_else(|| format!("unknown code language '{name}'; use auto, python, sanim, rust, javascript, typescript, tsx, c, cpp, json, yaml, toml, bash, sql, or plain"))
    }
    pub const ALL: &'static [(&'static str, Self)] = &[
        ("Auto", Self::Auto),
        ("Plain", Self::Plain),
        ("Sanim", Self::Sanim),
        ("Python", Self::Python),
        ("Rust", Self::Rust),
        ("JavaScript", Self::JavaScript),
        ("TypeScript", Self::TypeScript),
        ("Tsx", Self::Tsx),
        ("Bash", Self::Bash),
        ("Toml", Self::Toml),
        ("Yaml", Self::Yaml),
        ("C", Self::C),
        ("Cpp", Self::Cpp),
        ("Json", Self::Json),
        ("Sql", Self::Sql),
    ];
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, language)| *language == self)
            .expect("registered code language")
            .0
    }
    pub fn detect(path: &str) -> Self {
        let path = path
            .strip_suffix(".patch")
            .or_else(|| path.strip_suffix(".diff"))
            .unwrap_or(path);
        match Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "sanim" => Self::Sanim,
            "py" | "pyi" => Self::Python,
            "rs" => Self::Rust,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "sh" | "bash" | "zsh" => Self::Bash,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" => Self::Cpp,
            "json" => Self::Json,
            "sql" => Self::Sql,
            _ => Self::Plain,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Stable structural path, independent of byte offsets, for example `function:main/if:1`.
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub start_line: usize,
    pub end_line: usize,
    /// Exact UTF-8 syntax bounds, independent of whole-line presentation ranges.
    pub start_byte: usize,
    pub end_byte: usize,
    pub depth: usize,
}
impl Scope {
    /// Whole lines hidden by folding; a boundary shared with outside syntax stays visible.
    pub fn hidden_end_line(&self, source: &str) -> Result<usize, String> {
        let suffix = source.get(self.end_byte..).ok_or_else(|| {
            format!(
                "syntax node '{}' has an invalid end byte {}",
                self.label, self.end_byte
            )
        })?;
        if self.end_byte > 0 && source.as_bytes()[self.end_byte - 1] == b'\n' {
            return Ok(self.end_line);
        }
        let trailing = suffix
            .split('\n')
            .next()
            .expect("source suffix has one line");
        Ok(if trailing.trim().is_empty() {
            self.end_line
        } else {
            self.end_line.saturating_sub(1).max(self.start_line)
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub start: usize,
    pub end: usize,
    pub kind: Arc<str>,
}
/// Inner captures override outer spans; later equal spans are explicit overrides.
/// Shared by native code presentation and language-server semantic tokens.
pub fn normalize_highlights(highlights: &[Highlight], text: &str) -> Vec<Highlight> {
    use std::collections::BTreeSet;
    let mut events = Vec::with_capacity(highlights.len() * 2);
    for (index, span) in highlights.iter().enumerate() {
        if span.start < span.end
            && span.end <= text.len()
            && text.is_char_boundary(span.start)
            && text.is_char_boundary(span.end)
        {
            events.push((span.start, true, index));
            events.push((span.end, false, index));
        }
    }
    events.sort_unstable();
    let mut active: BTreeSet<(usize, std::cmp::Reverse<usize>, usize)> = BTreeSet::new();
    let mut output = Vec::<Highlight>::new();
    let mut previous = 0;
    for (offset, enter, index) in events {
        if offset > previous
            && let Some(&(_, _, current)) = active.first()
        {
            let span = &highlights[current];
            if let Some(last) = output
                .last_mut()
                .filter(|last| last.end == previous && last.kind == span.kind)
            {
                last.end = offset;
            } else {
                output.push(Highlight {
                    start: previous,
                    end: offset,
                    kind: span.kind.clone(),
                });
            }
        }
        let span = &highlights[index];
        let key = (span.end - span.start, std::cmp::Reverse(index), index);
        if enter {
            active.insert(key);
        } else {
            active.remove(&key);
        }
        previous = offset;
    }
    output
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxIssue {
    pub line: usize,
    pub column: usize,
    pub message: Arc<str>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    Source,
    Patch(Patch),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub language: Language,
    pub path: Arc<str>,
    pub text: Arc<str>,
    pub content: Content,
    /// Real parser-derived scopes. Empty means the document has no parsed scopes.
    pub scopes: Arc<[Scope]>,
    pub syntax_parsed: bool,
    pub highlights: Arc<[Highlight]>,
    pub diagnostics: Arc<[SyntaxIssue]>,
    #[serde(skip)]
    pub syntax: SyntaxArtifact,
}
impl Document {
    /// Source and syntax evidence that affect native layout. Paths, diagnostics,
    /// and opaque parser trees do not change the rendered document.
    pub fn visual_recipe(&self) -> impl Serialize + '_ {
        (
            self.language,
            &self.text,
            self.syntax_parsed,
            &self.highlights,
            &self.scopes,
        )
    }
    pub fn read(path: &str, text: &str, language: Language) -> Result<Self, String> {
        Self::read_shared(path, text.into(), language)
    }
    pub fn read_shared(path: &str, text: Arc<str>, language: Language) -> Result<Self, String> {
        Self::read_text(path, text, language, true)
    }
    pub fn source_shared(path: &str, text: Arc<str>, language: Language) -> Result<Self, String> {
        Self::read_text(path, text, language, false)
    }
    fn read_text(
        path: &str,
        text: Arc<str>,
        language: Language,
        patches: bool,
    ) -> Result<Self, String> {
        if text.len() > MAX_CODE_BYTES || text.lines().count() > MAX_CODE_LINES {
            return Err(format!(
                "Code source exceeds {MAX_CODE_BYTES} bytes or {MAX_CODE_LINES} lines"
            ));
        }
        let extension = Path::new(path).extension().and_then(|value| value.to_str());
        let content = if patches
            && (matches!(extension, Some("patch" | "diff"))
                || text.starts_with("diff --git ")
                || text.starts_with("--- ") && text.contains("\n+++ "))
        {
            Content::Patch(Patch::parse(&text)?)
        } else {
            Content::Source
        };
        let language = if language == Language::Auto {
            match &content {
                Content::Patch(patch) if patch.files.len() == 1 => {
                    Language::detect(&patch.files[0].new_path)
                }
                _ => Language::detect(path),
            }
        } else {
            language
        };
        Ok(Self {
            language,
            path: path.into(),
            text,
            content,
            scopes: Arc::from([]),
            syntax_parsed: false,
            highlights: Arc::from([]),
            diagnostics: Arc::from([]),
            syntax: SyntaxArtifact::default(),
        })
    }
    pub fn source(&self) -> Result<&str, String> {
        if matches!(self.content, Content::Source) {
            Ok(&self.text)
        } else {
            Err("a patch is a change document, not a complete source file; create CodeView(src=...base source...) and call view.patch(src=...patch file...)".into())
        }
    }
    pub fn apply(&self, patch: &Self, file: Option<&str>) -> Result<Self, String> {
        let Content::Patch(changes) = &patch.content else {
            return Err("CodeView.patch requires a unified .patch/.diff change file".into());
        };
        let selected = changes
            .select(file.or_else(|| (!self.path.is_empty()).then_some(self.path.as_ref())))?;
        let text = selected.apply(self.source()?)?;
        Self::read(&selected.new_path, &text, self.language)
    }
}
