//! Bound source intervals: one selector and provenance contract for every code operation.
use super::*;
use sanim_core::code::{
    Document, Scope,
    view::{RangeOrigin, SourceIndex, source_revision},
};
use std::{collections::BTreeMap, ops::Range as Bytes};

pub(super) const QUERY_KINDS: &[&str] = &["name", "level", "line", "character", "regex"];
const MAX_EXAMPLES: usize = 8;

pub(super) fn parameters(plural: bool) -> Vec<Parameter> {
    let mut output = vec![
        Parameter {
            name: "name",
            ty: Type::String,
            required: false,
        },
        Parameter {
            name: "level",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "line",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "end_line",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "column",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "character",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "length",
            ty: Type::Numeric(Unit::Number),
            required: false,
        },
        Parameter {
            name: "regex",
            ty: Type::String,
            required: false,
        },
        Parameter {
            name: "within",
            ty: range_type(),
            required: false,
        },
    ];
    if !plural {
        output.push(Parameter {
            name: "occurrence",
            ty: Type::Numeric(Unit::Number),
            required: false,
        });
    }
    output
}

fn contains(outer: &Bytes<usize>, inner: &Bytes<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}
fn argument(args: &imbl::OrdMap<String, Value>, name: &str) -> Result<Option<usize>, String> {
    args.get(name).map(|value| integer(value, name)).transpose()
}
fn syntax_ready(document: &Document) -> Result<(), String> {
    if !document.syntax_parsed || !document.syntax.is_present() {
        return Err(format!(
            "{} has no native syntax tree; choose a registered CodeView language before querying or folding syntax ranges",
            document.language.name()
        ));
    }
    if let Some(issue) = document.diagnostics.first() {
        return Err(format!(
            "fix displayed syntax in '{}':{}:{} before querying or folding syntax ranges ({})",
            document.path, issue.line, issue.column, issue.message
        ));
    }
    Ok(())
}
fn qualified(scope: &Scope, scopes: &BTreeMap<&str, &Scope>) -> String {
    scope
        .id
        .match_indices('/')
        .map(|(end, _)| &scope.id[..end])
        .chain(std::iter::once(scope.id.as_ref()))
        .filter_map(|id| scopes.get(id))
        .filter_map(|scope| scope.label.split_once(':').map(|(_, name)| name))
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn query(
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
    plural: bool,
) -> Result<Value, String> {
    let view = args.get("view").ok_or("range query requires a CodeView")?;
    let Value::Element(element) = view else {
        return Err("range query requires a CodeView".into());
    };
    let document = syntax::parse(current_document(view, &context)?.as_ref().clone())?;
    context.charge_work(document.text.len().saturating_add(document.scopes.len()))?;
    let mut index = SourceIndex::new(document.source()?);
    let within = args.get("within").map(range).transpose()?;
    let bounds = if let Some(within) = &within {
        within.validate_for(&element.id, &document)?;
        index.resolve(within)?
    } else {
        0..index.text().len()
    };
    let revision = within
        .as_ref()
        .and_then(|range| range.origin.as_ref())
        .map(|origin| origin.revision.clone())
        .unwrap_or_else(|| source_revision(&document));
    let origin = Arc::new(RangeOrigin {
        owner: element.id.as_str().into(),
        revision,
        scope_id: None,
    });
    let name = args.get("name").map(Value::string).transpose()?;
    let level = argument(args, "level")?;
    let line = argument(args, "line")?;
    let column = argument(args, "column")?;
    let end_line = argument(args, "end_line")?;
    let character = argument(args, "character")?;
    let length = argument(args, "length")?;
    let pattern = args.get("regex").map(Value::string).transpose()?;
    let occurrence = argument(args, "occurrence")?;
    let ast = name.is_some()
        || level.is_some()
        || (plural && QUERY_KINDS.iter().all(|key| !args.contains_key(*key)));
    if (ast
        && (character.is_some()
            || column.is_some()
            || end_line.is_some()
            || length.is_some()
            || pattern.is_some()))
        || (pattern.is_some()
            && (line.is_some()
                || character.is_some()
                || column.is_some()
                || end_line.is_some()
                || length.is_some()))
        || (character.is_some() && (line.is_some() || column.is_some() || end_line.is_some()))
        || (end_line.is_some() && (column.is_some() || length.is_some()))
    {
        return Err("choose one range query: name/level (optionally filtered by line), line/end_line, line/column/length, character/length, or regex; within limits any query".into());
    }
    if column.is_some() && line.is_none()
        || length.is_some() && character.is_none() && column.is_none()
        || end_line.is_some() && line.is_none()
    {
        return Err("column requires line; length requires character or line+column; end_line requires line".into());
    }
    let limit = context.limits.data.max_values;
    let mut found = Vec::new();
    let mut push = |range: Range| -> Result<bool, String> {
        if found.len() >= limit {
            return Err(format!(
                "range query exceeds its {limit}-result budget; narrow the query or use within= another range"
            ));
        }
        found.push(range);
        Ok(!plural && found.len() >= occurrence.unwrap_or(2))
    };
    if ast {
        syntax_ready(&document)?;
        if level == Some(0) {
            return Err("syntax levels are 1-based; use level=1 for outermost syntax nodes".into());
        }
        if name.is_some_and(str::is_empty) {
            return Err("range name cannot be empty; use a qualified declaration such as name=\"WorkQueue.push\"".into());
        }
        let scopes: BTreeMap<_, _> = document
            .scopes
            .iter()
            .map(|scope| (scope.id.as_ref(), scope))
            .collect();
        if name.is_some() {
            context.charge_work(document.scopes.iter().map(|scope| scope.id.len()).sum())?;
        }
        for scope in document.scopes.iter() {
            if level.is_some_and(|level| level != scope.depth)
                || line.is_some_and(|line| !(scope.start_line..=scope.end_line).contains(&line))
            {
                continue;
            }
            if name.is_some_and(|name| {
                scope.id.as_ref() != name
                    && scope
                        .label
                        .split_once(':')
                        .is_none_or(|(_, bare)| bare != name && qualified(scope, &scopes) != name)
            }) {
                continue;
            }
            if !contains(&bounds, &(scope.start_byte..scope.end_byte)) {
                continue;
            }
            if push(index.range(
                scope.start_byte..scope.end_byte,
                Some(Arc::new(RangeOrigin {
                    scope_id: Some(scope.id.clone()),
                    ..origin.as_ref().clone()
                })),
            )?)? {
                break;
            }
        }
    } else if let Some(pattern) = pattern {
        let regex = regex::RegexBuilder::new(pattern)
            .size_limit(context.limits.data.max_bytes)
            .dfa_size_limit(context.limits.data.max_bytes)
            .build()
            .map_err(|error| format!("invalid range regex: {error}"))?;
        let haystack = &index.text()[bounds.clone()];
        let mut at = 0;
        while at <= haystack.len() {
            context.charge_work(
                pattern
                    .len()
                    .max(1)
                    .checked_mul(haystack.len() - at)
                    .ok_or("regex search work budget overflow")?
                    .max(1),
            )?;
            let Some(matched) = regex.find_at(haystack, at) else {
                break;
            };
            if push(index.range(
                (bounds.start + matched.start())..(bounds.start + matched.end()),
                Some(origin.clone()),
            )?)? {
                break;
            }
            if matched.start() == matched.end() {
                if matched.end() == haystack.len() {
                    break;
                }
                at = matched.end()
                    + haystack[matched.end()..]
                        .chars()
                        .next()
                        .expect("nonempty Unicode suffix")
                        .len_utf8();
                if haystack.as_bytes().get(at) == Some(&b'\n')
                    && haystack.as_bytes().get(at - 1) == Some(&b'\r')
                {
                    at += 1;
                }
            } else {
                at = matched.end();
            }
        }
    } else if let Some(character) = character {
        let at = index.character(character)?;
        let end = index.advance(at, length.unwrap_or(usize::from(at < index.text().len())))?;
        if contains(&bounds, &(at..end)) {
            push(index.range(at..end, Some(origin.clone()))?)?;
        }
    } else if let Some(line) = line {
        if let Some(column) = column {
            let at = index.at(line, column)?;
            let end = index.advance(at, length.unwrap_or(0))?;
            if contains(&bounds, &(at..end)) {
                push(index.range(at..end, Some(origin.clone()))?)?;
            }
        } else {
            let end = end_line.unwrap_or(line);
            if end < line {
                return Err("end_line must not precede line".into());
            }
            let start = index.line(line)?.start;
            index.line(end)?;
            let finish = index
                .line(end.checked_add(1).ok_or("line range overflow")?)
                .map_or(index.text().len(), |line| line.start);
            if contains(&bounds, &(start..finish)) {
                push(Range {
                    start_line: line,
                    start_column: 1,
                    end_line: end.checked_add(1).ok_or("line range overflow")?,
                    end_column: 1,
                    origin: Some(origin.clone()),
                })?;
            }
        }
    } else {
        return Err("range query needs name, level, line, character, or regex".into());
    }
    context.charge_work(found.len())?;
    if plural {
        return Ok(Value::List(List::typed(
            range_type(),
            found.into_iter().map(range_value).collect(),
        )));
    }
    if let Some(occurrence) = occurrence {
        return found.into_iter().nth(occurrence.checked_sub(1).ok_or("range occurrence is 1-based")?).map(range_value).ok_or_else(|| format!("range query has no occurrence {occurrence}; use view.ranges(...) to inspect all matches"));
    }
    match found.len() {
        1 => Ok(range_value(found.remove(0))),
        0 => Err("range query matched nothing; check the declaration name, source positions, regex, and within interval, or use view.ranges() to inspect syntax ranges".into()),
        _ => {
            let scopes = document.scopes.iter().map(|scope| (scope.id.as_ref(), scope)).collect::<BTreeMap<_,_>>();
            Err(format!("range query is ambiguous; matches include {}; qualify name, supply within=, or choose occurrence= (1-based)", found.iter().take(MAX_EXAMPLES).map(|range| {
                let label = range.origin.as_ref().and_then(|origin| origin.scope_id.as_ref()).and_then(|id| scopes.get(id.as_ref())).map(|scope| qualified(scope, &scopes)).filter(|label| !label.is_empty()).unwrap_or_else(|| "text".into());
                format!("{label} at {}:{}..{}:{}",range.start_line,range.start_column,range.end_line,range.end_column)
            }).collect::<Vec<_>>().join(", ")))
        },
    }
}

pub(super) fn fold_target<'a>(
    document: &'a Document,
    owner: &str,
    range: &Range,
) -> Result<&'a Scope, String> {
    syntax_ready(document)?;
    range.validate_for(owner, document)?;
    let scope = if let Some(id) = range
        .origin
        .as_ref()
        .and_then(|origin| origin.scope_id.as_ref())
    {
        document
            .scopes
            .iter()
            .find(|scope| scope.id == *id)
            .ok_or_else(|| {
                "selected syntax range no longer exists; select it again from the current CodeView"
                    .to_string()
            })?
    } else {
        let bytes = range.byte_range(document.source()?)?;
        let text = &document.text[bytes.clone()];
        let trimmed = text.trim();
        let start = bytes.start + text.len() - text.trim_start().len();
        let end = start + trimmed.len();
        let mut matching = document.scopes.iter().filter(|scope| {
            scope.start_byte <= start
                && end <= scope.end_byte
                && (start != end || start < scope.end_byte)
        });
        let scope = matching.next().ok_or("selected text is not inside a foldable syntax node; select a declaration with view.range(name=...) or a character inside its body")?;
        matching.fold(scope, |inner, scope| {
            if scope.depth > inner.depth {
                scope
            } else {
                inner
            }
        })
    };
    if scope.start_line == scope.end_line {
        return Err(format!(
            "selected syntax node '{}' is on one line and cannot be folded; select a multiline node",
            scope.label
        ));
    }
    Ok(scope)
}

pub(super) fn action(
    name: &str,
    args: &imbl::OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let value = args
        .get("range")
        .ok_or("range action requires a source range")?;
    let selected = range(value)?;
    let origin = selected.origin.as_ref().ok_or("range actions need an owner; select with view.range(...) and add(view) before using the range")?;
    let view = context.animation.find(&origin.owner).ok_or_else(|| format!("CodeView '{}' is not on the animation timeline; call add(view) before using its range actions", origin.owner))?.clone();
    let owner = Value::Element(Arc::new(view));
    let mut normalized = args.clone();
    normalized.insert("view".into(), owner);
    let operation = match name {
        "CodeRange.fold" => "CodeView.fold",
        "CodeRange.unfold" => "CodeView.unfold",
        "CodeRange.select" => {
            normalized.insert(
                "ranges".into(),
                Value::List(List::typed(range_type(), vec![value.clone()])),
            );
            "CodeView.select"
        }
        "CodeRange.style" => {
            let update = style(
                args.get("style")
                    .ok_or("range.style requires style=CodeStyle(...)")?,
            )?;
            let options = context
                .animation
                .code_views
                .get(origin.owner.as_ref())
                .map(|state| &state.options)
                .or_else(|| {
                    let Value::Element(view) = normalized.get("view")? else {
                        return None;
                    };
                    let Geometry::Code { options, .. } = view.geometry.as_ref() else {
                        return None;
                    };
                    Some(options)
                })
                .ok_or("range owner is not a CodeView")?;
            let mut overrides = options.spans.clone();
            let mut merged = sanim_core::code::view::Style::default();
            for span in overrides.iter().filter(|span| span.range == selected) {
                merged.overlay(&span.style);
            }
            merged.overlay(&update);
            overrides.retain(|span| span.range != selected);
            overrides.push(sanim_core::code::view::Override {
                range: selected.clone(),
                style: merged,
            });
            normalized.insert("spans".into(), spans_value(&overrides));
            "CodeView.style"
        }
        "CodeRange.edit" => "CodeView.patch",
        "CodeRange.scroll_to" => "Element.scroll_to",
        _ => return Err(format!("unknown range action '{name}'")),
    };
    context.charge_work(1)?;
    if operation == "Element.scroll_to" {
        crate::scrolling::execute(operation, &normalized, context)
    } else {
        edits::execute(operation, &normalized, context)
    }
}
