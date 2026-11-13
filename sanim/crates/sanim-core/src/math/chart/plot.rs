//! Finite, deterministic statistics and shape-preserving sampled curves.
use super::super::float_arithmetic as calc;

pub const MAX_POINTS: usize = 16_384;
pub const MAX_BINS: usize = 4_096;
const NORMAL_FACTOR: f64 = 0.398_942_280_401_432_7;
const SILVERMAN_FACTOR: f64 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization {
    Count,
    Probability,
    Density,
}
impl Normalization {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "count" => Ok(Self::Count),
            "probability" => Ok(Self::Probability),
            "density" => Ok(Self::Density),
            _ => Err("normalization must be count, probability, or density".into()),
        }
    }
}

pub fn finite_samples(values: &[f64]) -> Result<(), String> {
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(format!(
            "sample {} is not finite; explicitly remove or replace missing/non-finite measurements",
            index + 1
        ));
    }
    Ok(())
}

pub fn extent(values: &[f64], padding: f64, include_zero: bool) -> Result<[f64; 2], String> {
    finite_samples(values)?;
    if values.is_empty() {
        return Err(
            "DataRange requires at least one value; use an explicit domain for an empty dataset"
                .into(),
        );
    }
    if !padding.is_finite() || padding < 0. {
        return Err("DataRange padding must be finite and nonnegative".into());
    }
    let mut low = values
        .iter()
        .copied()
        .min_by(f64::total_cmp)
        .expect("nonempty values");
    let mut high = values
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .expect("nonempty values");
    if include_zero {
        low = low.min(0.);
        high = high.max(0.);
    }
    let width = if low == high {
        low.abs().max(1.)
    } else {
        calc("-", high, low)?
    };
    let margin = calc(
        "*",
        width,
        if low == high {
            padding.max(0.05)
        } else {
            padding
        },
    )?;
    let bounds = [calc("-", low, margin)?, calc("+", high, margin)?];
    if bounds[0] >= bounds[1] {
        return Err("DataRange is too narrow to represent; provide a wider explicit domain".into());
    }
    Ok(bounds)
}

pub fn bin_edges(domain: [f64; 2], bins: usize) -> Result<Vec<f64>, String> {
    if !(1..=MAX_BINS).contains(&bins) {
        return Err(format!("bins must be an integer from 1 to {MAX_BINS}"));
    }
    if domain.iter().any(|value| !value.is_finite()) || domain[0] >= domain[1] {
        return Err("histogram domain must have finite increasing endpoints".into());
    }
    let width = calc("-", domain[1], domain[0])?;
    let mut edges = (0..=bins)
        .map(|index| {
            calc(
                "+",
                domain[0],
                calc("*", width, index as f64 / bins as f64)?,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    edges[bins] = domain[1];
    validate_edges(&edges)?;
    Ok(edges)
}

fn validate_edges(edges: &[f64]) -> Result<(), String> {
    if !(2..=MAX_BINS + 1).contains(&edges.len()) {
        return Err(format!(
            "histogram edges must contain 2 to {} values",
            MAX_BINS + 1
        ));
    }
    if edges.iter().any(|edge| !edge.is_finite()) || edges.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("histogram edges must be finite and strictly increasing".into());
    }
    Ok(())
}

/// Bins are [left,right), with the final right edge included. Every sample is
/// accounted for; out-of-range values fail rather than silently disappearing.
pub fn histogram(
    values: &[f64],
    edges: &[f64],
    normalization: Normalization,
) -> Result<Vec<f64>, String> {
    finite_samples(values)?;
    validate_edges(edges)?;
    let mut heights = vec![0.; edges.len() - 1];
    for (index, &value) in values.iter().enumerate() {
        if value < edges[0] || value > edges[edges.len() - 1] {
            return Err(format!(
                "histogram sample {} ({value}) lies outside the bin edges; widen the histogram domain",
                index + 1
            ));
        }
        let bin = edges
            .partition_point(|edge| *edge <= value)
            .saturating_sub(1)
            .min(heights.len() - 1);
        heights[bin] += 1.;
    }
    if !values.is_empty() && normalization != Normalization::Count {
        for (index, height) in heights.iter_mut().enumerate() {
            *height /= values.len() as f64;
            if normalization == Normalization::Density {
                *height = calc("/", *height, calc("-", edges[index + 1], edges[index])?)?;
            }
        }
    }
    Ok(heights)
}

/// ECDF duplicates are collapsed into one right-continuous step at their value.
pub fn ecdf(values: &[f64]) -> Result<Vec<[f64; 2]>, String> {
    finite_samples(values)?;
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut points = Vec::new();
    let mut at = 0;
    while at < sorted.len() {
        let x = sorted[at];
        let start = at;
        while at < sorted.len() && sorted[at] == x {
            at += 1;
        }
        if points.len() > MAX_POINTS - 2 {
            return Err(format!(
                "ECDFPlot exceeds {MAX_POINTS} vertices; aggregate or reduce distinct samples before plotting"
            ));
        }
        points.push([x, start as f64 / sorted.len() as f64]);
        points.push([x, at as f64 / sorted.len() as f64]);
    }
    Ok(points)
}

#[derive(Clone, Copy, Debug)]
pub struct DensityOptions {
    pub domain: [f64; 2],
    pub points: usize,
    pub bandwidth: Option<f64>,
    pub max_work: usize,
}
impl DensityOptions {
    fn grid(self) -> Result<Vec<f64>, String> {
        if !(2..=MAX_POINTS).contains(&self.points) {
            return Err(format!("points must be an integer from 2 to {MAX_POINTS}"));
        }
        if self.domain.iter().any(|value| !value.is_finite()) || self.domain[0] >= self.domain[1] {
            return Err("density domain must have finite increasing endpoints".into());
        }
        let width = calc("-", self.domain[1], self.domain[0])?;
        (0..self.points)
            .map(|index| {
                calc(
                    "+",
                    self.domain[0],
                    calc("*", width, index as f64 / (self.points - 1) as f64)?,
                )
            })
            .collect()
    }
}

pub fn normal_density(
    mean: f64,
    deviation: f64,
    options: DensityOptions,
) -> Result<Vec<[f64; 2]>, String> {
    if !mean.is_finite() || !deviation.is_finite() || deviation <= 0. {
        return Err("NormalCurve requires finite mean and positive finite std_dev".into());
    }
    let grid = options.grid()?;
    if grid.len() > options.max_work {
        return Err(
            "NormalCurve sampling exceeds the configured operation budget; reduce points".into(),
        );
    }
    grid.into_iter()
        .map(|x| {
            let z = calc("/", calc("-", x, mean)?, deviation)?;
            let height = calc("/", NORMAL_FACTOR * (-0.5 * z * z).exp(), deviation)?;
            Ok([x, height])
        })
        .collect()
}

/// Exact Gaussian KDE, normalized over the whole real line. Cropping the visible
/// domain never renormalizes the curve or changes its statistical meaning.
pub fn density(values: &[f64], options: DensityOptions) -> Result<Vec<[f64; 2]>, String> {
    finite_samples(values)?;
    if values.is_empty() {
        return Err("DensityPlot requires at least one sample".into());
    }
    let grid = options.grid()?;
    let work = values
        .len()
        .checked_mul(grid.len())
        .ok_or("DensityPlot work count overflow")?;
    if work > options.max_work {
        return Err(format!(
            "DensityPlot needs {work} kernel evaluations, exceeding the {}-operation budget; reduce points or samples, provide a Histogram, or increase the host operation limit",
            options.max_work
        ));
    }
    let bandwidth = match options.bandwidth {
        Some(value) if value.is_finite() && value > 0. => value,
        Some(_) => return Err("bandwidth must be positive and finite".into()),
        None => {
            if values.len() < 2 {
                return Err("automatic bandwidth requires at least two samples; provide bandwidth explicitly".into());
            }
            let center = values[0];
            let mean = values.iter().try_fold(0., |sum, value| {
                calc(
                    "+",
                    sum,
                    calc("/", calc("-", *value, center)?, values.len() as f64)?,
                )
            })?;
            let variance = values.iter().try_fold(0., |sum, value| {
                let residual = calc("-", calc("-", *value, center)?, mean)?;
                calc(
                    "+",
                    sum,
                    calc(
                        "/",
                        calc("*", residual, residual)?,
                        (values.len() - 1) as f64,
                    )?,
                )
            })?;
            let stats = super::summary(values)?;
            let iqr = calc("/", calc("-", stats[3], stats[1])?, 1.34)?;
            let sigma = variance.sqrt();
            let scale = if iqr > 0. { sigma.min(iqr) } else { sigma };
            let bandwidth = SILVERMAN_FACTOR * scale * (values.len() as f64).powf(-0.2);
            if !bandwidth.is_finite() || bandwidth <= 0. {
                return Err("automatic bandwidth is undefined for a constant dataset; provide a positive bandwidth".into());
            }
            bandwidth
        }
    };
    let factor = calc(
        "/",
        calc("/", NORMAL_FACTOR, bandwidth)?,
        values.len() as f64,
    )?;
    grid.into_iter()
        .map(|x| {
            let sum = values.iter().try_fold(0., |sum, value| {
                let z = calc("/", calc("-", x, *value)?, bandwidth)?;
                calc("+", sum, (-0.5 * z * z).exp())
            })?;
            Ok([x, calc("*", factor, sum)?])
        })
        .collect()
}

/// Piecewise cubic Hermite interpolation with monotone-limited slopes. It never
/// introduces extrema between adjacent measurements. Linear mode preserves the
/// supplied order and can draw parametric paths with repeated/decreasing x.
pub fn curve(
    points: &[[f64; 2]],
    interpolation: &str,
    subdivisions: usize,
) -> Result<Vec<[f64; 2]>, String> {
    if points.iter().flatten().any(|value| !value.is_finite()) {
        return Err("LinePlot coordinates must be finite".into());
    }
    if points.len() > MAX_POINTS {
        return Err(format!(
            "LinePlot supports at most {MAX_POINTS} input points"
        ));
    }
    if interpolation == "linear" {
        return Ok(points.to_vec());
    }
    if interpolation != "monotone" {
        return Err("interpolation must be linear or monotone".into());
    }
    if !(1..=64).contains(&subdivisions) {
        return Err("subdivisions must be an integer from 1 to 64".into());
    }
    if points.windows(2).any(|pair| pair[0][0] >= pair[1][0]) {
        return Err("monotone interpolation requires strictly increasing x coordinates; use linear for paths with repeated or decreasing x".into());
    }
    if points.len() < 2 {
        return Ok(points.to_vec());
    }
    let length=(points.len()-1).checked_mul(subdivisions).and_then(|n|n.checked_add(1)).filter(|n|*n<=MAX_POINTS).ok_or_else(||format!("interpolated curve exceeds {MAX_POINTS} points; reduce subdivisions or input samples"))?;
    let mut secants = Vec::with_capacity(points.len() - 1);
    for pair in points.windows(2) {
        secants.push(calc(
            "/",
            calc("-", pair[1][1], pair[0][1])?,
            calc("-", pair[1][0], pair[0][0])?,
        )?);
    }
    let mut slopes = Vec::with_capacity(points.len());
    slopes.push(secants[0]);
    for pair in secants.windows(2) {
        slopes.push(
            if pair[0] == 0. || pair[1] == 0. || pair[0].signum() != pair[1].signum() {
                0.
            } else {
                // Harmonic mean avoids overflow in products of very large slopes.
                let scale = pair[0].abs().max(pair[1].abs());
                scale
                    * (2. * (pair[0] / scale) * (pair[1] / scale)
                        / ((pair[0] / scale) + (pair[1] / scale)))
            },
        );
    }
    slopes.push(*secants.last().expect("nonempty secants"));
    let mut out = Vec::with_capacity(length);
    for (index, pair) in points.windows(2).enumerate() {
        let width = calc("-", pair[1][0], pair[0][0])?;
        let delta = calc("-", pair[1][1], pair[0][1])?;
        let (mut a, mut b) = (width * slopes[index], width * slopes[index + 1]);
        // Fritsch-Carlson circle limiter preserves each segment's value envelope.
        if delta == 0. {
            a = 0.;
            b = 0.;
        } else {
            let radius = (a / delta).hypot(b / delta);
            if radius > 3. {
                let factor = 3. / radius;
                a *= factor;
                b *= factor;
            }
        }
        for step in 0..subdivisions {
            let t = step as f64 / subdivisions as f64;
            let t2 = t * t;
            let t3 = t2 * t;
            let y = (2. * t3 - 3. * t2 + 1.) * pair[0][1]
                + (t3 - 2. * t2 + t) * a
                + (-2. * t3 + 3. * t2) * pair[1][1]
                + (t3 - t2) * b;
            if !y.is_finite() {
                return Err("monotone interpolation overflow; rescale the data".into());
            }
            out.push([
                calc("+", pair[0][0], calc("*", width, t)?)?,
                y.clamp(pair[0][1].min(pair[1][1]), pair[0][1].max(pair[1][1])),
            ]);
        }
    }
    out.push(*points.last().expect("nonempty points"));
    Ok(out)
}
