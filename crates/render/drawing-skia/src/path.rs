use skia_safe::{Path, PathBuilder, PathMeasure};

pub fn contours(path: &Path) -> Vec<Path> {
    measured_contours(path)
        .expect("Skia path contour measurement failed")
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

fn measured_contours(path: &Path) -> Result<Vec<(Path, f32)>, String> {
    if !path.is_finite() {
        return Err("path coordinates exceed Skia numeric range".into());
    }
    // Preflight in f64: Skia may report zero instead of infinity for an oversized segment.
    let control_length: f64 = path
        .iter()
        .map(|segment| {
            segment
                .points()
                .windows(2)
                .map(|points| {
                    (f64::from(points[1].x) - f64::from(points[0].x))
                        .hypot(f64::from(points[1].y) - f64::from(points[0].y))
                })
                .sum::<f64>()
        })
        .sum();
    if control_length > f64::from(f32::MAX) {
        return Err(
            "path contour length exceeds Skia numeric range; reduce coordinate magnitudes".into(),
        );
    }
    let mut measure = PathMeasure::new(path, false, None);
    let mut contours = Vec::new();
    loop {
        let length = measure.length();
        if !length.is_finite() || length < 0.0 {
            return Err(
                "path contour length exceeds Skia numeric range; reduce coordinate magnitudes"
                    .into(),
            );
        }
        if length > 0.0 {
            let mut contour = PathBuilder::new();
            if !measure.get_segment(0.0, length, &mut contour, true) {
                return Err("Skia could not extract a path contour".into());
            }
            if measure.is_closed() {
                contour.close();
            }
            contours.push((contour.detach(), length));
        }
        if !measure.next_contour() {
            break;
        }
    }
    if contours.is_empty() && control_length > 0.0 {
        return Err(
            "Skia could not measure non-degenerate path geometry; reduce coordinate magnitudes"
                .into(),
        );
    }
    Ok(contours)
}

/// Cached contour lengths for repeated path reveals. Contains vector geometry only.
#[derive(Clone)]
pub struct PreparedPath {
    pub path: Path,
    contours: Vec<(Path, f32)>,
    length: f32,
}

impl PreparedPath {
    pub fn new(path: Path) -> Result<Self, String> {
        let contours = measured_contours(&path)?;
        let length: f32 = contours.iter().map(|(_, length)| length).sum();
        if !length.is_finite() {
            return Err(
                "total path length exceeds Skia numeric range; reduce geometry size".into(),
            );
        }
        Ok(Self {
            path,
            contours,
            length,
        })
    }

    pub fn approximate_bytes_used(&self) -> usize {
        self.contours
            .iter()
            .fold(self.path.approximate_bytes_used(), |total, (path, _)| {
                total.saturating_add(path.approximate_bytes_used())
            })
    }

    pub fn partial(&self, progress: f32, reverse: bool) -> Path {
        if progress <= 0.0 {
            return Path::default();
        }
        if progress >= 1.0 {
            return self.path.clone();
        }
        let mut remaining = self.length * progress.clamp(0.0, 1.0);
        let mut output = PathBuilder::new_with_fill_type(self.path.fill_type());
        for ordinal in 0..self.contours.len() {
            let index = if reverse {
                self.contours.len() - 1 - ordinal
            } else {
                ordinal
            };
            let (contour, length) = &self.contours[index];
            let visible = remaining.clamp(0.0, *length);
            remaining -= visible;
            if visible <= 0.0 {
                break;
            }
            if visible >= *length {
                output.add_path(contour, None);
                continue;
            }
            let mut measure = PathMeasure::new(contour, false, None);
            let (start, end) = if reverse {
                (length - visible, *length)
            } else {
                (0.0, visible)
            };
            measure.get_segment(start, end, &mut output, true);
        }
        output.detach()
    }
}

pub fn partial_path(path: &Path, progress: f32, reverse: bool) -> Path {
    if progress <= 0.0 {
        return Path::default();
    }
    if progress >= 1.0 {
        return path.clone();
    }
    PreparedPath::new(path.clone())
        .expect("Skia path contour measurement failed")
        .partial(progress, reverse)
}
