use glam::Vec2;

const MANIM_SHAPE_TOLERANCE: f32 = 0.01;

#[derive(Clone, Debug)]
pub struct MorphContour {
    pub curves: Vec<[Vec2; 4]>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MorphPath {
    pub contours: Vec<MorphContour>,
    pub fill_type: MorphFillType,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum MorphFillType {
    #[default]
    Winding,
    EvenOdd,
    InverseWinding,
    InverseEvenOdd,
}

#[derive(Clone, Debug)]
pub struct MorphPathPair {
    pub source: usize,
    pub target: usize,
    pub source_path: MorphPath,
    pub target_path: MorphPath,
}

#[derive(Clone, Debug, Default)]
pub struct MorphMatching {
    pub pairs: Vec<MorphPathPair>,
    pub unmatched_source: Vec<usize>,
    pub unmatched_target: Vec<usize>,
}

pub fn manim_smooth(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let remaining = 1.0 - progress;
    progress.powi(3)
        * (10.0 * remaining * remaining + 5.0 * remaining * progress + progress * progress)
}

pub fn match_morph_paths(source: &[MorphPath], target: &[MorphPath]) -> MorphMatching {
    let mut used_source = vec![false; source.len()];
    let mut used_target = vec![false; target.len()];
    let mut pairs = Vec::new();
    for (source_index, source_path) in source.iter().enumerate() {
        for (target_index, target_path) in target.iter().enumerate() {
            if used_source[source_index]
                || used_target[target_index]
                || !same_morph_shape(source_path, target_path)
            {
                continue;
            }
            let (source_path, target_path) = align_morph_paths(source_path, target_path);
            pairs.push(MorphPathPair {
                source: source_index,
                target: target_index,
                source_path,
                target_path,
            });
            used_source[source_index] = true;
            used_target[target_index] = true;
        }
    }
    MorphMatching {
        pairs,
        unmatched_source: used_source
            .iter()
            .enumerate()
            .filter_map(|(index, used)| (!used).then_some(index))
            .collect(),
        unmatched_target: used_target
            .iter()
            .enumerate()
            .filter_map(|(index, used)| (!used).then_some(index))
            .collect(),
    }
}

pub fn interpolate_morph_path(source: &MorphPath, target: &MorphPath, progress: f32) -> MorphPath {
    let progress = progress.clamp(0.0, 1.0);
    MorphPath {
        contours: source
            .contours
            .iter()
            .zip(&target.contours)
            .map(|(source, target)| MorphContour {
                curves: source
                    .curves
                    .iter()
                    .zip(&target.curves)
                    .map(|(source, target)| {
                        std::array::from_fn(|index| source[index].lerp(target[index], progress))
                    })
                    .collect(),
                closed: if progress < 0.5 {
                    source.closed
                } else {
                    target.closed
                },
            })
            .collect(),
        fill_type: if progress < 0.5 {
            source.fill_type
        } else {
            target.fill_type
        },
    }
}

pub fn collapse_morph_path(path: &MorphPath, point: Vec2, progress: f32) -> MorphPath {
    let progress = progress.clamp(0.0, 1.0);
    MorphPath {
        contours: path
            .contours
            .iter()
            .map(|contour| MorphContour {
                curves: contour
                    .curves
                    .iter()
                    .map(|curve| curve.map(|value| value.lerp(point, progress)))
                    .collect(),
                closed: contour.closed,
            })
            .collect(),
        fill_type: path.fill_type,
    }
}

pub fn morph_path_center(path: &MorphPath) -> Vec2 {
    let Some((minimum, maximum)) = morph_path_bounds(path) else {
        return Vec2::ZERO;
    };
    (minimum + maximum) * 0.5
}

fn same_morph_shape(source: &MorphPath, target: &MorphPath) -> bool {
    let source = morph_points(source);
    let target = morph_points(target);
    if source.len() != target.len() || source.is_empty() {
        return false;
    }
    let normalize = |points: &[Vec2]| {
        let minimum = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let maximum = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        let center = (minimum + maximum) * 0.5;
        let scale = (maximum.y - minimum.y).abs().max(f32::EPSILON);
        (
            points
                .iter()
                .map(|point| (*point - center) / scale)
                .collect::<Vec<_>>(),
            (maximum.x - minimum.x).abs() / scale,
        )
    };
    let (source, source_width) = normalize(&source);
    let (target, _) = normalize(&target);
    let tolerance = source_width * MANIM_SHAPE_TOLERANCE;
    source
        .iter()
        .zip(target)
        .all(|(source, target)| source.abs_diff_eq(target, tolerance))
}

fn align_morph_paths(source: &MorphPath, target: &MorphPath) -> (MorphPath, MorphPath) {
    let mut source_contours = source.contours.clone();
    let mut target_contours = target.contours.clone();
    source_contours.sort_by(|left, right| contour_length(right).total_cmp(&contour_length(left)));
    target_contours.sort_by(|left, right| contour_length(right).total_cmp(&contour_length(left)));
    let contour_count = source_contours.len().max(target_contours.len());
    extend_contours(
        &mut source_contours,
        contour_count,
        morph_path_center(source),
    );
    extend_contours(
        &mut target_contours,
        contour_count,
        morph_path_center(target),
    );
    for (source, target) in source_contours.iter_mut().zip(&mut target_contours) {
        let count = source.curves.len().max(target.curves.len());
        split_to_count(source, count);
        split_to_count(target, count);
    }
    (
        MorphPath {
            contours: source_contours,
            fill_type: source.fill_type,
        },
        MorphPath {
            contours: target_contours,
            fill_type: target.fill_type,
        },
    )
}

fn extend_contours(contours: &mut Vec<MorphContour>, count: usize, fallback: Vec2) {
    if contours.is_empty() {
        contours.push(MorphContour {
            curves: vec![[fallback; 4]],
            closed: true,
        });
    }
    while contours.len() < count {
        let first = contours[0].clone();
        let mut curves = first.curves.clone();
        curves.extend(
            first
                .curves
                .iter()
                .rev()
                .map(|curve| [curve[3], curve[2], curve[1], curve[0]]),
        );
        contours.push(MorphContour {
            curves,
            closed: first.closed,
        });
    }
}

fn split_to_count(contour: &mut MorphContour, count: usize) {
    while contour.curves.len() < count {
        let index = contour
            .curves
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| curve_length(left).total_cmp(&curve_length(right)))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let curve = contour.curves.remove(index);
        let first_control = curve[0].lerp(curve[1], 0.5);
        let middle_control = curve[1].lerp(curve[2], 0.5);
        let second_control = curve[2].lerp(curve[3], 0.5);
        let first_middle = first_control.lerp(middle_control, 0.5);
        let second_middle = middle_control.lerp(second_control, 0.5);
        let middle = first_middle.lerp(second_middle, 0.5);
        contour.curves.splice(
            index..index,
            [
                [curve[0], first_control, first_middle, middle],
                [middle, second_middle, second_control, curve[3]],
            ],
        );
    }
}

fn morph_points(path: &MorphPath) -> Vec<Vec2> {
    path.contours
        .iter()
        .flat_map(|contour| contour.curves.iter().flatten().copied())
        .collect()
}

fn morph_path_bounds(path: &MorphPath) -> Option<(Vec2, Vec2)> {
    let mut points = morph_points(path).into_iter();
    let first = points.next()?;
    Some(points.fold((first, first), |(minimum, maximum), point| {
        (minimum.min(point), maximum.max(point))
    }))
}

fn contour_length(contour: &MorphContour) -> f32 {
    contour.curves.iter().map(curve_length).sum()
}

fn curve_length(curve: &[Vec2; 4]) -> f32 {
    curve[0].distance(curve[1]) + curve[1].distance(curve[2]) + curve[2].distance(curve[3])
}

pub const FLOW_GRID_SPACING: u32 = 4;

pub fn optical_flow_grid_size(width: u32, height: u32) -> Result<glam::UVec2, String> {
    if width < 2 || height < 2 {
        return Err("Morph endpoints must be at least two pixels wide and high".into());
    }
    Ok(glam::UVec2::new(
        (width - 1).div_ceil(FLOW_GRID_SPACING) + 1,
        (height - 1).div_ceil(FLOW_GRID_SPACING) + 1,
    ))
}
