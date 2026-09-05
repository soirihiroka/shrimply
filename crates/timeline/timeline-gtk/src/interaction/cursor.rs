use super::*;
pub(crate) fn timeline_cursor(
    _project: &Project,
    runtime: &TimelineRuntime,
    x: f64,
    y: f64,
) -> TimelineCursor {
    runtime.scene.cursor_at(vec2(x as f32, y as f32))
}
