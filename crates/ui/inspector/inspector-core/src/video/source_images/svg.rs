use super::*;
use shrimply_project_document::project::{Color, SvgColorOverride, SvgPaintKind};
const PREFIX: &str = "/@svg-color/";
pub(super) fn controls(
    item: &VideoItem,
    snapshot: &AssetSnapshot,
    colors: &[SvgPaintColor],
) -> Vec<InspectorControl> {
    let mut fills = 0;
    let mut strokes = 0;
    colors
        .iter()
        .map(|paint| {
            let (kind, label, count) = match paint.kind {
                SvgPaintKind::Fill => {
                    fills += 1;
                    ("fill", "Fill color %{number}", fills)
                }
                SvgPaintKind::Stroke => {
                    strokes += 1;
                    ("stroke", "Stroke color %{number}", strokes)
                }
            };
            let [r, g, b, a] = paint.color.to_array();
            let path = format!("{PREFIX}{}/{kind}/{r}/{g}/{b}/{a}", snapshot.cache_key());
            let value = item
                .svg_color_overrides
                .iter()
                .find(|value| value.kind == paint.kind && value.original == paint.color)
                .map_or(paint.color, |value| value.replacement);
            InspectorControl::new(
                ControlKind::Color,
                path,
                shrimply_i18n::text_args(label, &[("number", count.to_string())]),
            )
            .components(value.to_array().map(|value| value.to_string()).into())
            .tooltip("SVG color")
            .live_commit("svg-color-override")
        })
        .collect()
}
impl InspectorController {
    pub(crate) fn set_svg_color(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Color<u8>,
    ) -> Option<Result<(), String>> {
        let descriptor = path.strip_prefix(PREFIX)?;
        Some((|| {
            let InspectorTarget::Item(address) = target else {
                return Err("SVG color requires a video item".into());
            };
            let project = self.project.borrow();
            let item = project
                .video_item(address)
                .filter(|item| matches!(item.content, VideoItemContent::Svg))
                .ok_or("SVG item is no longer available")?;
            let State::Ready(snapshot, data) = self.visual_sources.borrow_mut().request(item)
            else {
                return Err("SVG source colors are not ready".into());
            };
            snapshot.verify_current()?;
            let fields = descriptor.split('/').collect::<Vec<_>>();
            if fields.len() != 6 || fields[0] != snapshot.cache_key() {
                return Err("SVG source changed; refresh the inspector".into());
            }
            let kind = match fields[1] {
                "fill" => SvgPaintKind::Fill,
                "stroke" => SvgPaintKind::Stroke,
                _ => return Err("Unknown SVG paint kind".into()),
            };
            let channels = fields[2..]
                .iter()
                .map(|value| value.parse::<u8>().map_err(|_| "Invalid SVG color"))
                .collect::<Result<Vec<_>, _>>()?;
            let original = Color::new(channels[0], channels[1], channels[2], channels[3]);
            let Data::Svg(colors) = &*data else {
                return Err("Source is no longer an SVG".into());
            };
            if !colors
                .iter()
                .any(|paint| paint.kind == kind && paint.color == original)
            {
                return Err("SVG source color is no longer available".into());
            }
            drop(project);
            let mut project = self.project.borrow_mut();
            let item = project.video_item_mut(address).expect("validated SVG item");
            if let Some(index) = item
                .svg_color_overrides
                .iter()
                .position(|value| value.kind == kind && value.original == original)
            {
                if item.svg_color_overrides[index].replacement == replacement {
                    return Ok(());
                }
                if replacement == original {
                    item.svg_color_overrides.remove(index);
                } else {
                    item.svg_color_overrides[index].replacement = replacement;
                }
            } else if replacement != original {
                if item.svg_color_overrides.len()
                    >= shrimply_project_document::svg_color::SVG_COLOR_LIMIT
                {
                    return Err("SVG color override limit reached".into());
                }
                item.svg_color_overrides.push(SvgColorOverride {
                    kind,
                    original,
                    replacement,
                });
            } else {
                return Ok(());
            }
            shrimply_project_document::project::commit_coalesced_edit(
                &project,
                "svg-color-override",
            );
            drop(project);
            shrimply_editor_state::player_state::refresh_project(
                &self.player_state,
                shrimply_editor_state::player_state::ProjectChange {
                    video: true,
                    ..Default::default()
                },
            );
            Ok(())
        })())
    }
}
