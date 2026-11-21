//! Writer compatibility compares active occurrences, without expanding repeat clocks.
use super::Interval;
use crate::{Fraction, math};

const MAX_OVERLAP_STEPS: usize = 1024;
// Equal clocks either meet in the current/next period or a finite limit ends them.
const SAME_PERIOD_OVERLAP_STEPS: usize = 3;
#[derive(Clone, Copy)]
struct Pulse {
    start: Fraction,
    width: Fraction,
    period: Option<Fraction>,
    until: Option<Fraction>,
}
impl Pulse {
    fn new(interval: Interval) -> Result<Self, String> {
        if !math::finite(&interval.start)
            || !math::finite(&interval.end)
            || interval.start < 0.into()
            || interval.end < interval.start
        {
            return Err("writer overlap requires finite nonnegative ordered intervals".into());
        }
        if interval
            .cycle
            .is_some_and(|cycle| interval.start < cycle.start() || interval.end > cycle.end())
        {
            return Err("writer overlap interval lies outside its repeat cycle".into());
        }
        Ok(Self {
            start: interval.start,
            width: math::arithmetic("-", interval.end, interval.start)?,
            period: interval
                .cycle
                .map(|cycle| math::arithmetic("-", cycle.end(), cycle.start()))
                .transpose()?,
            until: interval.cycle.and_then(|cycle| cycle.until()),
        })
    }
    // The first occurrence active at time, or beginning after it. Point writers
    // occupy their exact timestamp; positive writers occupy [start, end).
    fn next(self, time: Fraction) -> Result<Option<(Fraction, Fraction)>, String> {
        let mut start = self.start;
        if let Some(period) = self.period
            && time > start
        {
            let offset = math::arithmetic("-", time, start)?;
            let phase = math::arithmetic("%", offset, period)?;
            start = math::arithmetic("-", time, phase)?;
            if phase >= self.width && !(self.width == 0.into() && phase == 0.into()) {
                start = math::arithmetic("+", start, period)?;
            }
        }
        let mut end = math::arithmetic("+", start, self.width)?;
        if self
            .until
            .is_some_and(|until| start > until || start == until && self.width > 0.into())
        {
            return Ok(None);
        }
        if let Some(until) = self.until {
            end = end.min(until);
        }
        if time > end || time == end && self.width > 0.into() {
            return Ok(None);
        }
        Ok(Some((start, end)))
    }
}
fn intersects((a, b): (Fraction, Fraction), (c, d): (Fraction, Fraction)) -> bool {
    match (a == b, c == d) {
        (true, true) => a == c,
        (true, false) => c <= a && a < d,
        (false, true) => a <= c && c < b,
        (false, false) => a < d && c < b,
    }
}
// Rational periods form an exact lattice. Euclid bounds work by numeric bit
// width; no playback duration or number of repeated occurrences is traversed.
fn lattice_possible(a: Pulse, b: Pulse) -> Result<bool, String> {
    let (mut gcd, mut remainder) = (
        a.period.expect("periodic pulse"),
        b.period.expect("periodic pulse"),
    );
    for _ in 0..MAX_OVERLAP_STEPS {
        if remainder == 0.into() {
            let delta = math::arithmetic("-", b.start, a.start)?;
            let mut phase = math::arithmetic("%", delta, gcd)?;
            if phase < 0.into() {
                phase = math::arithmetic("+", phase, gcd)?;
            }
            return Ok(phase == 0.into()
                || phase < a.width
                || math::arithmetic("-", gcd, phase)? < b.width);
        }
        (gcd, remainder) = (remainder, math::arithmetic("%", gcd, remainder)?);
    }
    Err("temporal overlap proof exceeded its arithmetic work budget".into())
}
impl Interval {
    /// Exact half-open writer intersection, including finite repeat limits.
    /// Distinct finite clocks that cannot be proved within the bounded solver
    /// fail explicitly instead of accepting potentially conflicting writers.
    pub fn overlaps(self, other: Self) -> Result<bool, String> {
        let (a, b) = (Pulse::new(self)?, Pulse::new(other)?);
        if a.period.is_none() || b.period.is_none() {
            let (fixed, periodic) = if a.period.is_none() { (a, b) } else { (b, a) };
            let fixed = (
                fixed.start,
                math::arithmetic("+", fixed.start, fixed.width)?,
            );
            return Ok(periodic
                .next(fixed.0)?
                .is_some_and(|next| intersects(fixed, next)));
        }
        if !lattice_possible(a, b)? {
            return Ok(false);
        }
        if a.until.is_none() && b.until.is_none() {
            return Ok(true);
        }
        let steps = if a.period == b.period {
            SAME_PERIOD_OVERLAP_STEPS
        } else {
            MAX_OVERLAP_STEPS
        };
        let mut time = a.start.max(b.start);
        for _ in 0..steps {
            let (Some(left), Some(right)) = (a.next(time)?, b.next(time)?) else {
                return Ok(false);
            };
            if intersects(left, right) {
                return Ok(true);
            }
            let next = left.0.max(right.0);
            if next <= time {
                return Err("temporal overlap solver did not advance".into());
            }
            time = next;
        }
        Err("temporal overlap proof exceeded its work budget for distinct finite repeat clocks; align their periods or separate their active windows".into())
    }
}
