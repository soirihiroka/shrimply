use core::pin::Pin;

use cxx_qt_lib::{QString, QStringList};

use crate::backend::qobject::InspectorBackend;
use crate::section::InspectorControl;

pub(crate) fn has_transform_controls(section: &crate::section::InspectorSection) -> bool {
    shrimply_inspector_core::keyframe_graph::has_transform_controls(section)
}

pub(crate) fn is_transform_path(path: &str) -> bool {
    shrimply_inspector_core::keyframe_graph::is_transform_path(path)
}

pub(crate) fn update_transform_graphs(
    document: &mut crate::list::InspectorDocument,
    live: &shrimply_inspector_core::transform::TransformLivePresentation,
) {
    for category in &mut document.categories {
        for item in &mut category.items {
            let section = match item {
                crate::item::InspectorListItem::Item(item) => &mut item.section,
                crate::item::InspectorListItem::Flat(section) => section,
            };
            shrimply_inspector_core::keyframe_graph::update_transform_graphs(section, live);
        }
    }
}

pub(crate) fn update_control_graphs(
    document: &mut crate::list::InspectorDocument,
    target: &shrimply_inspector_core::InspectorTarget,
) {
    let mut vector_source = None;
    for category in &mut document.categories {
        for item in &mut category.items {
            let section = match item {
                crate::item::InspectorListItem::Item(item) => &mut item.section,
                crate::item::InspectorListItem::Flat(section) => section,
            };
            for control in &mut section.controls {
                if control.scalar_graph.is_none() {
                    continue;
                }
                let graph = if matches!(
                    control.kind,
                    crate::section::ControlKind::LayeredVector2
                        | crate::section::ControlKind::LayeredVector3
                ) {
                    let Ok((value, runtime)) = vector_source
                        .get_or_insert_with(|| super::control_graph_source(target))
                        .as_ref()
                    else {
                        continue;
                    };
                    shrimply_inspector_core::keyframe_graph::vector_control_graph(
                        value, *runtime, control,
                    )
                    .expect("layered vector control must have a vector graph result")
                } else {
                    super::control_graph(target, control)
                };
                if let Ok(graph) = graph {
                    control.scalar_graph = graph;
                }
            }
        }
    }
}

impl InspectorBackend {
    pub fn control_graph_point_times(&self, category: i32, item: i32, control: i32) -> QStringList {
        self.control(category, item, control)
            .and_then(|control| control.scalar_graph.as_ref())
            .map(|graph| {
                graph
                    .points
                    .iter()
                    .map(|point| QString::from(crate::backend::time_text(point.time)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn control_graph_point_values(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> QStringList {
        self.control(category, item, control)
            .and_then(|control| control.scalar_graph.as_ref())
            .map(|graph| {
                graph
                    .points
                    .iter()
                    .map(|point| QString::from(point.value.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn control_graph_segments(&self, category: i32, item: i32, control: i32) -> QStringList {
        let speed = self.control(category, item, control).and_then(|control| {
            shrimply_inspector_core::InspectorGraphKind::for_control(control.kind)
        }) == Some(shrimply_inspector_core::InspectorGraphKind::Speed);
        self.control(category, item, control)
            .and_then(|control| control.scalar_graph.as_ref())
            .map(|graph| {
                graph
                    .segments
                    .iter()
                    .map(|segment| {
                        QString::from(if speed {
                            format!(
                                "{}\t{}\t{}\t{}\t{}",
                                segment.owner_id,
                                crate::backend::time_text(segment.start),
                                crate::backend::time_text(segment.end),
                                segment.start_value,
                                segment.interpolation,
                            )
                        } else {
                            format!(
                                "{}\t{}\t{}\t{}\t{}\t{}",
                                segment.owner_id,
                                crate::backend::time_text(segment.start),
                                crate::backend::time_text(segment.end),
                                segment.start_value,
                                segment.end_value,
                                segment.interpolation,
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn control_graph_timing(&self, category: i32, item: i32, control: i32) -> QStringList {
        self.control(category, item, control)
            .and_then(|control| control.scalar_graph.as_ref())
            .map(|graph| {
                [graph.range.0, graph.range.1, graph.frame_step]
                    .into_iter()
                    .flat_map(time_parts)
                    .map(|part| QString::from(part.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn control_graph_playhead(&self, category: i32, item: i32, control: i32) -> QStringList {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return QStringList::default();
        };
        let fallback = control.scalar_graph.as_ref().map(|graph| graph.playhead);
        super::current_keyframe_time(&target)
            .ok()
            .or(fallback)
            .map(|time| {
                time_parts(time)
                    .into_iter()
                    .map(|part| QString::from(part.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn seek_control_graph(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        numerator: i64,
        denominator: i64,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if control.scalar_graph.is_none() {
            return;
        }
        let result = shrimply_inspector_core::keyframe_model::exact_time(numerator, denominator)
            .and_then(|time| {
                crate::with_controller(|controller| {
                    controller.apply_control_graph_action(
                        &target,
                        &control,
                        shrimply_inspector_core::keyframe_graph::FrameGraphAction::PlayheadChanged(
                            time,
                        ),
                    )
                })
            });
        self.as_mut().finish(result);
    }

    pub fn move_control_graph_keys(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        old_times: &QStringList,
        times: &QStringList,
        values: &QStringList,
    ) -> QStringList {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return QStringList::default();
        };
        let result = (|| {
            if control.scalar_graph.is_none() {
                return Err("control has no keyframe graph".to_string());
            }
            if old_times.len() != times.len() || times.len() != values.len() {
                return Err("keyframe graph columns have different lengths".to_string());
            }
            let moves = old_times
                .iter()
                .zip(times.iter())
                .zip(values.iter())
                .map(|((old_time, time), value)| {
                    Ok(shrimply_inspector_core::keyframe_graph::FrameGraphKeyMove {
                        old_time: shrimply_inspector_core::keyframe_model::parse_time(
                            &old_time.to_string(),
                        )?,
                        time: shrimply_inspector_core::keyframe_model::parse_time(
                            &time.to_string(),
                        )?,
                        value: shrimply_inspector_core::keyframe_model::parse_graph_value(
                            &value.to_string(),
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            crate::with_controller(|controller| {
                controller.move_control_graph_keys(&target, &control, &moves)
            })
        })();
        match result {
            Ok(times) => {
                crate::EXPRESSION_DIRTY.set(true);
                crate::GRAPH_DIRTY.set(true);
                if timeline_path(&control).starts_with("/content/") {
                    super::mark_dirty();
                }
                times
                    .into_iter()
                    .map(|time| QString::from(crate::backend::time_text(time)))
                    .collect()
            }
            Err(error) => {
                self.as_mut().finish(Err(error));
                QStringList::default()
            }
        }
    }

    pub fn delete_control_graph_keys(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        times: &QStringList,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = parse_times(times).and_then(|times| {
            crate::with_controller(|controller| {
                controller.delete_control_keyframes(&target, &control, &times)
            })
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish(result);
    }

    pub fn add_control_graph_key(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        numerator: i64,
        denominator: i64,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = shrimply_inspector_core::keyframe_model::exact_time(numerator, denominator)
            .and_then(|time| {
                crate::with_controller(|controller| {
                    controller.add_control_keyframe(&target, &control, time)
                })
            });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish(result);
    }

    pub fn copy_control_graph_keys(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        times: &QStringList,
    ) -> bool {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return false;
        };
        let result = parse_times(times)
            .and_then(|times| {
                crate::with_controller(|controller| {
                    controller.copy_control_keyframes(&target, &control, &times)
                })
            })
            .map(|count| shrimply_inspector_core::document::graph::confirmation(count, false));
        let copied = matches!(&result, Ok(Some(_)));
        self.as_mut().finish_confirmation(result);
        copied
    }

    pub fn paste_control_graph_keys(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        numerator: i64,
        denominator: i64,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = shrimply_inspector_core::keyframe_model::exact_time(numerator, denominator)
            .and_then(|time| {
                crate::with_controller(|controller| {
                    controller.paste_control_keyframes(&target, &control, time)
                })
            })
            .map(|count| shrimply_inspector_core::document::graph::confirmation(count, true));
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish_confirmation(result);
    }

    pub fn set_control_graph_interpolation(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        owner_id: &QString,
        interpolation: i32,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = (|| {
            let owner_id =
                shrimply_inspector_core::keyframe_model::parse_owner_id(&owner_id.to_string())?;
            let interpolation = usize::try_from(interpolation)
                .map_err(|_| "keyframe interpolation is invalid".to_string())?;
            crate::with_controller(|controller| {
                controller.set_control_interpolation(&target, &control, owner_id, interpolation)
            })
        })();
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish(result);
    }

    pub fn text_interpolation_labels(&self) -> QStringList {
        shrimply_property_model::timeline_value::TextInterpolation::ALL
            .into_iter()
            .map(|interpolation| shrimply_i18n_qt::text(interpolation.label()))
            .collect()
    }

    pub fn text_interpolation_tooltips(&self) -> QStringList {
        shrimply_property_model::timeline_value::TextInterpolation::ALL
            .into_iter()
            .map(|interpolation| shrimply_i18n_qt::text(interpolation.tooltip()))
            .collect()
    }

    pub fn control_graph_text_interpolation(
        &self,
        category: i32,
        item: i32,
        control: i32,
        owner_id: &QString,
    ) -> i32 {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return -1;
        };
        let Ok(owner_id) =
            shrimply_inspector_core::keyframe_model::parse_owner_id(&owner_id.to_string())
        else {
            return -1;
        };
        crate::with_controller(|controller| {
            controller.control_text_interpolation_menu(&target, &control, owner_id)
        })
        .ok()
        .and_then(|menu| menu.value.parse().ok())
        .unwrap_or(-1)
    }

    pub fn set_control_graph_text_interpolation(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        owner_id: &QString,
        interpolation: i32,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = (|| {
            let owner_id =
                shrimply_inspector_core::keyframe_model::parse_owner_id(&owner_id.to_string())?;
            let interpolation = usize::try_from(interpolation)
                .map_err(|_| "text interpolation is invalid".to_string())?;
            crate::with_controller(|controller| {
                controller.set_control_text_interpolation(
                    &target,
                    &control,
                    owner_id,
                    interpolation,
                )
            })
        })();
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish(result);
    }

    pub fn toggle_control_graph_playback(self: Pin<&mut Self>) {
        super::toggle_keyframe_playback();
    }
}

fn timeline_path(control: &InspectorControl) -> &str {
    control.timeline_path.as_deref().unwrap_or(&control.path)
}

fn time_parts(time: shrimply_project_document::project::Time) -> [i64; 2] {
    [
        shrimply_property_model::timeline_value::fraction_numerator(time.seconds),
        shrimply_property_model::timeline_value::fraction_denominator(time.seconds),
    ]
}

fn parse_times(
    values: &QStringList,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    values
        .iter()
        .map(|value| shrimply_inspector_core::keyframe_model::parse_time(&value.to_string()))
        .collect()
}
