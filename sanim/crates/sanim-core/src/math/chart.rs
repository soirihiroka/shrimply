//! Checked chart mathematics, independent of language and rendering.
pub mod plot;
use super::float_arithmetic as calc;

#[derive(Clone, Copy, Debug)]
pub struct LinearScale {
    pub domain: [f64; 2],
    pub range: [f64; 2],
    pub clamp: bool,
}
impl LinearScale {
    pub fn validate(self) -> Result<Self, String> {
        if self
            .domain
            .iter()
            .chain(&self.range)
            .any(|n| !n.is_finite())
        {
            return Err("LinearScale domain and range must be finite".into());
        }
        if self.domain[0] >= self.domain[1] {
            return Err("LinearScale domain must increase: domain.x < domain.y".into());
        }
        if self.range[0] == self.range[1] {
            return Err(
                "LinearScale range must have nonzero length; reverse it for an inverted axis"
                    .into(),
            );
        }
        calc("-", self.domain[1], self.domain[0])?;
        calc("-", self.range[1], self.range[0])?;
        Ok(self)
    }
    pub fn map(self, value: f64) -> Result<f64, String> {
        if !value.is_finite() {
            return Err("chart values must be finite".into());
        }
        let value = if self.clamp {
            value.clamp(self.domain[0], self.domain[1])
        } else {
            value
        };
        if value < self.domain[0] || value > self.domain[1] {
            return Err(format!(
                "chart value {value} lies outside domain {}..{}; widen the domain or explicitly set clamp=true",
                self.domain[0], self.domain[1]
            ));
        }
        let progress = calc(
            "/",
            calc("-", value, self.domain[0])?,
            calc("-", self.domain[1], self.domain[0])?,
        )?;
        calc(
            "+",
            self.range[0],
            calc("*", progress, calc("-", self.range[1], self.range[0])?)?,
        )
    }
    pub fn ticks(self, count: usize) -> Result<Vec<f64>, String> {
        if !(2..=100).contains(&count) {
            return Err("tick_count must be an integer from 2 to 100".into());
        }
        let raw = calc(
            "/",
            calc("-", self.domain[1], self.domain[0])?,
            (count - 1) as f64,
        )?;
        let magnitude = 10f64.powf(raw.log10().floor());
        if magnitude == 0. || !magnitude.is_finite() {
            return Err("chart domain is too small or too large for automatic ticks; provide ticks explicitly".into());
        }
        let ratio = raw / magnitude;
        let step = [1., 2., 2.5, 5., 10.]
            .into_iter()
            .find(|v| *v >= ratio)
            .expect("tick step")
            * magnitude;
        let first = (self.domain[0] / step).ceil();
        let last = (self.domain[1] / step).floor();
        if !first.is_finite() || !last.is_finite() || last - first > 100. {
            return Err("automatic tick calculation overflow; provide ticks explicitly".into());
        }
        Ok((0..=((last - first).max(-1.) as i64))
            .map(|i| (first + i as f64) * step)
            .filter(|v| *v >= self.domain[0] && *v <= self.domain[1])
            .collect())
    }
}

pub fn summary(values: &[f64]) -> Result<[f64; 5], String> {
    if values.is_empty() {
        return Err("BoxSummary requires at least one sample".into());
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err("BoxSummary samples must be finite".into());
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let quantile = |numerator: usize| -> Result<f64, String> {
        let rank = (sorted.len() - 1)
            .checked_mul(numerator)
            .ok_or("quantile sample count overflow")?;
        let index = rank / 4;
        let fraction = (rank % 4) as f64 / 4.;
        let a = sorted[index];
        let b = sorted[(index + 1).min(sorted.len() - 1)];
        calc("+", calc("*", a, 1. - fraction)?, calc("*", b, fraction)?)
    };
    Ok([
        sorted[0],
        quantile(1)?,
        quantile(2)?,
        quantile(3)?,
        *sorted.last().expect("nonempty samples"),
    ])
}
