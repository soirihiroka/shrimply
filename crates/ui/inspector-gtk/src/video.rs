use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nWidgetExt;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use adw::prelude::{ActionRowExt, ComboRowExt, PreferencesRowExt};
use gtk::glib;
use gtk::prelude::*;

use crate::InspectedItem as SelectedItem;
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::timeline_value::*;
use shrimply_gtk_components::resource_pipeline::{UiSubscription, deliver};
use shrimply_gtk_components::ui::{ColorPicker, control_row, dropdown};
use shrimply_math_core::{fraction_denominator, fraction_numerator};
use shrimply_project::project::{
    AssetSnapshot, AudioItem, AudioSource, AudioTrack, Color, LayerBlendMode, LayerVisibility,
    ManimParameterValue, MeshFlowAdaptiveWeights, Project, ResolvedTransform, SkiaDrawingStrategy,
    SvgColorOverride, SvgPaintKind, Time, Transform, VideoItem, VideoItemContent,
    VideoSampleMethod, VideoStabilizationMethod, VisualAlphaMaskTarget, VisualCompositing,
};
use shrimply_project::svg_color;
use shrimply_resource_pipeline::{Event, JobContext, Pipeline, Processor};
use shrimply_video_modifiers::VisualKind;

use super::{
    Inspectable, InspectorContext,
    item::{DefaultInspectorItem, HeaderAction, InspectorListItem},
    list,
    section::InspectorSection,
    selector::{StringChoice, labeled_string_selector, selector},
    timeline_value::boolean::{BoolTarget, bool_control},
    timeline_value::scalar::{ScalarAccess, ScalarSpec, ScalarTarget, scalar_control},
    timeline_value::step::{StepTarget, step_control},
};

mod blender;
mod manim_parameters;
mod pdf;
mod playback;

const SVG_COLOR_DELIVERY_INTERVAL: Duration = Duration::from_millis(50);

fn header_actions(
    actions: Vec<
        shrimply_inspector_core::item::HeaderAction<
            shrimply_inspector_core::video::VideoCardAction,
        >,
    >,
) -> Vec<HeaderAction> {
    actions
        .into_iter()
        .map(|action| HeaderAction {
            icon: action.icon,
            tooltip: action.tooltip,
            sensitive: action.sensitive,
            activate: Rc::new(move || match &action.activate {
                shrimply_inspector_core::video::VideoCardAction::ReloadAsset { asset, kind } => {
                    if let Err(error) = shrimply_inspector_core::video::reload_asset(asset, *kind) {
                        tracing::warn!(%error, "Could not reload video source");
                    }
                }
            }),
        })
        .collect()
}

impl Inspectable for VideoItem {
    fn title(&self) -> &'static str {
        if self.video_generation.is_some() {
            return "Video Generation";
        }
        match self.content {
            shrimply_project::project::VideoItemContent::Text(_) => "Text",
            shrimply_project::project::VideoItemContent::Shape(_) => "Shape",
            shrimply_project::project::VideoItemContent::Paint(_) => "Paint",
            shrimply_project::project::VideoItemContent::Background(_) => "Background",
            shrimply_project::project::VideoItemContent::Media => "Video",
            shrimply_project::project::VideoItemContent::Image => "Image",
            shrimply_project::project::VideoItemContent::Gif => "GIF",
            shrimply_project::project::VideoItemContent::Svg => "SVG",
            shrimply_project::project::VideoItemContent::Pdf(_) => "PDF",
            shrimply_project::project::VideoItemContent::Manim(_) => "Manim",
            shrimply_project::project::VideoItemContent::Blender(_) => "Blender",
            shrimply_project::project::VideoItemContent::LayeredImage(_) => "Layered Image",
            shrimply_project::project::VideoItemContent::Obj(_) => "OBJ",
            shrimply_project::project::VideoItemContent::Gaussian(_) => "3D Gaussian Splat",
            shrimply_project::project::VideoItemContent::FoldedSequence(_) => "Folded Sequence",
        }
    }

    fn add_rows(&self, _section: &InspectorSection, _context: &InspectorContext) {}

    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let mut visual_items = Vec::new();
        let mut playback_items = Vec::new();
        let static_visual = self.is_static_visual_media() || self.is_generated();
        let stream_rows = match &self.content {
            VideoItemContent::Media => video_stream_rows(self, context),
            _ => Vec::new(),
        };
        if let Some(settings) = &self.video_generation {
            let id = self.id;
            visual_items.push(
                DefaultInspectorItem::new(
                    "video-generation",
                    "Video Generation",
                    (**settings).clone(),
                    move |settings, context| vec![video_generation_editor(id, settings, context)],
                    |context, settings| {
                        let Some(key) = context.selected_item.clone() else {
                            return;
                        };
                        let mut project = context.project.borrow_mut();
                        let Some(item) = project.video_item_mut(&key) else {
                            return;
                        };
                        let Some(current) = &mut item.video_generation else {
                            return;
                        };
                        **current = settings;
                        shrimply_project::project::commit_edit(&project, "reset-video-generation");
                        drop(project);
                        (context.refresh)();
                    },
                )
                .boxed(),
            );
        }
        if matches!(self.content, VideoItemContent::Media) {
            visual_items.push(
                DefaultInspectorItem::new(
                    "stabilization",
                    "Stabilization",
                    Some(self.clone()),
                    video_stabilization_controls,
                    reset_video_stabilization,
                )
                .boxed(),
            );
        }
        let info_items = vec![super::info::item(
            context,
            super::info::ItemInfo {
                leading: stream_rows,
                kind: self.title(),
                natural_duration: (!static_visual).then_some(self.source_duration),
                start: self.start,
                end: self.end,
                source_offset: Some(self.time_offset),
                dimensions: Some(glam::UVec2::new(self.source_width, self.source_height)),
                file: (!matches!(self.content, VideoItemContent::FoldedSequence(_)))
                    .then(|| self.file.clone()),
                source_metadata: if matches!(
                    self.content,
                    shrimply_project::project::VideoItemContent::Media
                        | shrimply_project::project::VideoItemContent::Gif
                ) {
                    super::info::SourceMetadata::Video(self.track_id)
                } else {
                    super::info::SourceMetadata::None
                },
            },
        )];
        if let Some(manim) = shrimply_inspector_core::manim_parameters::presentation(self) {
            visual_items.push(manim_item(manim.clone()));
            if let Some((card, reset)) =
                manim.parameters.clone().zip(manim.parameters_reset.clone())
            {
                visual_items.push(manim_parameters::item(card, reset));
            }
        }
        if let VideoItemContent::Blender(blender) = &self.content {
            visual_items.push(blender::item(blender, self.file.clone(), context));
        }
        if let VideoItemContent::Pdf(pdf) = &self.content {
            visual_items.push(pdf::item(pdf, context));
        }
        if let Some(items) = super::generated::items(self, context) {
            visual_items.extend(items);
        } else {
            match &self.content {
                shrimply_project::project::VideoItemContent::Paint(paint) => {
                    visual_items.extend(super::paint::items(paint));
                }
                shrimply_project::project::VideoItemContent::Background(background) => {
                    visual_items.push(super::background::item(background));
                }
                shrimply_project::project::VideoItemContent::Obj(scene) => {
                    visual_items.extend(super::scene_3d::items(scene, context));
                }
                shrimply_project::project::VideoItemContent::Gaussian(scene) => {
                    visual_items.extend(super::gaussian_3d::items(scene, context));
                }
                shrimply_project::project::VideoItemContent::Text(_)
                | shrimply_project::project::VideoItemContent::Shape(_)
                | shrimply_project::project::VideoItemContent::Media
                | shrimply_project::project::VideoItemContent::Image
                | shrimply_project::project::VideoItemContent::Gif
                | shrimply_project::project::VideoItemContent::Svg
                | shrimply_project::project::VideoItemContent::Pdf(_)
                | shrimply_project::project::VideoItemContent::Manim(_)
                | shrimply_project::project::VideoItemContent::Blender(_)
                | shrimply_project::project::VideoItemContent::LayeredImage(_)
                | shrimply_project::project::VideoItemContent::FoldedSequence(_) => {
                    if !self.is_static_visual_media() {
                        playback_items.push(playback::speed_item(self));
                    }
                    if matches!(
                        self.content,
                        shrimply_project::project::VideoItemContent::Svg
                    ) {
                        visual_items.push(
                            DefaultInspectorItem::new(
                                "svg-colors",
                                "SVG Colors",
                                SvgColors(Some(self.clone())),
                                |value, context| {
                                    value.0.as_ref().map_or_else(Vec::new, |item| {
                                        svg_color_controls(item, context)
                                    })
                                },
                                |context, _: SvgColors| {
                                    apply_video_reset(context, "reset-svg-colors", |item| {
                                        item.svg_color_overrides.clear()
                                    });
                                },
                            )
                            .boxed(),
                        );
                    }
                    if let shrimply_project::project::VideoItemContent::LayeredImage(image) =
                        &self.content
                    {
                        visual_items.push(
                        DefaultInspectorItem::new(
                            "layered-image-layers",
                            "Layers",
                            LayeredImageLayers {
                                file: self.file.path().to_path_buf(),
                                layers: image.layers.clone(),
                            },
                            layered_image_controls,
                            |context, _: LayeredImageLayers| {
                                apply_video_reset(context, "reset-layered-image-layers", |item| {
                                    if let shrimply_project::project::VideoItemContent::LayeredImage(image) =
                                        &mut item.content
                                    {
                                        image.layers.clear();
                                    }
                                });
                            },
                        )
                        .boxed(),
                    );
                    }
                }
            }
        }
        if !matches!(
            self.content,
            VideoItemContent::Media | VideoItemContent::Gif | VideoItemContent::Background(_)
        ) {
            playback_items.push(playback::frame_rate_item(self));
        }
        match self.source_visual_kind() {
            shrimply_video_modifiers::VisualKind::Vector => visual_items.push(
                DefaultInspectorItem::new(
                    "skia-drawing",
                    "Skia drawing",
                    self.skia_drawing_strategy,
                    skia_drawing_controls,
                    |context, value: SkiaDrawingStrategy| {
                        apply_video_reset(context, "reset-skia-drawing", move |item| {
                            item.skia_drawing_strategy = value
                        });
                    },
                )
                .boxed(),
            ),
            shrimply_video_modifiers::VisualKind::Raster
            | shrimply_video_modifiers::VisualKind::Manim
            | shrimply_video_modifiers::VisualKind::Background
            | shrimply_video_modifiers::VisualKind::Scene3d => {}
        }
        if !matches!(
            self.content,
            VideoItemContent::Obj(_)
                | VideoItemContent::Gaussian(_)
                | VideoItemContent::Background(_)
        ) {
            playback_items.push(playback::motion_blur_item(self));
        }
        if !matches!(self.content, VideoItemContent::Background(_)) {
            playback_items.push(playback::repeat_item(self));
        }
        let mut compositing_item = DefaultInspectorItem::new(
            "compositing",
            "Compositing",
            CompositingCard {
                compositing: self.compositing.clone(),
                visibility: self.visibility.clone(),
                sample_method: self.sample_method.clone(),
                show_upsampling: self.source_visual_kind() == VisualKind::Raster,
            },
            compositing_controls,
            |context, value: CompositingCard| {
                apply_video_reset(context, "reset-video-compositing", move |item| {
                    item.compositing = value.compositing;
                    item.visibility = value.visibility;
                    item.sample_method = value.sample_method;
                });
            },
        );
        if self.modifier_output_kind().ok() == Some(VisualKind::Raster) {
            compositing_item = compositing_item.button_toggle(crate::alpha_mask::button_toggle(
                VisualAlphaMaskTarget::Compositing,
                context,
            ));
        }
        visual_items.push(compositing_item.boxed());
        if !matches!(
            self.content,
            shrimply_project::project::VideoItemContent::Obj(_)
                | shrimply_project::project::VideoItemContent::Gaussian(_)
                | shrimply_project::project::VideoItemContent::Manim(_)
                | shrimply_project::project::VideoItemContent::Background(_)
        ) {
            visual_items.push(
                DefaultInspectorItem::new(
                    "transform",
                    "Transform",
                    MediaTransform(self.transform.clone()),
                    |value, context| super::transform::controls(&value.0, context),
                    |context, value: MediaTransform| {
                        apply_video_reset(context, "reset-transform", move |item| {
                            item.transform = value.0
                        });
                    },
                )
                .default_with(|context| {
                    let project = context.project.borrow();
                    let transform = context
                        .selected_item
                        .clone()
                        .and_then(|key| {
                            let item = project.video_item(&key)?;
                            Some(item.natural_transform(project.canvas_size))
                        })
                        .unwrap_or_else(|| MediaTransform::default().0);
                    MediaTransform(transform)
                })
                .boxed(),
            );
        }
        visual_items.extend(super::modifiers::items(self, context));
        list::render_categories(
            vec![
                list::InspectorCategory {
                    key: "visual",
                    label: "Visual",
                    icon: "blend-tool-symbolic",
                    items: visual_items,
                },
                list::InspectorCategory {
                    key: "playback",
                    label: "Playback",
                    icon: "playback-speed-symbolic",
                    items: playback_items,
                },
                list::InspectorCategory {
                    key: "info",
                    label: "Info",
                    icon: "info-outline-symbolic",
                    items: info_items,
                },
            ],
            context,
        )
    }
}

fn video_stabilization_controls(
    item: &Option<VideoItem>,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    if let Some(item) = item {
        for row in video_stabilization_rows(item, context) {
            section.add_wide_control(&row);
        }
    }
    vec![section.into_widget()]
}

fn reset_video_stabilization(context: &InspectorContext, _: Option<VideoItem>) {
    apply_video_reset(
        context,
        "reset-video-stabilization",
        |item: &mut VideoItem| {
            shrimply_video_cuda::video_stabilization::cancel(item);
            item.stabilize_video = false;
            item.stabilization_method = Default::default();
            item.stabilization_crop_ratio =
                shrimply_project::project::default_video_stabilization_crop_ratio();
            item.stabilization_first_derivative_weight =
                shrimply_project::project::default_video_stabilization_first_derivative_weight();
            item.stabilization_second_derivative_weight =
                shrimply_project::project::default_video_stabilization_second_derivative_weight();
            item.stabilization_third_derivative_weight =
                shrimply_project::project::default_video_stabilization_third_derivative_weight();
            item.mesh_flow_rows = shrimply_project::project::default_mesh_flow_rows();
            item.mesh_flow_columns = shrimply_project::project::default_mesh_flow_columns();
            item.mesh_flow_smoothing_radius =
                shrimply_project::project::default_mesh_flow_smoothing_radius();
            item.mesh_flow_iterations = shrimply_project::project::default_mesh_flow_iterations();
            item.mesh_flow_adaptive_weights = Default::default();
        },
    );
}

fn video_stabilization_rows(item: &VideoItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    let unavailable = item.alpha_mask_video.is_some();
    let method = item.stabilization_method();
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let method_key = context.selected_item.clone();
    let refresh = context.refresh.clone();
    let method_control = dropdown(
        method,
        [
            (VideoStabilizationMethod::Off, "Off"),
            (VideoStabilizationMethod::L1, "L1"),
            (VideoStabilizationMethod::MeshFlow, "MeshFlow"),
        ],
        move |method| {
            let Some(key) = method_key.clone() else {
                return;
            };
            update_video_item(
                &project,
                &player_state,
                key,
                "video-stabilization-method",
                move |item| {
                    if item.stabilization_method() == method {
                        return false;
                    }
                    shrimply_video_cuda::video_stabilization::cancel(item);
                    item.stabilize_video = !matches!(method, VideoStabilizationMethod::Off);
                    item.stabilization_method = method;
                    if item.stabilize_video {
                        shrimply_video_cuda::video_stabilization::request(item);
                    }
                    true
                },
            );
            refresh();
        },
    );
    method_control.set_sensitive(!unavailable && context.selected_item.is_some());
    let method_status = gtk::Label::builder()
        .label(stabilization_status(item))
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    method_status.set_visible(!method_status.label().is_empty());
    let spinner = adw::Spinner::new();
    spinner.set_size_request(18, 18);
    spinner.set_visible(
        item.stabilize_video && shrimply_video_cuda::video_stabilization::is_generating(item),
    );
    let method_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    method_controls.append(&method_control);
    method_controls.append(&spinner);
    let method_row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    method_row.append(&control_row("Method", &method_controls));
    method_row.append(&method_status);

    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    let crop = adw::SpinRow::with_range(10.0, 100.0, 1.0);
    crop.set_title(tr!("Crop ratio").as_ref());
    crop.set_subtitle(tr!("Visible source area after stabilization").as_ref());
    crop.set_value(f64::from(item.stabilization_crop_ratio) * 100.0);
    crop.set_digits(0);
    crop.connect_value_notify(move |crop| {
        let Some(key) = key.clone() else {
            return;
        };
        let value = (crop.value() / 100.0).clamp(0.1, 1.0) as f32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "video-stabilization-crop-ratio",
            move |item| {
                if item.stabilization_crop_ratio == value {
                    return false;
                }
                item.stabilization_crop_ratio = value;
                if item.stabilize_video {
                    shrimply_video_cuda::video_stabilization::request(item);
                }
                true
            },
        );
    });
    crop.set_sensitive(!unavailable);

    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    let first = adw::SpinRow::with_range(0.0, 1_000.0, 1.0);
    first.set_title(tr!("Static-camera weight").as_ref());
    first.set_subtitle(tr!("Preference for frames with no camera motion").as_ref());
    first.set_value(f64::from(item.stabilization_first_derivative_weight));
    first.set_digits(1);
    first.connect_value_notify(move |first| {
        let Some(key) = key.clone() else {
            return;
        };
        let value = first.value().clamp(0.0, 1_000.0) as f32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "video-stabilization-first-derivative-weight",
            move |item| {
                if item.stabilization_first_derivative_weight == value {
                    return false;
                }
                item.stabilization_first_derivative_weight = value;
                if item.stabilize_video {
                    shrimply_video_cuda::video_stabilization::request(item);
                }
                true
            },
        );
    });
    first.set_sensitive(!unavailable);

    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    let second = adw::SpinRow::with_range(0.0, 1_000.0, 1.0);
    second.set_title(tr!("Constant-motion weight").as_ref());
    second.set_subtitle(tr!("Preference for a steady camera velocity").as_ref());
    second.set_value(f64::from(item.stabilization_second_derivative_weight));
    second.set_digits(1);
    second.connect_value_notify(move |second| {
        let Some(key) = key.clone() else {
            return;
        };
        let value = second.value().clamp(0.0, 1_000.0) as f32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "video-stabilization-second-derivative-weight",
            move |item| {
                if item.stabilization_second_derivative_weight == value {
                    return false;
                }
                item.stabilization_second_derivative_weight = value;
                if item.stabilize_video {
                    shrimply_video_cuda::video_stabilization::request(item);
                }
                true
            },
        );
    });
    second.set_sensitive(!unavailable);

    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    let third = adw::SpinRow::with_range(0.0, 1_000.0, 1.0);
    third.set_title(tr!("Constant-acceleration weight").as_ref());
    third.set_subtitle(tr!("Preference for smoothly changing camera motion").as_ref());
    third.set_value(f64::from(item.stabilization_third_derivative_weight));
    third.set_digits(1);
    third.connect_value_notify(move |third| {
        let Some(key) = key.clone() else {
            return;
        };
        let value = third.value().clamp(0.0, 1_000.0) as f32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "video-stabilization-third-derivative-weight",
            move |item| {
                if item.stabilization_third_derivative_weight == value {
                    return false;
                }
                item.stabilization_third_derivative_weight = value;
                if item.stabilize_video {
                    shrimply_video_cuda::video_stabilization::request(item);
                }
                true
            },
        );
    });
    third.set_sensitive(!unavailable);

    let cache_row = adw::ActionRow::builder()
        .title(tr!("Stabilization cache").as_ref())
        .subtitle(tr!("Discard and reanalyze the current source-time chunk").as_ref())
        .build();
    let generating = shrimply_video_cuda::video_stabilization::is_generating(item);
    let rebuild = gtk::Button::builder()
        .label(tr!(if generating { "Cancel" } else { "Rebuild" }).as_ref())
        .sensitive(item.stabilize_video && !unavailable)
        .valign(gtk::Align::Center)
        .build();
    cache_row.add_suffix(&rebuild);

    if let Some(key) = context.selected_item.clone() {
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let rebuild_key = key.clone();
        rebuild.connect_clicked(move |_| {
            let item = project.borrow().video_item(&rebuild_key).cloned();
            if let Some(item) = item {
                if shrimply_video_cuda::video_stabilization::is_generating(&item) {
                    shrimply_video_cuda::video_stabilization::cancel(&item);
                    return;
                }
                let timeline_position = player_state::current_time(&player_state);
                let source_position =
                    shrimply_project::project::video_source_time_at(&item, timeline_position)
                        .unwrap_or(item.time_offset);
                shrimply_video_cuda::video_stabilization::rebuild(&item, source_position);
            }
        });

        let method_row = method_row.downgrade();
        let method_status = method_status.downgrade();
        let spinner = spinner.downgrade();
        let rebuild = rebuild.downgrade();
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let status_key = key;
        let mut was_generating = shrimply_video_cuda::video_stabilization::is_generating(item);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(_method_row) = method_row.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(method_status) = method_status.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(spinner) = spinner.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let item = project.borrow().video_item(&status_key).cloned();
            let Some(item) = item else {
                return glib::ControlFlow::Break;
            };
            spinner.set_visible(
                item.stabilize_video
                    && shrimply_video_cuda::video_stabilization::is_generating(&item),
            );
            let generating = shrimply_video_cuda::video_stabilization::is_generating(&item);
            if was_generating && !generating {
                player_state::refresh_project(
                    &player_state,
                    ProjectChange {
                        video: true,
                        inspector: true,
                        ..ProjectChange::default()
                    },
                );
            }
            was_generating = generating;
            let status = stabilization_status(&item);
            method_status.set_label(tr!(status).as_ref());
            method_status.set_visible(!status.is_empty());
            if let Some(rebuild) = rebuild.upgrade() {
                rebuild.set_sensitive(item.stabilize_video && item.alpha_mask_video.is_none());
                rebuild.set_label(tr!(if generating { "Cancel" } else { "Rebuild" }).as_ref());
            }
            glib::ControlFlow::Continue
        });
    } else {
        crop.set_sensitive(false);
        first.set_sensitive(false);
        second.set_sensitive(false);
        third.set_sensitive(false);
        rebuild.set_sensitive(false);
    }

    let mut rows = vec![method_row.upcast()];
    if !matches!(method, VideoStabilizationMethod::Off) {
        rows.push(crop.upcast());
    }
    if matches!(method, VideoStabilizationMethod::L1) {
        rows.extend([first.upcast(), second.upcast(), third.upcast()]);
    } else if matches!(method, VideoStabilizationMethod::MeshFlow) {
        rows.extend(mesh_flow_settings_rows(item, context, unavailable));
    }
    if !matches!(method, VideoStabilizationMethod::Off) {
        rows.push(cache_row.upcast());
    }
    rows
}

fn mesh_flow_settings_rows(
    item: &VideoItem,
    context: &InspectorContext,
    unavailable: bool,
) -> Vec<gtk::Widget> {
    let rows = adw::SpinRow::with_range(2.0, 32.0, 1.0);
    rows.set_title(tr!("Mesh rows").as_ref());
    rows.set_subtitle(tr!("Number of independently moving cell rows").as_ref());
    rows.set_value(f64::from(item.mesh_flow_rows));
    rows.set_digits(0);
    rows.set_sensitive(!unavailable);
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    rows.connect_value_notify(move |row| {
        let Some(key) = key.clone() else { return };
        let value = row.value().round().clamp(2.0, 32.0) as u32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "mesh-flow-rows",
            move |item| {
                if item.mesh_flow_rows == value {
                    return false;
                }
                item.mesh_flow_rows = value;
                shrimply_video_cuda::video_stabilization::request(item);
                true
            },
        );
    });

    let columns = adw::SpinRow::with_range(2.0, 32.0, 1.0);
    columns.set_title(tr!("Mesh columns").as_ref());
    columns.set_subtitle(tr!("Number of independently moving cell columns").as_ref());
    columns.set_value(f64::from(item.mesh_flow_columns));
    columns.set_digits(0);
    columns.set_sensitive(!unavailable);
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    columns.connect_value_notify(move |row| {
        let Some(key) = key.clone() else { return };
        let value = row.value().round().clamp(2.0, 32.0) as u32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "mesh-flow-columns",
            move |item| {
                if item.mesh_flow_columns == value {
                    return false;
                }
                item.mesh_flow_columns = value;
                shrimply_video_cuda::video_stabilization::request(item);
                true
            },
        );
    });

    let radius = adw::SpinRow::with_range(1.0, 120.0, 1.0);
    radius.set_title(tr!("Smoothing radius").as_ref());
    radius.set_subtitle(tr!("Neighboring frames considered on each side").as_ref());
    radius.set_value(f64::from(item.mesh_flow_smoothing_radius));
    radius.set_digits(0);
    radius.set_sensitive(!unavailable);
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    radius.connect_value_notify(move |row| {
        let Some(key) = key.clone() else { return };
        let value = row.value().round().clamp(1.0, 120.0) as u32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "mesh-flow-smoothing-radius",
            move |item| {
                if item.mesh_flow_smoothing_radius == value {
                    return false;
                }
                item.mesh_flow_smoothing_radius = value;
                shrimply_video_cuda::video_stabilization::request(item);
                true
            },
        );
    });

    let iterations = adw::SpinRow::with_range(1.0, 500.0, 1.0);
    iterations.set_title(tr!("Optimization iterations").as_ref());
    iterations.set_subtitle(tr!("Jacobi passes used to minimize the MeshFlow energy").as_ref());
    iterations.set_value(f64::from(item.mesh_flow_iterations));
    iterations.set_digits(0);
    iterations.set_sensitive(!unavailable);
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    iterations.connect_value_notify(move |row| {
        let Some(key) = key.clone() else { return };
        let value = row.value().round().clamp(1.0, 500.0) as u32;
        update_video_item(
            &project,
            &player_state,
            key.clone(),
            "mesh-flow-iterations",
            move |item| {
                if item.mesh_flow_iterations == value {
                    return false;
                }
                item.mesh_flow_iterations = value;
                shrimply_video_cuda::video_stabilization::request(item);
                true
            },
        );
    });

    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let key = context.selected_item.clone();
    let adaptive_control = dropdown(
        item.mesh_flow_adaptive_weights,
        [
            (MeshFlowAdaptiveWeights::Original, "Original"),
            (MeshFlowAdaptiveWeights::Flipped, "Flipped"),
            (MeshFlowAdaptiveWeights::ConstantHigh, "Constant high"),
            (MeshFlowAdaptiveWeights::ConstantLow, "Constant low"),
        ],
        move |value| {
            let Some(key) = key.clone() else { return };
            update_video_item(
                &project,
                &player_state,
                key,
                "mesh-flow-adaptive-weights",
                move |item| {
                    if item.mesh_flow_adaptive_weights == value {
                        return false;
                    }
                    item.mesh_flow_adaptive_weights = value;
                    shrimply_video_cuda::video_stabilization::request(item);
                    true
                },
            );
        },
    );
    adaptive_control.set_sensitive(!unavailable && context.selected_item.is_some());
    adaptive_control.set_tooltip_i18n("Motion-dependent temporal smoothing model");
    let adaptive = control_row("Adaptive weights", &adaptive_control);

    vec![
        rows.upcast(),
        columns.upcast(),
        radius.upcast(),
        iterations.upcast(),
        adaptive.upcast(),
    ]
}

fn stabilization_status(item: &VideoItem) -> &'static str {
    if item.alpha_mask_video.is_some() {
        "Unavailable while an alpha-mask stream is selected"
    } else if !item.stabilize_video {
        ""
    } else if shrimply_video_cuda::video_stabilization::is_generating(item) {
        "Analyzing source motion…"
    } else if shrimply_video_cuda::video_stabilization::has_failed(item) {
        "Analysis failed; use Rebuild to retry"
    } else if shrimply_video_cuda::video_stabilization::is_ready(item) {
        "Using the reusable chunked analysis cache"
    } else {
        "Analysis starts as source-time chunks are viewed"
    }
}

fn video_generation_editor(
    id: uuid::Uuid,
    settings: &shrimply_video_generation::VideoGenerationSettings,
    context: &InspectorContext,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let changed_project = context.project.clone();
    let changed_key = key.clone();
    let commit_project = context.project.clone();
    let generated_project = context.project.clone();
    let generated_player = context.player_state.clone();
    let has_output = context
        .project
        .borrow()
        .video_item(&key)
        .is_some_and(|item| !item.file.as_os_str().is_empty());
    shrimply_video_generation_gtk::editor(
        id,
        context.preferences.clone(),
        settings,
        has_output,
        move |settings| {
            let mut project = changed_project.borrow_mut();
            let Some(item) = project.video_item_mut(&changed_key) else {
                return;
            };
            let Some(current) = &mut item.video_generation else {
                return;
            };
            **current = settings;
        },
        move || {
            shrimply_project::project::commit_edit(
                &commit_project.borrow(),
                "edit-video-generation",
            );
        },
        move |path, result| {
            let source_duration = Time::from_fraction(
                fraction_numerator(result.duration),
                fraction_denominator(result.duration),
            );
            let mut project = generated_project.borrow_mut();
            let Some(item) = project.video_item(&key) else {
                drop(project);
                let _ = std::fs::remove_file(path);
                return;
            };
            if item.video_generation.is_none()
                || result.video_streams == 0
                || result.audio_streams == 0
            {
                drop(project);
                let _ = std::fs::remove_file(path);
                return;
            }
            let start = item.start;
            let item_id = item.id;
            let previous_file = item.file.path().to_path_buf();
            let had_output = !previous_file.as_os_str().is_empty();
            let previous_group = item.group_id;
            let next_video_start = match project.track(&key.track()) {
                Some(shrimply_project::project::TrackRef::Video(track)) => track
                    .items
                    .iter()
                    .filter_map(|candidate| {
                        (candidate.id != item_id && candidate.start > start)
                            .then_some(candidate.start)
                    })
                    .min(),
                _ => None,
            };
            let paired_audio = previous_group.and_then(|group_id| {
                if previous_file.as_os_str().is_empty() {
                    return None;
                }
                project
                    .audio_tracks
                    .iter()
                    .enumerate()
                    .find_map(|(track_index, track)| {
                        track.items.iter().find_map(|candidate| {
                            (candidate.group_id == Some(group_id)
                                && matches!(candidate.source, AudioSource::Media)
                                && candidate.file.path() == previous_file)
                                .then_some((track_index, candidate.id))
                        })
                    })
            });
            let mut candidate_tracks = paired_audio
                .map(|(track_index, _)| track_index)
                .into_iter()
                .chain(0..project.audio_tracks.len())
                .collect::<Vec<_>>();
            candidate_tracks.dedup();
            let audio_track = candidate_tracks.into_iter().find(|track_index| {
                project.audio_tracks[*track_index]
                    .items
                    .iter()
                    .all(|candidate| {
                        paired_audio.is_some_and(|(_, id)| candidate.id == id)
                            || candidate.end <= start
                            || candidate.start > start
                    })
            });
            let audio_track = audio_track.unwrap_or_else(|| {
                project.audio_tracks.push(AudioTrack::default());
                project.audio_tracks.len() - 1
            });
            let next_audio_start = project.audio_tracks[audio_track]
                .items
                .iter()
                .filter_map(|candidate| {
                    (paired_audio.is_none_or(|(_, id)| candidate.id != id)
                        && candidate.start > start)
                        .then_some(candidate.start)
                })
                .min();
            let mut end = start
                .saturating_add(source_duration)
                .snapped(project.frame_step());
            if let Some(next) = next_video_start {
                end = end.min(next);
            }
            if let Some(next) = next_audio_start {
                end = end.min(next);
            }
            if end <= start {
                drop(project);
                let _ = std::fs::remove_file(path);
                return;
            }
            let group_id = previous_group.unwrap_or_else(|| next_video_generation_group(&project));
            let mut audio = paired_audio
                .and_then(|(track_index, id)| {
                    let track = &mut project.audio_tracks[track_index];
                    track
                        .items
                        .iter()
                        .position(|candidate| candidate.id == id)
                        .map(|index| track.items.remove(index))
                })
                .unwrap_or_else(|| {
                    AudioItem::builder(start, end)
                        .group_id(Some(group_id))
                        .build()
                });
            audio.start = start;
            audio.end = end;
            audio.time_offset = Time::ZERO;
            audio.source_duration = source_duration;
            audio.playback_speed = shrimply_project::project::default_playback_speed();
            audio.repeat_strategy = shrimply_project::project::RepeatStrategy::Hold;
            audio.group_id = Some(group_id);
            audio.source = AudioSource::Media;
            audio.track_id = 0;
            audio.file = path.clone().into();
            project.audio_tracks[audio_track].items.push(audio);
            project.audio_tracks[audio_track]
                .items
                .sort_by_key(|candidate| candidate.start);

            let canvas_size = project.canvas_size;
            let transform = Transform::natural_size(canvas_size, result.width, result.height);
            let Some(item) = project.video_item_mut(&key) else {
                drop(project);
                let _ = std::fs::remove_file(path);
                return;
            };
            item.end = end;
            item.time_offset = Time::ZERO;
            item.source_duration = source_duration;
            item.playback_speed = shrimply_project::project::default_playback_speed();
            item.playback_fps = result.frame_rate;
            item.repeat_strategy = shrimply_project::project::RepeatStrategy::Hold;
            if !had_output {
                item.transform = transform.clone();
                item.default_transform = Some(transform);
            }
            item.source_width = result.width;
            item.source_height = result.height;
            item.group_id = Some(group_id);
            item.content = VideoItemContent::Media;
            item.track_id = 0;
            item.file = path.into();
            let duration = project.duration();
            shrimply_project::project::commit_edit(&project, "generate-video");
            drop(project);
            player_state::refresh_project(
                &generated_player,
                ProjectChange {
                    duration: Some(duration),
                    video: true,
                    audio: true,
                    audio_beats: true,
                    audio_waveforms: true,
                    inspector: true,
                    ..ProjectChange::default()
                },
            );
        },
    )
}

fn next_video_generation_group(project: &Project) -> u64 {
    project
        .video_tracks
        .iter()
        .flat_map(|track| track.items.iter().filter_map(|item| item.group_id))
        .chain(
            project
                .audio_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
        )
        .chain(
            project
                .caption_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
        )
        .chain(project.folded_sequences.iter().flat_map(|sequence| {
            sequence
                .video_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id))
        }))
        .chain(project.folded_sequences.iter().flat_map(|sequence| {
            sequence
                .audio_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id))
        }))
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn video_stream_rows(item: &VideoItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    let stream_count = super::info::video_stream_count(&item.file) as u32;
    if stream_count < 2 {
        return Vec::new();
    }
    let labels = (0..stream_count)
        .map(|stream| {
            shrimply_gtk_components::i18n::text_args(
                "Video stream %{number}",
                &[("number", (stream + 1).to_string())],
            )
        })
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let stream = adw::ComboRow::builder()
        .title(tr!("Video Stream").as_ref())
        .model(&gtk::StringList::new(&label_refs))
        .selected(item.track_id.min(stream_count - 1))
        .build();

    let mut alpha_options = vec![None];
    alpha_options.extend(
        (0..stream_count)
            .filter(|candidate| *candidate != item.track_id)
            .map(Some),
    );
    let alpha_labels = alpha_options
        .iter()
        .map(|stream| {
            stream.map_or_else(
                || tr!("None").into_owned(),
                |stream| {
                    shrimply_gtk_components::i18n::text_args(
                        "Video stream %{number}",
                        &[("number", (stream + 1).to_string())],
                    )
                },
            )
        })
        .collect::<Vec<_>>();
    let alpha_label_refs = alpha_labels.iter().map(String::as_str).collect::<Vec<_>>();
    let alpha = adw::ComboRow::builder()
        .title(tr!("Alpha Mask Stream").as_ref())
        .model(&gtk::StringList::new(&alpha_label_refs))
        .selected(
            alpha_options
                .iter()
                .position(|candidate| *candidate == item.alpha_mask_video)
                .unwrap_or_default() as u32,
        )
        .build();

    if let Some(key) = context.selected_item.clone() {
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let refresh = context.refresh.clone();
        let stream_key = key.clone();
        stream.connect_selected_notify(move |stream| {
            let track_id = stream.selected();
            update_video_item(
                &project,
                &player_state,
                stream_key.clone(),
                "video-stream",
                move |item| {
                    if item.track_id == track_id {
                        return false;
                    }
                    item.track_id = track_id;
                    if item.alpha_mask_video == Some(track_id) {
                        item.alpha_mask_video = None;
                    }
                    true
                },
            );
            refresh();
        });

        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let alpha_key = key;
        alpha.connect_selected_notify(move |alpha| {
            let alpha_mask_video = alpha_options
                .get(alpha.selected() as usize)
                .copied()
                .flatten();
            update_video_item(
                &project,
                &player_state,
                alpha_key.clone(),
                "video-alpha-mask-stream",
                move |item| {
                    if item.alpha_mask_video == alpha_mask_video {
                        return false;
                    }
                    item.alpha_mask_video = alpha_mask_video;
                    true
                },
            );
        });
    } else {
        stream.set_sensitive(false);
        alpha.set_sensitive(false);
    }

    vec![stream.upcast(), alpha.upcast()]
}

#[derive(Default)]
struct LayeredImageLayers {
    file: PathBuf,
    layers: Vec<LayerVisibility>,
}

fn layered_image_controls(
    value: &LayeredImageLayers,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let Ok(image) = shrimply_video_cuda::load_layered_image(&value.file) else {
        return Vec::new();
    };
    let section = InspectorSection::controls();
    for entry in &image.entries {
        let stored = value.layers.iter().find(|stored| stored.path == entry.path);
        let timeline_value = stored
            .and_then(|entry| entry.visibility.clone())
            .unwrap_or_else(|| TimelineValue::<TimelineBool>::new_const(entry.visible.into()));
        section.add_wide_control(&bool_control(
            &format!(
                "{}{}{}",
                "  ".repeat(entry.depth),
                if entry.group { "▾ " } else { "" },
                layer_display_name(&entry.name),
            ),
            &timeline_value,
            entry.visible,
            context,
            BoolTarget::Layer {
                id: stored.map_or_else(uuid::Uuid::new_v4, |entry| entry.id),
                path: entry.path.clone(),
            },
        ));
    }
    vec![section.into_widget()]
}

fn layer_display_name(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '\0')
        .collect()
}

fn sample_method_control(
    value: &TimelineValue<VideoSampleMethod>,
    context: &InspectorContext,
) -> gtk::Widget {
    step_control(
        "Upsampling",
        value,
        context,
        StepTarget::new(
            video_sample_method,
            video_sample_method_mut,
            "video-upsampling",
            ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        ),
    )
}

fn video_sample_method(
    project: &Project,
    key: SelectedItem,
) -> Option<&TimelineValue<VideoSampleMethod>> {
    project.video_item(&key).map(|item| &item.sample_method)
}

fn video_sample_method_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<VideoSampleMethod>> {
    project
        .video_item_mut(&key)
        .map(|item| &mut item.sample_method)
}

fn skia_drawing_controls(
    value: &SkiaDrawingStrategy,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    let Some(key) = context.selected_item.clone() else {
        return vec![section.into_widget()];
    };
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    let dropdown = selector(
        "Strategy",
        *value,
        [
            (SkiaDrawingStrategy::Immediate, "Immediate"),
            (SkiaDrawingStrategy::Picture, "Picture"),
        ],
        move |drawing_strategy| {
            update_video_item(
                &project,
                &player_state,
                key.clone(),
                "skia-drawing-strategy",
                |item| {
                    if item.skia_drawing_strategy == drawing_strategy {
                        return false;
                    }
                    item.skia_drawing_strategy = drawing_strategy;
                    true
                },
            );
        },
    );
    section.add_wide_control(&dropdown);
    vec![section.into_widget()]
}

fn compositing_controls(value: &CompositingCard, context: &InspectorContext) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    section.add_wide_control(&bool_control(
        "Visible",
        &value.visibility,
        true,
        context,
        BoolTarget::ItemVisibility,
    ));
    if value.show_upsampling {
        section.add_wide_control(&sample_method_control(&value.sample_method, context));
    }
    section.add_wide_control(&scalar_control(
        "Opacity",
        &value.compositing.opacity,
        context,
        ScalarTarget {
            access: ScalarAccess::Item {
                get: compositing_opacity,
                get_mut: compositing_opacity_mut,
            },
            scope_id: Some(value.compositing.opacity.id),
            local_time: visual_local_time,
            duration: visual_duration,
            refresh: ProjectChange {
                video: true,
                ..Default::default()
            },
            commit_name: "visual-compositing-opacity",
        },
        ScalarSpec {
            drag_step: 1.0,
            digits: 0,
            integer: false,
            width_chars: 9,
            minimum: Some(0.0),
            maximum: Some(100.0),
            unit_name: Some("%"),
            rotating_icon: None,
            display: |value| f64::from(value) * 100.0,
            store: |value| (value / 100.0) as f32,
            clamp: crate::timeline_value::scalar::ScalarClamp::Function(|value| {
                value.clamp(0.0, 1.0)
            }),
        },
    ));
    section.add_wide_control(&blend_mode_control(&value.compositing.blend_mode, context));
    let mut controls = vec![section.into_widget()];
    let raster_output = context.selected_item.as_ref().is_some_and(|key| {
        context
            .project
            .borrow()
            .video_item(key)
            .and_then(|item| item.modifier_output_kind().ok())
            == Some(VisualKind::Raster)
    });
    if raster_output {
        controls.push(crate::alpha_mask::widget(
            VisualAlphaMaskTarget::Compositing,
            context,
        ));
    }
    controls
}

#[derive(Default)]
struct CompositingCard {
    compositing: VisualCompositing,
    visibility: TimelineValue<TimelineBool>,
    sample_method: TimelineValue<VideoSampleMethod>,
    show_upsampling: bool,
}

fn compositing_opacity(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    Some(&project.video_item(&key)?.compositing.opacity)
}

fn compositing_opacity_mut(
    project: &mut Project,
    key: SelectedItem,
) -> Option<&mut TimelineValue<f32>> {
    Some(&mut project.video_item_mut(&key)?.compositing.opacity)
}

pub(super) fn visual_local_time(project: &Project, key: SelectedItem, time: Time) -> Option<Time> {
    let time = visual_sequence_time(project, &key, time)?;
    let item = project.video_item(&key)?;
    shrimply_project::project::generated_item_time(item, time)
}

pub(super) fn visual_sequence_time(
    project: &Project,
    key: &SelectedItem,
    time: Time,
) -> Option<Time> {
    project.timeline_time_to_sequence(&key.track(), time)
}

pub(super) fn visual_duration(project: &Project, key: SelectedItem) -> Option<Time> {
    let (start, end) = visual_visible_area(project, key)?;
    Some(end.saturating_sub(start))
}

pub(super) fn visual_visible_area(project: &Project, key: SelectedItem) -> Option<(Time, Time)> {
    let (start, end) = project.projected_item_times(&key)?;
    let track = key.track();
    let item = project.video_item(&key)?;
    let mut start = project
        .timeline_time_to_sequence(&track, start)?
        .signed_sub(item.start)
        .saturating_add(item.animation_time_offset);
    let mut end = project
        .timeline_time_to_sequence(&track, end)?
        .signed_sub(item.start)
        .saturating_add(item.animation_time_offset);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    Some((start, end))
}

fn svg_color_controls(item: &VideoItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    let Ok(snapshot) = item.file.snapshot() else {
        return Vec::new();
    };
    let out = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let spinner = gtk::Spinner::new();
    spinner.start();
    out.append(&spinner);

    let (_, request) = svg_color_pipeline().request(snapshot.clone());
    let subscription = Rc::new(RefCell::<Option<UiSubscription>>::new(None));
    let keep_subscription = subscription.clone();
    let item = item.clone();
    let context = context.clone();
    let listener_scope = Rc::downgrade(&context.listener_scope);
    let delivery = deliver(
        out.downgrade(),
        request,
        SVG_COLOR_DELIVERY_INTERVAL,
        move |out, event| {
            if keep_subscription.borrow().is_none() || listener_scope.upgrade().is_none() {
                return;
            }
            if !snapshot.is_current() {
                (context.refresh)();
                return;
            }
            while let Some(child) = out.first_child() {
                out.remove(&child);
            }
            match event {
                Event::Finished(colors) if !colors.is_empty() => {
                    out.append(&svg_color_section(&colors, &item, &context));
                }
                Event::Failed(error) => {
                    tracing::warn!(path = %snapshot.path().display(), %error, "Could not load SVG colors");
                }
                Event::Finished(_) | Event::Cancelled | Event::Progress(_) => {}
            }
        },
    );
    *subscription.borrow_mut() = Some(delivery);
    vec![out.upcast()]
}

fn svg_color_section(
    colors: &[svg_color::SvgPaintColor],
    item: &VideoItem,
    context: &InspectorContext,
) -> gtk::Widget {
    let section = InspectorSection::controls();
    let mut fill_count = 0;
    let mut stroke_count = 0;
    for color in colors {
        let label = match color.kind {
            SvgPaintKind::Fill => {
                fill_count += 1;
                shrimply_gtk_components::i18n::text_args(
                    "Fill color %{number}",
                    &[("number", fill_count.to_string())],
                )
            }
            SvgPaintKind::Stroke => {
                stroke_count += 1;
                shrimply_gtk_components::i18n::text_args(
                    "Stroke color %{number}",
                    &[("number", stroke_count.to_string())],
                )
            }
        };
        section.add_control_row(
            &label,
            &svg_color_button(
                color.kind,
                color.color,
                svg_override_replacement(item, color.kind, color.color),
                context,
            ),
        );
    }
    section.into_widget()
}

struct SvgColorProcessor;

impl Processor<AssetSnapshot> for SvgColorProcessor {
    type Progress = ();
    type Output = Vec<svg_color::SvgPaintColor>;

    fn process(
        &self,
        snapshot: AssetSnapshot,
        _context: &JobContext<Self::Progress>,
    ) -> Result<Self::Output, String> {
        snapshot
            .read_to_string()
            .map(|svg| svg_color::paint_colors(&svg))
    }
}

fn svg_color_pipeline() -> &'static Pipeline<AssetSnapshot, SvgColorProcessor> {
    static PIPELINE: OnceLock<Pipeline<AssetSnapshot, SvgColorProcessor>> = OnceLock::new();
    PIPELINE.get_or_init(|| {
        Pipeline::new(SvgColorProcessor, |job| {
            std::thread::spawn(job);
        })
    })
}

fn svg_color_button(
    kind: SvgPaintKind,
    original: Color<u8>,
    value: Color<u8>,
    context: &InspectorContext,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return ColorPicker::builder(value)
            .title(tr!("SVG color").as_ref())
            .hexpand(true)
            .build();
    };
    let project = context.project.clone();
    let player_state = context.player_state.clone();
    ColorPicker::builder(value)
        .title(tr!("SVG color").as_ref())
        .hexpand(true)
        .on_change(move |color| {
            update_svg_color_override(&project, &player_state, key.clone(), kind, original, color);
        })
        .build()
}

fn svg_override_replacement(
    item: &VideoItem,
    kind: SvgPaintKind,
    original: Color<u8>,
) -> Color<u8> {
    item.svg_color_overrides
        .iter()
        .find(|override_color| override_color.kind == kind && override_color.original == original)
        .map_or(original, |override_color| override_color.replacement)
}

fn update_svg_color_override(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    kind: SvgPaintKind,
    original: Color<u8>,
    replacement: Color<u8>,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.video_item_mut(&key) else {
        return;
    };
    if !matches!(
        item.content,
        shrimply_project::project::VideoItemContent::Svg
    ) {
        return;
    }
    if let Some(index) = item
        .svg_color_overrides
        .iter_mut()
        .position(|override_color| {
            override_color.kind == kind && override_color.original == original
        })
    {
        if item.svg_color_overrides[index].replacement == replacement {
            return;
        }
        if replacement == original {
            item.svg_color_overrides.remove(index);
        } else {
            item.svg_color_overrides[index].replacement = replacement;
        }
    } else {
        if replacement == original || item.svg_color_overrides.len() >= svg_color::SVG_COLOR_LIMIT {
            return;
        }
        item.svg_color_overrides.push(SvgColorOverride {
            kind,
            original,
            replacement,
        });
    }
    shrimply_project::project::commit_coalesced_edit(&project, "svg-color-override");
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            video: true,
            ..ProjectChange::default()
        },
    );
}

fn manim_item(
    presentation: shrimply_inspector_core::manim_parameters::ManimPresentation,
) -> InspectorListItem {
    let default = presentation.clone();
    let reset = presentation.main_reset.clone();
    let actions = header_actions(presentation.main.actions.clone());
    DefaultInspectorItem::new_with_default(
        presentation.main.key,
        presentation.main.title,
        presentation,
        manim_controls,
        move |_| default.clone(),
        move |context, _| {
            let Some(item) = context.selected_item.clone() else {
                return;
            };
            if let Err(error) = context.inspector_core.reset_manim(
                &shrimply_inspector_core::InspectorTarget::Item(item),
                &reset,
            ) {
                tracing::error!(%error, "Could not reset GTK Manim scene");
            }
        },
    )
    .actions(actions)
    .boxed()
}

fn manim_controls(
    presentation: &shrimply_inspector_core::manim_parameters::ManimPresentation,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    use shrimply_inspector_core::manim_parameters as shared;

    let section = InspectorSection::controls();
    let Some(key) = context.selected_item.clone() else {
        return vec![section.into_widget()];
    };
    let target = shrimply_inspector_core::InspectorTarget::Item(key);
    let scene = presentation
        .main
        .section
        .controls
        .iter()
        .find(|control| control.path == shared::SCENE_PATH)
        .expect("shared Manim card must contain its scene selector");
    let scenes = labeled_string_selector(
        &scene.label,
        &scene.value,
        scene
            .values
            .iter()
            .zip(&scene.labels)
            .map(|(value, label)| StringChoice {
                value: value.clone(),
                label: if presentation.current_scene.is_empty() {
                    tr!(label).into_owned()
                } else {
                    label.clone()
                },
            })
            .collect(),
        {
            let controller = context.inspector_core.clone();
            let target = target.clone();
            let commit_name = scene.commit_name.clone();
            move |value| {
                if let Err(error) = controller.set_manim_scene(&target, value, &commit_name) {
                    tracing::error!(%error, "Could not select GTK Manim scene");
                }
            }
        },
    );
    scenes.set_sensitive(scene.sensitive);
    if !scene.tooltip.is_empty() {
        scenes.widget().set_tooltip_text(Some(&scene.tooltip));
    }
    section.add_wide_control(scenes.widget());

    if let Some(control) = presentation
        .main
        .section
        .controls
        .iter()
        .find(|control| shared::parameter_key(&control.path).is_some())
    {
        let key = shared::parameter_key(&control.path)
            .expect("shared anti-aliasing control must have a parameter path")
            .to_string();
        let controller = context.inspector_core.clone();
        let target = target.clone();
        let commit_name = control.commit_name.clone();
        section.add_wide_control(&selector(
            &control.label,
            control
                .value
                .parse::<i64>()
                .expect("shared anti-aliasing value must be an integer"),
            control
                .values
                .iter()
                .zip(&control.labels)
                .map(|(value, label)| {
                    (
                        value
                            .parse::<i64>()
                            .expect("shared anti-aliasing option must be an integer"),
                        label.clone(),
                    )
                }),
            move |value| {
                if let Err(error) = controller.set_manim_parameter(
                    &target,
                    &key,
                    ManimParameterValue::Integer(value),
                    &commit_name,
                ) {
                    tracing::error!(%error, "Could not change GTK Manim anti-aliasing");
                }
            },
        ));
    }

    if let Some(error) = presentation
        .main
        .section
        .controls
        .iter()
        .find(|control| control.kind == shrimply_inspector_core::ControlKind::ReadOnly)
        .map(|control| &control.value)
    {
        let error = gtk::Label::builder()
            .label(error)
            .halign(gtk::Align::Fill)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        error.add_css_class("error");
        error.add_css_class("caption");
        section.add_wide_control(&error);
    }

    let source = shrimply_project::project::Asset::from(std::path::Path::new(&presentation.source));
    let current_scene = presentation.current_scene.clone();
    let listener_scope = Rc::downgrade(&context.listener_scope);
    let controller = context.inspector_core.clone();
    if scene.sensitive {
        if scene.value != current_scene {
            let selected = scene.value.clone();
            glib::idle_add_local_once(move || {
                if listener_scope.upgrade().is_some()
                    && let Err(error) =
                        controller.set_manim_scene(&target, selected, "select-manim-scene")
                {
                    tracing::error!(%error, "Could not select the default GTK Manim scene");
                }
            });
        }
    } else if scene.tooltip.is_empty() {
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_millis(50)).await;
                if listener_scope.upgrade().is_none() {
                    return;
                }
                shared::poll_scenes();
                match shared::scenes(&source, &current_scene) {
                    shared::ManimScenes::Loading => continue,
                    shared::ManimScenes::Ready(discovery) => {
                        let control = shared::discovered_scene_control(&discovery);
                        scenes.set_options(&control.value, control.values);
                        scenes.set_sensitive(control.sensitive);
                        if discovery.changed
                            && let Err(error) = controller.set_manim_scene(
                                &target,
                                discovery.selected,
                                "select-manim-scene",
                            )
                        {
                            tracing::error!(%error, "Could not select the default GTK Manim scene");
                        }
                        return;
                    }
                    shared::ManimScenes::Failed(error) => {
                        let control = shared::failed_scene_control(&error);
                        scenes.set_options(&control.value, control.values);
                        scenes.set_sensitive(control.sensitive);
                        scenes.widget().set_tooltip_text(Some(&control.tooltip));
                        return;
                    }
                }
            }
        });
    }

    vec![section.into_widget()]
}

struct MediaTransform(Transform);

#[derive(Default)]
struct SvgColors(Option<VideoItem>);

impl Default for MediaTransform {
    fn default() -> Self {
        Self(Transform::from_resolved(ResolvedTransform::IDENTITY))
    }
}

fn blend_mode_control(
    value: &TimelineValue<LayerBlendMode>,
    context: &InspectorContext,
) -> gtk::Widget {
    crate::timeline_value::step::step_control(
        "Blend mode",
        value,
        context,
        crate::timeline_value::step::StepTarget::new(
            |project, key| Some(&project.video_item(&key)?.compositing.blend_mode),
            |project, key| Some(&mut project.video_item_mut(&key)?.compositing.blend_mode),
            "video-compositing-blend-mode",
            ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        ),
    )
}

fn update_video_item(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    commit_name: &'static str,
    update: impl FnOnce(&mut VideoItem) -> bool,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.video_item_mut(&key) else {
        return;
    };
    if !update(item) {
        return;
    }
    shrimply_project::project::commit_edit(&project, commit_name);
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            video: true,
            ..ProjectChange::default()
        },
    );
}

fn apply_video_reset(
    context: &InspectorContext,
    commit_name: &'static str,
    update: impl FnOnce(&mut VideoItem),
) {
    let Some(key) = context.selected_item.clone() else {
        return;
    };
    let mut project = context.project.borrow_mut();
    let Some(item) = project.video_item_mut(&key) else {
        return;
    };
    update(item);
    shrimply_project::project::commit_edit(&project, commit_name);
    drop(project);
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            video: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}
