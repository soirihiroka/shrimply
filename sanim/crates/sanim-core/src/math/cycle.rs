//! Exact periodic clocks retain one finite cycle, independent of playback length.
use crate::{Fraction, math};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Data", into = "Data")]
pub struct Cycle {
    start: Fraction,
    end: Fraction,
    until: Option<Fraction>,
}
#[derive(Serialize, Deserialize)]
struct Data {
    start: Fraction,
    end: Fraction,
    #[serde(default)]
    until: Option<Fraction>,
}
impl Cycle {
    pub fn new(start: Fraction, end: Fraction) -> Result<Self, String> {
        let duration = math::arithmetic("-", end, start)?;
        if duration <= Fraction::from(0) {
            return Err("repeat(forever) needs a positive finite cycle duration; include wait(...) or a finite animation".into());
        }
        math::temporal::end(start, duration)?;
        Ok(Self {
            start,
            end,
            until: None,
        })
    }
    pub fn start(self) -> Fraction {
        self.start
    }
    pub fn end(self) -> Fraction {
        self.end
    }
    pub fn until(self) -> Option<Fraction> {
        self.until
    }
    pub fn limited(mut self, end: Fraction) -> Result<Self, String> {
        if !math::finite(&end) || end < self.start {
            return Err("cycle clipping limit must be finite and at or after its start".into());
        }
        self.until = Some(self.until.map_or(end, |previous| previous.min(end)));
        Ok(self)
    }
    pub fn time(self, time: Fraction) -> Result<Fraction, String> {
        if !math::finite(&time) || time < Fraction::from(0) {
            return Err("cycle sample time must be finite and nonnegative".into());
        }
        let stopped = self
            .until
            .is_some_and(|until| time >= until && until > self.start);
        let time = self.until.map_or(time, |until| time.min(until));
        if time < self.end {
            return Ok(time);
        }
        let duration = math::arithmetic("-", self.end, self.start)?;
        let offset = math::arithmetic("-", time, self.start)?;
        let phase = math::arithmetic("%", offset, duration)?;
        if stopped && phase == Fraction::from(0) {
            return Ok(self.end);
        }
        math::arithmetic("+", self.start, phase)
    }
}
impl TryFrom<Data> for Cycle {
    type Error = String;
    fn try_from(data: Data) -> Result<Self, String> {
        let cycle = Self::new(data.start, data.end)?;
        if let Some(until) = data.until {
            if !math::finite(&until) || until < cycle.start {
                return Err("cycle clipping limit must be finite and at or after its start".into());
            }
            cycle.limited(until)
        } else {
            Ok(cycle)
        }
    }
}
impl From<Cycle> for Data {
    fn from(cycle: Cycle) -> Self {
        Self {
            start: cycle.start,
            end: cycle.end,
            until: cycle.until,
        }
    }
}
