use std::collections::BTreeSet;

/// Exact CFR decode timestamps for encoders that emit presentation timestamps only.
/// The pending set is bounded by the codec's picture reorder capacity.
pub struct DecodeOrder {
    expected: u64,
    presented: u64,
    maximum_reorder: u32,
    pending: BTreeSet<u64>,
}

impl DecodeOrder {
    pub fn new(expected: u64, maximum_reorder: u32) -> Self {
        Self {
            expected,
            presented: 0,
            maximum_reorder: expected.saturating_sub(1).min(u64::from(maximum_reorder)) as u32,
            pending: BTreeSet::new(),
        }
    }

    pub fn next(&mut self, pts: i64) -> Result<i64, &'static str> {
        let presentation =
            u64::try_from(pts).map_err(|_| "Encoder returned a negative presentation timestamp")?;
        if presentation >= self.expected {
            return Err("Encoder returned a frame outside the submitted timeline");
        }
        if presentation < self.presented || self.pending.contains(&presentation) {
            return Err("Encoder returned a duplicate presentation timestamp");
        }
        // A future reference can jump ahead by several presentation slots
        // while occupying only one slot in the decoder's reorder buffer.
        if presentation != self.presented && self.pending.len() == self.maximum_reorder as usize {
            return Err("Encoder exceeded the codec's picture reorder capacity");
        }
        // Every packet already received is either presented or pending.
        let decode = i64::try_from(self.presented + self.pending.len() as u64)
            .map_err(|_| "Decode timestamp exceeds the exact range")?
            - i64::from(self.maximum_reorder);
        self.pending.insert(presentation);
        while self.pending.remove(&self.presented) {
            self.presented += 1;
        }
        Ok(decode)
    }

    pub fn finish(&self) -> Result<(), &'static str> {
        if self.presented == self.expected {
            Ok(())
        } else {
            Err("Encoder did not return every submitted video frame")
        }
    }
}
