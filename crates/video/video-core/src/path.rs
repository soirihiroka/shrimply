use skia_safe::{Path, PathBuilder, PathMeasure};

pub fn contours(path: &Path) -> Vec<Path> {
    let mut measure = PathMeasure::new(path, false, None);
    let mut contours = Vec::new();
    loop {
        let length = measure.length();
        if length > 0.0 {
            let mut contour = PathBuilder::new();
            measure.get_segment(0.0, length, &mut contour, true);
            if measure.is_closed() {
                contour.close();
            }
            contours.push(contour.detach());
        }
        if !measure.next_contour() {
            break;
        }
    }
    contours
}
