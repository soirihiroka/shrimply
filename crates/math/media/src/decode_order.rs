use std::collections::BTreeSet;

/// Exact CFR decode timestamps for encoders that emit presentation timestamps only.
/// The pending set is bounded by the configured number of reordered frames.
pub struct DecodeOrder {
    expected: u64,
    decoded: u64,
    presented: u64,
    maximum_reorder: u32,
    pending: BTreeSet<u64>,
}

impl DecodeOrder {
    pub fn new(expected: u64, maximum_reorder: u32) -> Self {
        Self {
            expected,
            decoded: 0,
            presented: 0,
            maximum_reorder,
            pending: BTreeSet::new(),
        }
    }

    pub fn next(&mut self, presentation: i64) -> Result<i64, &'static str> {
        let presentation = u64::try_from(presentation)
            .map_err(|_| "Encoder returned a negative presentation timestamp")?;
        if presentation >= self.expected || self.decoded >= self.expected {
            return Err("Encoder returned a frame outside the submitted timeline");
        }
        if presentation < self.presented || self.pending.contains(&presentation) {
            return Err("Encoder returned a duplicate presentation timestamp");
        }
        if presentation.saturating_sub(self.presented) > u64::from(self.maximum_reorder) {
            return Err("Encoder exceeded the configured B-frame reorder limit");
        }
        let decode = i64::try_from(self.decoded)
            .map_err(|_| "Decode timestamp exceeds the exact range")?
            .checked_sub(i64::from(self.maximum_reorder))
            .ok_or("Decode timestamp exceeds the exact range")?;
        if decode
            > i64::try_from(presentation)
                .map_err(|_| "Presentation timestamp exceeds the exact range")?
        {
            return Err("Encoder presentation timestamp precedes its decode timestamp");
        }
        self.pending.insert(presentation);
        while self.pending.remove(&self.presented) {
            self.presented += 1;
        }
        self.decoded += 1;
        Ok(decode)
    }

    pub fn finish(&self) -> Result<(), &'static str> {
        if self.decoded == self.expected
            && self.presented == self.expected
            && self.pending.is_empty()
        {
            Ok(())
        } else {
            Err("Encoder did not return every submitted video frame")
        }
    }
}
