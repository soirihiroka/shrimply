use std::collections::BTreeSet;

/// Validates a CFR picture sequence and provides DTS when the encoder omits it.
pub struct DecodeOrder {
    expected: u64,
    presented: u64,
    maximum_reorder: u32,
    pending: BTreeSet<u64>,
    last_dts: Option<i64>,
    native_dts: bool,
}

impl DecodeOrder {
    pub fn new(expected: u64, maximum_reorder: u32) -> Self {
        Self {
            expected,
            presented: 0,
            maximum_reorder: expected.saturating_sub(1).min(u64::from(maximum_reorder)) as u32,
            pending: BTreeSet::new(),
            last_dts: None,
            native_dts: false,
        }
    }

    pub fn next(&mut self, pts: i64, dts: Option<i64>) -> Result<i64, &'static str> {
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
        if self.last_dts.is_none() {
            self.native_dts = dts.is_some();
        }
        let decode = if self.native_dts {
            dts.ok_or("Encoder stopped supplying decode timestamps")?
        } else {
            // Every packet already received is either presented or pending.
            i64::try_from(self.presented + self.pending.len() as u64)
                .map_err(|_| "Decode timestamp exceeds the exact range")?
                - i64::from(self.maximum_reorder)
        };
        if decode > pts || self.last_dts.is_some_and(|previous| previous >= decode) {
            return Err("Encoder returned an invalid decode timestamp");
        }
        self.pending.insert(presentation);
        while self.pending.remove(&self.presented) {
            self.presented += 1;
        }
        self.last_dts = Some(decode);
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

#[cfg(test)]
mod tests {
    use super::DecodeOrder;

    #[test]
    fn validates_reordered_pictures_with_or_without_native_dts() {
        let mut native = DecodeOrder::new(5, 2);
        let mut inferred = DecodeOrder::new(5, 2);
        for (pts, dts) in [0, 4, 2, 1, 3].into_iter().zip(-2..=2) {
            assert_eq!(native.next(pts, Some(dts)), Ok(dts));
            assert_eq!(inferred.next(pts, None), Ok(dts));
        }
        assert_eq!(native.finish(), Ok(()));
        assert_eq!(inferred.finish(), Ok(()));
    }
}
