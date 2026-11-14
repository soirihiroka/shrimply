//! Headless grammar registry. Parsing, highlights and fold scopes share one tree.
mod cache;
pub mod view;
use cache::{Identity, Parsed};
use sanim_core::code::{Content, Document, Highlight, Language, Scope, SyntaxIssue};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
use tree_sitter::{InputEdit, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

const MAX_SYNTAX_NODES: usize = 200_000;
const MAX_SYNTAX_DEPTH: usize = 256;
const MAX_SYNTAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SYNTAX_CACHE_ENTRIES: usize = 32;
pub type ParseProvider = fn(Document) -> Result<Document, String>;
fn providers() -> &'static Mutex<BTreeMap<Language, ParseProvider>> {
    static PROVIDERS: OnceLock<Mutex<BTreeMap<Language, ParseProvider>>> = OnceLock::new();
    PROVIDERS.get_or_init(Default::default)
}
/// Install a native source parser without introducing a renderer → language dependency.
pub fn register_parser(language: Language, provider: ParseProvider) -> Result<(), String> {
    let mut providers = providers().lock().expect("code parser registry poisoned");
    if let Some(previous) = providers.get(&language)
        && !std::ptr::fn_addr_eq(*previous, provider)
    {
        return Err(format!(
            "{} already has a registered native parser",
            language.name()
        ));
    }
    providers.insert(language, provider);
    Ok(())
}

pub struct Grammar {
    pub language: tree_sitter::Language,
    pub highlights: Query,
    pub folds: Query,
    identity: [u8; 32],
}
impl Grammar {
    pub fn new(
        language: tree_sitter::Language,
        highlights: &str,
        folds: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            identity: sanim_core::digest_bytes(
                &serde_json::to_vec(&(highlights, folds)).map_err(|error| error.to_string())?,
            ),
            highlights: Query::new(&language, highlights)
                .map_err(|error| format!("highlight query: {error}"))?,
            folds: Query::new(&language, folds).map_err(|error| format!("fold query: {error}"))?,
            language,
        })
    }
}
pub fn grammar(language: Language) -> Result<Option<Arc<Grammar>>, String> {
    static GRAMMARS: OnceLock<Mutex<BTreeMap<Language, Arc<Grammar>>>> = OnceLock::new();
    let mut registry = GRAMMARS
        .get_or_init(Default::default)
        .lock()
        .expect("code grammar registry poisoned");
    if let Some(grammar) = registry.get(&language) {
        return Ok(Some(grammar.clone()));
    }
    let (grammar, highlights, folds): (tree_sitter::Language, String, &str) = match language {
        Language::Python => (
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY.into(),
            "[(function_definition) (class_definition) (if_statement) (for_statement) (while_statement) (try_statement) (with_statement) (match_statement)] @fold",
        ),
        Language::Rust => (
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
            "[(function_item) (impl_item) (trait_item) (mod_item) (struct_item) (enum_item) (if_expression) (for_expression) (while_expression) (loop_expression) (match_expression)] @fold",
        ),
        Language::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY.into(),
            "[(function_declaration) (function_expression) (arrow_function) (method_definition) (class_declaration) (if_statement) (for_statement) (for_in_statement) (while_statement) (switch_statement) (try_statement) (object) (array)] @fold",
        ),
        Language::TypeScript | Language::Tsx => (
            if language == Language::Tsx {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            },
            format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
            "[(function_declaration) (function_expression) (arrow_function) (method_definition) (class_declaration) (interface_declaration) (if_statement) (for_statement) (for_in_statement) (while_statement) (switch_statement) (try_statement) (object) (array)] @fold",
        ),
        Language::C => (
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY.into(),
            "[(function_definition) (struct_specifier) (enum_specifier) (if_statement) (for_statement) (while_statement) (switch_statement)] @fold",
        ),
        Language::Cpp => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
            "[(function_definition) (class_specifier) (struct_specifier) (enum_specifier) (namespace_definition) (if_statement) (for_statement) (while_statement) (switch_statement)] @fold",
        ),
        Language::Json => (
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY.into(),
            "[(object) (array)] @fold",
        ),
        Language::Bash => (
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY.into(),
            "[(function_definition) (if_statement) (for_statement) (while_statement) (case_statement) (subshell)] @fold",
        ),
        Language::Toml => (
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
            "[(table) (table_array_element) (array) (inline_table)] @fold",
        ),
        Language::Yaml => (
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
            "[(block_mapping_pair) (block_sequence_item) (flow_mapping) (flow_sequence)] @fold",
        ),
        Language::Sql => (
            tree_sitter_sequel::LANGUAGE.into(),
            tree_sitter_sequel::HIGHLIGHTS_QUERY.into(),
            "(statement) @fold",
        ),
        Language::Auto | Language::Plain | Language::Sanim => return Ok(None),
    };
    let config = Arc::new(
        Grammar::new(grammar, &highlights, folds)
            .map_err(|error| format!("{} grammar: {error}", language.name()))?,
    );
    registry.insert(language, config.clone());
    Ok(Some(config))
}

type Capture = (usize, usize, usize, Arc<str>);
pub fn parse(document: Document) -> Result<Document, String> {
    if document.syntax_parsed && document.syntax.is_present()
        || matches!(document.content, Content::Patch(_))
    {
        return Ok(document);
    }
    let provider = providers()
        .lock()
        .expect("code parser registry poisoned")
        .get(&document.language)
        .copied();
    if let Some(provider) = provider {
        return cache::memo(
            document,
            Identity::Provider(provider as usize),
            |document| {
                let document = provider(document)?;
                Ok(Arc::new(Parsed {
                    bytes: cache::bytes(&document),
                    document: Arc::new(document),
                }))
            },
        );
    }
    let Some(grammar) = grammar(document.language)? else {
        return Ok(document);
    };
    parse_with(document, &grammar, true)
}
/// Hosts can supply another native grammar or override its highlight/fold queries.
/// Code never loads a dynamic library or executes the displayed language.
pub fn parse_with(
    document: Document,
    grammar: &Grammar,
    incremental: bool,
) -> Result<Document, String> {
    if incremental {
        return cache::memo(
            document,
            Identity::Native {
                language: grammar.language.clone(),
                queries: grammar.identity,
            },
            |document| parse_tree(document, grammar, true),
        );
    }
    Ok(parse_tree(document, grammar, false)?
        .document
        .as_ref()
        .clone())
}
fn parse_tree(
    mut document: Document,
    grammar: &Grammar,
    incremental: bool,
) -> Result<Arc<Parsed>, String> {
    let previous = if incremental {
        cache::previous(&document)
    } else {
        None
    };
    let mut old = previous
        .as_ref()
        .and_then(|previous| previous.document.syntax.get::<Tree>())
        .filter(|tree| *tree.language() == grammar.language)
        .cloned();
    if let (Some(tree), Some(previous)) = (old.as_mut(), previous.as_ref()) {
        tree.edit(&edit(&previous.document.text, &document.text));
    }
    let mut parser = Parser::new();
    parser
        .set_language(&grammar.language)
        .map_err(|error| format!("{} grammar: {error}", document.language.name()))?;
    let mut calls = 0usize;
    let mut progress = |_: &tree_sitter::ParseState| {
        calls += 1;
        if calls > MAX_SYNTAX_NODES {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    let tree = parser
        .parse_with_options(
            &mut |offset, _| &document.text.as_bytes()[offset..],
            old.as_ref(),
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or("code syntax parsing exceeded its bounded work budget")?;
    let mut issues = Vec::new();
    let mut stack = vec![(tree.root_node(), 0)];
    let mut visited = 0;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > MAX_SYNTAX_NODES || depth > MAX_SYNTAX_DEPTH {
            return Err("code syntax tree exceeds node/depth limit".into());
        }
        if (node.is_error() || node.is_missing()) && issues.len() < 64 {
            issues.push(SyntaxIssue {
                line: node.start_position().row + 1,
                column: document.text[..node.start_byte()]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .count()
                    + 1,
                message: if node.is_missing() {
                    format!("missing {}", node.kind()).into()
                } else {
                    "unrecognized syntax".into()
                },
            });
        }
        if node.child_count() as usize > MAX_SYNTAX_NODES.saturating_sub(visited + stack.len()) {
            return Err("code syntax tree exceeds node limit".into());
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor).map(|child| (child, depth + 1)));
    }
    document.diagnostics = issues.into();
    document.highlights = captures(
        &grammar.highlights,
        tree.root_node(),
        document.text.as_bytes(),
    )?
    .into_iter()
    .map(|(_, start, end, kind)| Highlight { start, end, kind })
    .collect::<Vec<_>>()
    .into();
    let mut folds = captures(&grammar.folds, tree.root_node(), document.text.as_bytes())?;
    folds.sort_by_key(|(_, start, end, _)| (*start, std::cmp::Reverse(*end)));
    folds.dedup_by_key(|(_, start, end, _)| (*start, *end));
    let mut scopes = Vec::<Scope>::new();
    let mut parents = Vec::<(usize, usize, Arc<str>)>::new();
    let mut siblings = BTreeMap::<(Arc<str>, String), usize>::new();
    for (_, start, end, _) in folds {
        let Some(node) = tree.root_node().descendant_for_byte_range(start, end) else {
            continue;
        };
        let start_line = node.start_position().row + 1;
        let end_line = node.end_position().row + usize::from(node.end_position().column > 0);
        if node.has_error() {
            continue;
        }
        while parents
            .last()
            .is_some_and(|(_, parent_end, _)| *parent_end <= start || *parent_end < end)
        {
            parents.pop();
        }
        let parent = parents
            .last()
            .map(|(_, _, id)| id.clone())
            .unwrap_or_else(|| Arc::from(""));
        let name = declaration_name(node)
            .and_then(|node| node.utf8_text(document.text.as_bytes()).ok())
            .unwrap_or("");
        let label = if name.is_empty() {
            node.kind().to_string()
        } else {
            format!("{}:{name}", node.kind())
        };
        let ordinal = siblings.entry((parent.clone(), label.clone())).or_default();
        *ordinal += 1;
        let id: Arc<str> = format!(
            "{}{label}#{}",
            if parent.is_empty() {
                String::new()
            } else {
                format!("{parent}/")
            },
            ordinal
        )
        .into();
        scopes.push(Scope {
            id: id.clone(),
            label: label.into(),
            start_line,
            end_line,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            depth: parents.len() + 1,
        });
        parents.push((start, end, id));
    }
    document.scopes = scopes.into();
    document.syntax_parsed = true;
    document.syntax = sanim_core::code::SyntaxArtifact::new(
        tree,
        visited.saturating_mul(std::mem::size_of::<usize>() * 8),
    );
    Ok(Arc::new(Parsed {
        bytes: cache::bytes(&document),
        document: Arc::new(document),
    }))
}

// Grammars expose declaration names through different native fields. Follow
// declarators, not source-text heuristics, so qualified C++ names stay intact.
fn declaration_name(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(name);
    }
    if node.kind() == "impl_item" {
        return node.child_by_field_name("type");
    }
    if matches!(node.kind(), "arrow_function" | "function_expression")
        && let Some(parent) = node
            .parent()
            .filter(|parent| parent.kind() == "variable_declarator")
    {
        return parent.child_by_field_name("name");
    }
    let mut declarator = node.child_by_field_name("declarator")?;
    for _ in 0..MAX_SYNTAX_DEPTH {
        if matches!(
            declarator.kind(),
            "identifier"
                | "field_identifier"
                | "qualified_identifier"
                | "scoped_identifier"
                | "destructor_name"
                | "operator_name"
        ) {
            return Some(declarator);
        }
        if let Some(name) = declarator.child_by_field_name("name") {
            return Some(name);
        }
        declarator = declarator.child_by_field_name("declarator")?;
    }
    None
}

fn captures(query: &Query, root: Node<'_>, source: &[u8]) -> Result<Vec<Capture>, String> {
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(MAX_SYNTAX_NODES as u32);
    let mut output = Vec::new();
    {
        let mut matches = cursor.captures(query, root, source);
        while let Some((found, index)) = matches.next() {
            if output.len() >= MAX_SYNTAX_NODES {
                return Err("code highlight/fold capture limit exceeded".into());
            }
            let capture = found.captures()[*index];
            output.push((
                found.pattern_index,
                capture.node.start_byte(),
                capture.node.end_byte(),
                query.capture_names()[capture.index as usize].into(),
            ));
        }
    }
    if cursor.did_exceed_match_limit() {
        return Err("code syntax query exceeded match limit".into());
    }
    Ok(output)
}
fn point(text: &str) -> Point {
    let row = text.bytes().filter(|byte| *byte == b'\n').count();
    Point {
        row,
        column: text.rsplit('\n').next().unwrap_or("").len(),
    }
}
fn edit(old: &str, new: &str) -> InputEdit {
    let mut start = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let mut suffix = old[start..]
        .bytes()
        .rev()
        .zip(new[start..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(&old[..start]),
        old_end_position: point(&old[..old_end]),
        new_end_position: point(&new[..new_end]),
    }
}
