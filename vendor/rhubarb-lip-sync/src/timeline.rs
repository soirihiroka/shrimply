// Ported from Rhubarb Lip Sync's half-open Timeline types.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Span<T> {
    pub start: i64,
    pub end: i64,
    pub value: T,
}

impl<T> Span<T> {
    pub const fn duration(&self) -> i64 {
        self.end - self.start
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Timeline<T> {
    start: i64,
    end: i64,
    auto_join: bool,
    spans: Vec<Span<T>>,
}

impl<T: Clone + Eq> Timeline<T> {
    pub fn new(start: i64, end: i64, value: T) -> Self {
        Self::with_joining(start, end, value, true)
    }

    pub fn new_unjoined(start: i64, end: i64, value: T) -> Self {
        Self::with_joining(start, end, value, false)
    }

    fn with_joining(start: i64, end: i64, value: T, auto_join: bool) -> Self {
        assert!(start <= end, "timeline start is after its end");
        let spans = (start < end)
            .then(|| Span { start, end, value })
            .into_iter()
            .collect();
        Self {
            start,
            end,
            auto_join,
            spans,
        }
    }

    pub const fn start(&self) -> i64 {
        self.start
    }

    pub const fn end(&self) -> i64 {
        self.end
    }

    pub fn spans(&self) -> &[Span<T>] {
        &self.spans
    }

    pub fn set(&mut self, start: i64, end: i64, value: T) {
        let start = start.max(self.start);
        let end = end.min(self.end);
        if start >= end {
            return;
        }

        let mut next = Vec::with_capacity(self.spans.len() + 2);
        for span in &self.spans {
            if span.end <= start || span.start >= end {
                next.push(span.clone());
                continue;
            }
            if span.start < start {
                next.push(Span {
                    start: span.start,
                    end: start,
                    value: span.value.clone(),
                });
            }
            if span.end > end {
                next.push(Span {
                    start: end,
                    end: span.end,
                    value: span.value.clone(),
                });
            }
        }
        next.push(Span { start, end, value });
        next.sort_by_key(|span| span.start);
        self.spans = if self.auto_join { join(next) } else { next };
    }

    pub fn clipped_spans(&self, start: i64, end: i64) -> Vec<Span<T>> {
        self.spans
            .iter()
            .filter_map(|span| {
                let clipped_start = span.start.max(start);
                let clipped_end = span.end.min(end);
                (clipped_start < clipped_end).then(|| Span {
                    start: clipped_start,
                    end: clipped_end,
                    value: span.value.clone(),
                })
            })
            .collect()
    }
}

pub(crate) fn join<T: Clone + Eq>(spans: Vec<Span<T>>) -> Vec<Span<T>> {
    let mut result: Vec<Span<T>> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.start >= span.end {
            continue;
        }
        if let Some(previous) = result.last_mut()
            && previous.end == span.start
            && previous.value == span.value
        {
            previous.end = span.end;
        } else {
            result.push(span);
        }
    }
    result
}
