use skia_safe::{Path, PathEffect, Rect, StrokeRec};

pub fn apply(path: &Path, effect: &PathEffect, cull: Rect) -> Path {
    effect
        .filter_path(path, &StrokeRec::new_fill(), cull)
        .map(|(mut path, _)| path.detach())
        .unwrap_or_else(|| path.clone())
}
