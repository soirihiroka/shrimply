//! Exact execution provenance with direct playback indexing and indexed source invalidation.
use crate::{Fraction, Span, math};
mod frames;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeMapEntry {
    pub source: String,
    pub span: Span,
    pub start: Fraction,
    /// Exclusive finite end; None retains an indefinitely active source interval.
    pub end: Option<Fraction>,
    /// Root execution boundary: resume from this statement's pre-state.
    pub statement: usize,
    pub call_path: imbl::Vector<String>,
    pub clock: Option<Arc<crate::math::cycle::Cycle>>,
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
    frames: Arc<frames::Index>,
    #[serde(skip)]
    sources: BTreeMap<String, SourceIndex>,
    #[serde(skip)]
    event_index: Arc<shrimply_math_core::events::Index>,
    #[serde(skip)]
    metadata_bytes: usize,
}
impl std::fmt::Debug for TimeMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeMap")
            .field("executions", &self.entries.len())
            .field("frames", &self.frames.count())
            .field("sources", &self.sources.len())
            .finish()
    }
}
impl TimeMap {
    pub fn push(&mut self, entry: TimeMapEntry, max_bytes: usize) -> Result<(), String> {
        math::temporal::viewport(entry.start)?;
        if let Some(end) = entry.end {
            math::temporal::viewport(end)?;
            if end < entry.start {
                return Err("source execution ends before it begins".into());
            }
        }
        let bytes = entry.estimated_bytes();
        let metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes)
            .ok_or("time map metadata size overflow")?;
        if metadata_bytes > max_bytes {
            return Err(format!(
                "time map execution metadata exceeds {max_bytes} bytes; reduce repeated scheduling or increase the host's time-map limit"
            ));
        }
        self.metadata_bytes = metadata_bytes;
        self.entries.push_back(Arc::new(entry));
        Ok(())
    }
    /// Direct lookup for small timelines; indexed winner runs for large or repeated timelines.
    pub fn at_frame(&self, frame: usize) -> Option<&TimeMapEntry> {
        self.frames.at(frame, &self.entries)
    }
    /// None is an unbounded source; Some(0) is metadata-only or not indexed yet.
    pub fn indexed_frames(&self) -> Option<usize> {
        self.frames.count()
    }
    /// Shared exact finite and periodic execution boundaries, without expanding playback cycles.
    pub fn event_index(&self) -> Arc<shrimply_math_core::events::Index> {
        self.event_index.clone()
    }
    /// Indexed overlapping source spans; repeated calls are aggregated at their earliest use.
    pub fn earliest(&self, source: &str, span: Span) -> Option<Fraction> {
        self.sources.get(source)?.earliest(span)
    }
    /// Index a source clock once; absent duration is indefinite, absent rate is metadata only.
    pub fn index(
        &mut self,
        rate: Option<Fraction>,
        duration: Option<Fraction>,
        max_bytes: usize,
    ) -> Result<(), String> {
        if let Some(rate) = rate {
            math::frame_count(Fraction::from(0), rate)?;
        }
        if let Some(duration) = duration {
            math::temporal::viewport(duration)?;
        }
        let count = match (rate, duration) {
            (Some(rate), Some(duration)) => Some(
                usize::try_from(math::frame_count(duration, rate)?)
                    .map_err(|_| "time map frame count exceeds address space")?
                    .checked_add(1)
                    .ok_or("time map frame count overflow")?,
            ),
            (Some(_), None) => None,
            (None, _) => Some(0),
        };
        let mut source_spans = BTreeMap::<String, BTreeMap<(usize, usize), Fraction>>::new();
        let mut event_index = shrimply_math_core::events::Builder::new(max_bytes);
        self.metadata_bytes = 0;
        for entry in &self.entries {
            event_index.boundaries([Some(entry.start), entry.end].into_iter().flatten().filter(
                |time| *time >= Fraction::from(0) && duration.is_none_or(|end| *time <= end),
            ))?;
            if let Some(clock) = &entry.clock {
                event_index.clock(
                    clock.start(),
                    clock.end(),
                    clock.until(),
                    [
                        clock.start(),
                        clock.end(),
                        entry.start,
                        entry.end.unwrap_or(clock.end()),
                    ]
                    .into_iter()
                    .filter(|time| *time >= clock.start() && *time <= clock.end()),
                )?;
                if let Some(until) = clock.until() {
                    event_index.boundaries([until])?;
                }
            }
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
        let event_index = event_index.finish()?;
        let total_bytes = self
            .metadata_bytes
            .saturating_add(source_bytes)
            .saturating_add(event_index.estimated_bytes());
        if total_bytes > max_bytes {
            return Err(format!(
                "time map needs approximately {total_bytes} bytes including frame/source indexes; limit {max_bytes}. Reduce scheduling work, frame rate or duration."
            ));
        }
        let frames = if let Some(rate) = rate {
            frames::Index::build(
                &self.entries,
                count,
                rate,
                max_bytes - total_bytes,
                &event_index,
            )?
        } else {
            frames::Index::default()
        };
        self.frames = Arc::new(frames);
        self.event_index = Arc::new(event_index);
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
