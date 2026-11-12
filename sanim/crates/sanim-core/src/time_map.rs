//! Exact execution provenance with direct playback indexing and indexed source invalidation.
use crate::{Fraction, Span, math};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TimeMapKind {
    Statement,
    Function,
    Schedule,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeMapEntry {
    pub source: String,
    pub span: Span,
    pub start: Fraction,
    pub end: Fraction,
    /// Root execution boundary: resume from this statement's pre-state.
    pub statement: usize,
    pub kind: TimeMapKind,
    pub call_path: imbl::Vector<String>,
}
impl TimeMapEntry {
    fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.len()
            + self
                .call_path
                .iter()
                .map(|path| path.len() + std::mem::size_of::<String>())
                .sum::<usize>()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TimeMap {
    pub entries: imbl::Vector<Arc<TimeMapEntry>>,
    #[serde(skip)]
    frames: Arc<Vec<Option<Arc<TimeMapEntry>>>>,
    #[serde(skip)]
    sources: BTreeMap<String, SourceIndex>,
    #[serde(skip)]
    metadata_bytes: usize,
}
impl std::fmt::Debug for TimeMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeMap")
            .field("executions", &self.entries.len())
            .field("frames", &self.frames.len())
            .field("sources", &self.sources.len())
            .finish()
    }
}
impl TimeMap {
    pub fn push(&mut self, entry: TimeMapEntry, max_bytes: usize) -> Result<(), String> {
        let bytes = entry.estimated_bytes();
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes)
            .ok_or("time map metadata size overflow")?;
        if self.metadata_bytes > max_bytes {
            return Err(format!(
                "time map execution metadata exceeds {max_bytes} bytes; reduce repeated scheduling or increase the host's time-map limit"
            ));
        }
        self.entries.push_back(Arc::new(entry));
        Ok(())
    }
    /// O(1): direct access to the execution covering this presentation frame.
    pub fn at_frame(&self, frame: usize) -> Option<&TimeMapEntry> {
        self.frames.get(frame)?.as_deref()
    }
    pub fn indexed_frames(&self) -> usize {
        self.frames.len()
    }
    /// Indexed overlapping source spans; repeated calls are aggregated at their earliest use.
    pub fn earliest(&self, source: &str, span: Span) -> Option<Fraction> {
        self.sources.get(source)?.earliest(span)
    }
    /// Build once per successful revision. The sweep never compares every entry to every frame.
    pub fn index(
        &mut self,
        rate: Fraction,
        duration: Fraction,
        max_bytes: usize,
    ) -> Result<(), String> {
        let count = usize::try_from(math::frame_count(duration, rate)?)
            .map_err(|_| "time map frame count exceeds address space")?
            .checked_add(1)
            .ok_or("time map frame count overflow")?;
        let bytes = count
            .checked_mul(std::mem::size_of::<Option<Arc<TimeMapEntry>>>())
            .ok_or("time map byte count overflow")?;
        if bytes > max_bytes {
            return Err(format!(
                "time map needs {bytes} bytes for {count} frames; limit {max_bytes}. Reduce the external frame rate or duration."
            ));
        }
        let mut events = BTreeMap::<usize, Vec<(bool, (TimeMapKind, usize, usize))>>::new();
        let mut source_spans = BTreeMap::<String, BTreeMap<(usize, usize), Fraction>>::new();
        self.metadata_bytes = 0;
        for (index, entry) in self.entries.iter().enumerate() {
            self.metadata_bytes = self
                .metadata_bytes
                .checked_add(entry.estimated_bytes())
                .ok_or("time map metadata size overflow")?;
            if self.metadata_bytes > max_bytes {
                return Err(format!(
                    "time map execution metadata exceeds {max_bytes} bytes"
                ));
            }
            let spans = source_spans.entry(entry.source.clone()).or_default();
            spans
                .entry((entry.span.start, entry.span.end))
                .and_modify(|time| *time = (*time).min(entry.start))
                .or_insert(entry.start);
            let start = usize::try_from(math::frame_count(entry.start, rate)?)
                .map_err(|_| "time map start overflow")?;
            let mut end = usize::try_from(math::frame_count(entry.end, rate)?)
                .map_err(|_| "time map end overflow")?;
            if entry.start == entry.end {
                if math::arithmetic("/", Fraction::from(start as u64), rate)? != entry.start {
                    continue;
                }
                end = start.checked_add(1).ok_or("time map boundary overflow")?;
            }
            if start >= count || end <= start {
                continue;
            }
            let key = (entry.kind, entry.statement, index);
            events.entry(start).or_default().push((true, key));
            events.entry(end).or_default().push((false, key));
        }
        let source_bytes = source_spans
            .iter()
            .map(|(source, spans)| {
                source.len()
                    + spans.len()
                        * (std::mem::size_of::<((usize, usize), Fraction)>()
                            + 4 * std::mem::size_of::<Option<SpanSummary>>())
            })
            .sum::<usize>();
        let total_bytes = bytes
            .saturating_add(self.metadata_bytes)
            .saturating_add(source_bytes);
        if total_bytes > max_bytes {
            return Err(format!(
                "time map needs approximately {total_bytes} bytes including frame/source indexes; limit {max_bytes}. Reduce scheduling work, frame rate or duration."
            ));
        }
        let mut active = BTreeSet::new();
        let mut frames = Vec::with_capacity(count);
        for frame in 0..count {
            if let Some(changes) = events.remove(&frame) {
                // End intervals before opening the next ones at the same exact frame boundary.
                for &(_, key) in changes.iter().filter(|(add, _)| !add) {
                    active.remove(&key);
                }
                for (_, key) in changes.into_iter().filter(|(add, _)| *add) {
                    active.insert(key);
                }
            }
            frames.push(
                active
                    .last()
                    .map(|&(_, _, index)| self.entries[index].clone()),
            );
        }
        self.frames = Arc::new(frames);
        self.sources = source_spans
            .into_iter()
            .map(|(source, spans)| (source, SourceIndex::new(spans)))
            .collect();
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct SourceIndex {
    spans: Vec<((usize, usize), Fraction)>,
    tree: Vec<Option<SpanSummary>>,
}
#[derive(Clone, Copy, Debug)]
struct SpanSummary {
    min_end: usize,
    max_end: usize,
    earliest: Fraction,
}
impl SourceIndex {
    fn new(spans: BTreeMap<(usize, usize), Fraction>) -> Self {
        let mut index = Self {
            spans: spans.into_iter().collect(),
            tree: Vec::new(),
        };
        index.tree.resize(index.spans.len().saturating_mul(4), None);
        if !index.spans.is_empty() {
            index.build(1, 0, index.spans.len());
        }
        index
    }
    fn build(&mut self, node: usize, left: usize, right: usize) -> SpanSummary {
        let summary = if right - left == 1 {
            let ((_, end), earliest) = self.spans[left];
            SpanSummary {
                min_end: end,
                max_end: end,
                earliest,
            }
        } else {
            let middle = left + (right - left) / 2;
            let a = self.build(node * 2, left, middle);
            let b = self.build(node * 2 + 1, middle, right);
            SpanSummary {
                min_end: a.min_end.min(b.min_end),
                max_end: a.max_end.max(b.max_end),
                earliest: a.earliest.min(b.earliest),
            }
        };
        self.tree[node] = Some(summary);
        summary
    }
    fn earliest(&self, span: Span) -> Option<Fraction> {
        let end = span.end.max(span.start.saturating_add(1));
        let limit = self.spans.partition_point(|((start, _), _)| *start < end);
        if limit == 0 {
            return None;
        }
        self.query(1, 0, self.spans.len(), limit, span.start)
    }
    fn query(
        &self,
        node: usize,
        left: usize,
        right: usize,
        limit: usize,
        start: usize,
    ) -> Option<Fraction> {
        if left >= limit {
            return None;
        }
        let summary = self.tree[node]?;
        if summary.max_end <= start {
            return None;
        }
        if right <= limit && summary.min_end > start {
            return Some(summary.earliest);
        }
        let middle = left + (right - left) / 2;
        let a = self.query(node * 2, left, middle, limit, start);
        let b = self.query(node * 2 + 1, middle, right, limit, start);
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}
