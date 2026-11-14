//! Inspect the renderer's current cached Sanim scene without evaluating source on GTK.
use super::*;
use crate::video::sanim_inspection as inspection;
use shrimply_components_gtk::tr;
use std::path::PathBuf;

const HIT_RADIUS_PX: f32 = 4.0;
const POPOVER_WIDTH_CHARS: i32 = 64;
const POPOVER_MAX_HEIGHT: i32 = 480;

#[derive(Clone, Default)]
pub(super) struct Inspector(Rc<RefCell<Panel>>);

#[derive(Default)]
struct Panel {
    request: u64,
    popover: Option<gtk::Popover>,
    selected: Option<(
        inspection::Inspection,
        glam::Mat3,
        crate::project::ItemAddress,
    )>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    item: uuid::Uuid,
    address: crate::project::ItemAddress,
    path: PathBuf,
    revision: u64,
    position: Time,
}

impl Inspector {
    pub(super) fn dismiss(&self) {
        let popover = {
            let mut panel = self.0.borrow_mut();
            panel.request = panel
                .request
                .checked_add(1)
                .expect("Sanim inspection ID overflow");
            panel.selected = None;
            panel.popover.take()
        };
        if let Some(popover) = popover {
            popover.popdown();
            popover.unparent();
        }
    }

    pub(super) fn draw(
        &self,
        canvas: &skia_safe::Canvas,
        sources: Option<&inspection::FrameSources>,
    ) {
        let panel = self.0.borrow();
        let Some((selected, matrix, item_id)) = &panel.selected else {
            return;
        };
        if !inspection::is_current(selected)
            || sources
                .and_then(|sources| sources.get(item_id))
                .is_none_or(|source| !inspection::matches_frame(selected, source))
        {
            drop(panel);
            self.dismiss();
            return;
        }
        let points = selected
            .quad
            .map(|p| matrix.transform_point2(GlamVec2::from(p)));
        for index in 0..points.len() {
            shrimply_preview_provider_skia::draw_control_line(
                canvas,
                points[index],
                points[(index + 1) % points.len()],
                Color::BLUE5,
            );
        }
    }

    fn open(&self, area: &gtk::GLArea, point: GlamVec2) -> (u64, gtk::Box) {
        self.dismiss();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(POPOVER_MAX_HEIGHT)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .build();
        let popover = gtk::Popover::builder().child(&scroll).build();
        popover.set_parent(area);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            point.x as i32,
            point.y as i32,
            1,
            1,
        )));
        let request = self.0.borrow().request;
        let weak = Rc::downgrade(&self.0);
        let weak_area = area.downgrade();
        popover.connect_closed(move |_| {
            if let Some(panel) = weak.upgrade() {
                let mut panel = panel.borrow_mut();
                if panel.request == request {
                    panel.request = panel
                        .request
                        .checked_add(1)
                        .expect("Sanim inspection ID overflow");
                    panel.selected = None;
                }
            }
            if let Some(area) = weak_area.upgrade() {
                area.queue_render();
            }
        });
        self.0.borrow_mut().popover = Some(popover.clone());
        popover.popup();
        (request, content)
    }
}

fn binding(surface: &ProviderDispatch<'_>) -> Option<Binding> {
    let project = surface.project.borrow();
    let address = selection_state::focused_video_address(surface.selection_state, &project)?;
    let item = project.video_item(&address)?;
    if !matches!(item.content, crate::project::VideoItemContent::Sanim(_)) {
        return None;
    }
    let player = player_state::snapshot(surface.player_state);
    Some(Binding {
        item: item.id,
        address: address.clone(),
        path: item.file.path().to_owned(),
        revision: player.revision,
        position: player.position,
    })
}

pub(super) fn inspect(surface: ProviderDispatch<'_>, point: GlamVec2) {
    let Some(selected) = binding(&surface) else {
        return;
    };
    if player_state::snapshot(surface.player_state).playing {
        return;
    }
    let inspector = surface.state.borrow().sanim_inspector.clone();
    if surface.state.borrow().presentation
        != Some(Presentation {
            position: selected.position,
            revision: selected.revision,
            settled: true,
        })
    {
        let (_, content) = inspector.open(surface.area, point);
        message(
            &content,
            &tr!(
                "Wait for the current Sanim frame to finish loading, then click the element again."
            ),
        );
        return;
    }
    let source = surface
        .state
        .borrow()
        .frame
        .as_ref()
        .and_then(|frame| frame.sanim_sources.get(&selected.address))
        .cloned();
    let Some(source) = source else {
        let (_, content) = inspector.open(surface.area, point);
        message(
            &content,
            &tr!(
                "This preview has no rendered Sanim scene to inspect. Wait for loading to finish, then click again."
            ),
        );
        return;
    };
    let (request, matrix) = {
        let project = surface.project.borrow();
        let state = surface.state.borrow();
        let controller = surface.controller.borrow();
        let Some(address) =
            selection_state::focused_video_address(surface.selection_state, &project)
        else {
            return;
        };
        let Some(position) = project.timeline_time_to_sequence(&address.track(), selected.position)
        else {
            return;
        };
        let Some(item) = project.video_item(&address) else {
            return;
        };
        if position < item.start || position >= item.end {
            return;
        }
        // Reuse editing geometry when available. A focused modifier such as
        // Rasterize need not have an editing provider to inspect its Sanim source.
        let viewport = surface_viewport(surface.area, &project, &state);
        let provider = controller.core.provider.as_ref().filter(|provider| {
            !controller.core.context_invalidated
                && provider.project_revision == selected.revision
                && provider.context.item_id == selected.item
                && provider.context.timeline_position == selected.position
                && provider.context.viewport == viewport
        });
        let fallback;
        let (evaluation, geometry, viewport) = if let Some(provider) = provider {
            let prepared = &provider.context;
            (&prepared.evaluation, prepared.geometry, prepared.viewport)
        } else {
            fallback = shrimply_preview_runtime_cuda::provider::prepare_geometry(
                &project,
                &address,
                selected.position,
                &state.audio_analysis,
                &state.expression_cache,
                viewport,
                Some(&controller.core.extensions),
            );
            let Some(prepared) = fallback.as_ref() else {
                let (_, content) = inspector.open(surface.area, point);
                message(
                    &content,
                    &tr!("This Sanim clip has no inspectable geometry at the current time."),
                );
                return;
            };
            (&prepared.evaluation, prepared.geometry, viewport)
        };
        {
            let mut cache = state.expression_cache.borrow_mut();
            if !crate::transform_eval::resolve_bool(&item.visibility, evaluation, &mut cache)
                || crate::transform_eval::resolve_scalar(
                    &item.compositing.opacity,
                    evaluation,
                    &mut cache,
                ) <= 0.0
            {
                return;
            }
        }
        let matrix = viewport.canvas_to_screen * geometry.local_to_canvas;
        let Some(inverse) = crate::math::inverse_affine(matrix) else {
            return;
        };
        let source_point = inverse.transform_point2(point);
        let source_bounds = Rect::from_min_size(
            GlamVec2::ZERO,
            GlamVec2::new(
                project.canvas_size.width as f32,
                project.canvas_size.height as f32,
            ),
        );
        if !source_bounds.contains(source_point) {
            return;
        }
        let tolerance_px = HIT_RADIUS_PX
            * inverse
                .transform_vector2(GlamVec2::X)
                .length()
                .max(inverse.transform_vector2(GlamVec2::Y).length());
        (
            inspection::Request {
                source,
                item: item.clone(),
                position,
                canvas: project.canvas_size,
                rate: project.fps,
                point: source_point.to_array(),
                tolerance_px,
            },
            matrix,
        )
    };
    let (serial, content) = inspector.open(surface.area, point);
    let receiver = match inspection::request(request) {
        Ok(receiver) => receiver,
        Err(error) => {
            message(&content, &error);
            return;
        }
    };
    let pending = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::builder().spinning(true).build();
    pending.append(&spinner);
    pending.append(&gtk::Label::new(Some(&tr!("Inspecting Sanim element…"))));
    content.append(&pending);
    let area = surface.area.downgrade();
    let project = surface.project.clone();
    let player = surface.player_state.clone();
    let selection = surface.selection_state.clone();
    let focus = surface.preview_focus.clone();
    let state = surface.state.clone();
    let controller = surface.controller.clone();
    // Gio delivers completion to GTK; neither source files nor result channels are polled.
    glib::spawn_future_local(async move {
        let result = gtk::gio::spawn_blocking(move || receiver.recv())
            .await
            .expect("Sanim inspection result delivery panicked")
            .unwrap_or_else(|_| {
                Err(tr!("Sanim inspection stopped before completing.").into_owned())
            });
        if inspector.0.borrow().request != serial {
            return;
        }
        let Some(area) = area.upgrade().filter(|area| area.is_mapped()) else {
            inspector.dismiss();
            return;
        };
        let current = binding(&ProviderDispatch {
            area: &area,
            project: &project,
            player_state: &player,
            selection_state: &selection,
            preview_focus: &focus,
            state: &state,
            controller: &controller,
        });
        if current.as_ref() != Some(&selected)
            || state.borrow().presentation
                != Some(Presentation {
                    position: selected.position,
                    revision: selected.revision,
                    settled: true,
                })
        {
            inspector.dismiss();
            return;
        }
        content.remove(&pending);
        match result {
            Ok(Some(hit))
                if inspection::is_current(&hit)
                    && state
                        .borrow()
                        .frame
                        .as_ref()
                        .and_then(|frame| frame.sanim_sources.get(&selected.address))
                        .is_some_and(|source| inspection::matches_frame(&hit, source)) =>
            {
                show_element(&content, &hit);
                inspector.0.borrow_mut().selected = Some((hit, matrix, selected.address.clone()));
                area.queue_render();
            }
            Ok(Some(_)) => message(
                &content,
                &tr!("Sanim changed while inspecting. Click the element again."),
            ),
            Ok(None) => message(&content, &tr!("No Sanim element at this position.")),
            Err(error) => message(&content, &error),
        }
    });
}

fn message(content: &gtk::Box, text: &str) {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .max_width_chars(POPOVER_WIDTH_CHARS)
        .xalign(0.0)
        .selectable(true)
        .build();
    content.append(&label);
}

fn show_element(content: &gtk::Box, hit: &inspection::Inspection) {
    let title = gtk::Label::builder()
        .label(format!("{} · {}", hit.kind, hit.id))
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .max_width_chars(POPOVER_WIDTH_CHARS)
        .css_classes(["heading"])
        .build();
    content.append(&title);
    if !hit.ancestors.is_empty() {
        message(content, &hit.ancestors.join(" → "));
    }
    if let Some(location) = &hit.creation {
        let label = if hit.source_element.as_deref() == Some(hit.id.as_str()) {
            tr!("Created at")
        } else {
            tr!("Generated by")
        };
        location_row(content, &label, location);
    } else {
        message(content, &tr!("This element has no source location."));
    }
    for location in &hit.call_sites {
        location_row(content, &tr!("Called from"), location);
    }
}

fn location_row(content: &gtk::Box, heading: &str, location: &inspection::Location) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::builder()
        .label(heading)
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("dim-label");
    header.append(&label);
    let reference = format!(
        "{}:{}:{}",
        location.path.display(),
        location.line,
        location.column
    );
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(tr!("Copy source location"))
        .css_classes(["flat"])
        .build();
    let copied = reference.clone();
    copy.connect_clicked(move |button| button.display().clipboard().set_text(&copied));
    header.append(&copy);
    group.append(&header);
    message(&group, &reference);
    let code = gtk::Label::builder()
        .label(location.line_text.as_ref())
        .xalign(0.0)
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(POPOVER_WIDTH_CHARS)
        .tooltip_text(location.line_text.as_ref())
        .css_classes(["monospace"])
        .build();
    group.append(&code);
    let copy_line = gtk::Button::with_label(&tr!("Copy source line"));
    copy_line.set_halign(gtk::Align::Start);
    copy_line.add_css_class("flat");
    let line = location.line_text.clone();
    copy_line.connect_clicked(move |button| button.display().clipboard().set_text(&line));
    group.append(&copy_line);
    content.append(&group);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{
        CanvasSize, SanimItem, VideoItem, VideoItemContent, VideoTrack, VisualModifier,
    };
    use shrimply_preview_provider_skia::{PreviewTarget, PreviewViewport};

    fn scene() -> (
        Project,
        crate::project::ItemAddress,
        PreviewViewport,
        uuid::Uuid,
    ) {
        let canvas = CanvasSize {
            width: 100,
            height: 100,
        };
        let mut item = VideoItem::shape_item(canvas, Time::ZERO, Time { seconds: 1.into() });
        item.content = VideoItemContent::Sanim(Box::<SanimItem>::default());
        item.modifiers = vec![VisualModifier::new(
            crate::modifiers::ModifierEffect::Rasterize(Default::default()),
        )];
        let modifier = item.modifiers[0].id;
        let track = VideoTrack {
            items: vec![item],
            ..Default::default()
        };
        let address = crate::project::ItemAddress::Video {
            sequence_path: vec![],
            track_id: track.id,
            item_id: track.items[0].id,
        };
        let project = Project {
            canvas_size: canvas,
            video_tracks: vec![track],
            ..Default::default()
        };
        let viewport = PreviewViewport::new(
            GlamVec2::splat(100.0),
            Rect::from_min_size(GlamVec2::ZERO, GlamVec2::splat(200.0)),
        );
        (project, address, viewport, modifier)
    }

    #[test]
    fn sanim_inspector_geometry_survives_a_focused_rasterize_without_an_editing_provider() {
        let (project, address, viewport, modifier) = scene();
        let expressions = RefCell::new(TransformExpressionCache::default());
        let prepared = shrimply_preview_runtime_cuda::provider::prepare_geometry(
            &project,
            &address,
            Time::ZERO,
            &FrameAudioAnalysis::default(),
            &expressions,
            viewport,
            None,
        )
        .expect("Sanim source geometry is independent of its modifier editing provider");
        let context = prepared.context(Time::ZERO, &expressions, viewport);
        let item = project.video_item(&address).unwrap();
        let target = PreviewTarget::new(modifier, crate::modifiers::MODIFIER_PREVIEW_FACET);
        assert!(item.owns_preview_target(target));
        assert!(item.preview_provider(target, &context).is_none());
        assert!(
            item.preview_provider(item.default_preview_target(), &context)
                .is_some()
        );
        let matrix = viewport.canvas_to_screen * prepared.geometry.local_to_canvas;
        let point = GlamVec2::splat(50.0);
        let inverse = crate::math::inverse_affine(matrix).unwrap();
        assert!(
            (inverse.transform_point2(matrix.transform_point2(point)) - point).length()
                < f32::EPSILON
        );
    }

    #[test]
    #[ignore = "manual benchmark: make sanim-preview-inspector-bench"]
    fn sanim_inspector_geometry_fallback_benchmark() {
        const CLICKS: u32 = 1000;
        for clips in [1, 1000] {
            let (mut project, address, viewport, _) = scene();
            let prototype = project.video_tracks[0].items[0].clone();
            for _ in 1..clips {
                let mut sibling = prototype.clone();
                sibling.id = uuid::Uuid::new_v4();
                sibling.modifiers.clear();
                Project::regenerate_video_property_ids(&mut sibling);
                project.video_tracks[0].items.push(sibling);
            }
            let expressions = RefCell::new(TransformExpressionCache::default());
            let audio = FrameAudioAnalysis::default();
            let started = std::time::Instant::now();
            for _ in 0..CLICKS {
                std::hint::black_box(
                    shrimply_preview_runtime_cuda::provider::prepare_geometry(
                        &project,
                        &address,
                        Time::ZERO,
                        &audio,
                        &expressions,
                        viewport,
                        None,
                    )
                    .unwrap(),
                );
            }
            let elapsed = started.elapsed();
            eprintln!(
                "Sanim inspector canonical geometry fallback ({clips} clips): {CLICKS} clicks in {elapsed:?}; {:?}/click (debug/release dependent)",
                elapsed / CLICKS
            );
        }
    }
}
