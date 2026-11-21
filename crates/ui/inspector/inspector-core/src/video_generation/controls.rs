use super::*;
use crate::{
    ControlKind as K, InspectorControl as C, InspectorControlAction as A, InspectorSection,
    NumberSpec, VideoCard,
};
use shrimply_math_core::fraction_as_f64;

fn action(path: impl Into<String>, label: &str, action: A, enabled: bool) -> C {
    C::new(K::Action, path, label)
        .action(action)
        .sensitive(enabled)
        .action_sensitive(enabled)
}
impl InspectorController {
    pub fn video_generation_card(
        &self,
        target: &InspectorTarget,
        server: &str,
    ) -> Result<Option<VideoCard>, String> {
        let InspectorTarget::Item(address) = target else {
            return Ok(None);
        };
        if self
            .project
            .borrow()
            .video_item(address)
            .is_none_or(|item| item.video_generation.is_none())
        {
            return Ok(None);
        }
        let (mut settings, generated) = self.video_generation_settings(target)?;
        let mut runtime = self.video_generation_runtime.borrow_mut();
        runtime.server = server.into();
        runtime
            .jobs
            .retain(|owner, job| owner == target || job.task.is_some());
        let catalog = runtime.catalog.request(server, None, models);
        let active = runtime
            .jobs
            .get(target)
            .is_some_and(|job| job.task.is_some());
        let mut controls = Vec::new();
        match catalog {
            None => controls.push(C::new(
                K::InfoLoading,
                "generation-catalog",
                "Loading video-generation models…",
            )),
            Some(Err(error)) => {
                controls.push(C::new(K::ReadOnly, "generation-error", "Status").value(error));
                controls.push(action(
                    "generation-retry",
                    "Retry",
                    A::RetryVideoGenerationModels,
                    !active,
                ));
            }
            Some(Ok(models)) => {
                let model = settings
                    .model
                    .as_ref()
                    .and_then(|id| models.iter().find(|model| &model.id == id))
                    .or_else(|| models.first())
                    .ok_or("Server has no video-generation models")?;
                shrimply_video_generation::sync_settings(&mut settings, model);
                let model_id = signature(model);
                let mut selector = C::new(K::Selector, MODEL_PATH, "Model")
                    .value(&model.id)
                    .sensitive(!active);
                selector.values = models.iter().map(|model| model.id.clone()).collect();
                selector.labels = models.iter().map(|model| model.label.clone()).collect();
                selector.commit_name = "edit-video-generation".into();
                selector.commit_immediately = true;
                controls.push(selector);
                for (index, input) in model.inputs.iter().enumerate().filter(|(_, input)| {
                    shrimply_video_generation::is_visible(input, &settings.inputs)
                }) {
                    let path = format!("{INPUT_PREFIX}{model_id}/{index}");
                    let stored = settings
                        .inputs
                        .get(input.key())
                        .expect("synchronized generation input");
                    let mut control = match (input, stored) {
                        (
                            InputDefinition::Text {
                                label, multiline, ..
                            },
                            VideoGenerationValue::Text { value },
                        ) => C::new(
                            if *multiline {
                                K::MultilineText
                            } else {
                                K::Text
                            },
                            path,
                            label,
                        )
                        .value(value),
                        (
                            InputDefinition::Select { label, options, .. },
                            VideoGenerationValue::Select { value },
                        ) => {
                            let mut control = C::new(K::Selector, path, label).value(value);
                            control.values =
                                options.iter().map(|option| option.value.clone()).collect();
                            control.labels =
                                options.iter().map(|option| option.label.clone()).collect();
                            control
                        }
                        (
                            InputDefinition::Number {
                                label,
                                minimum,
                                maximum,
                                step,
                                ..
                            },
                            VideoGenerationValue::Number { value },
                        ) => {
                            let mut control =
                                C::new(K::Fraction, path, label).value(value.to_string());
                            control.number = NumberSpec {
                                minimum: fraction_as_f64(*minimum),
                                maximum: fraction_as_f64(*maximum),
                                drag_step: fraction_as_f64(*step),
                                digits: crate::tts::decimal_places(*step) as i32,
                                unit: "",
                            };
                            control
                        }
                        (
                            InputDefinition::Media {
                                label,
                                maximum_items,
                                ordered,
                                ..
                            },
                            VideoGenerationValue::Media { items },
                        ) => {
                            let items_id = signature(items);
                            for (media_index, item) in items.iter().enumerate() {
                                controls.push(
                                    C::new(
                                        K::ReadOnly,
                                        format!("{path}/{items_id}/{media_index}/name"),
                                        label,
                                    )
                                    .value(item.value.path().to_string_lossy()),
                                );
                                if *ordered {
                                    for (label, offset, enabled) in [
                                        ("Move up", -1, media_index > 0),
                                        ("Move down", 1, media_index + 1 < items.len()),
                                    ] {
                                        controls.push(action(
                                            format!("{path}/{items_id}/{media_index}/{offset}"),
                                            label,
                                            A::MoveVideoGenerationMedia {
                                                model: model_id,
                                                input: index,
                                                index: media_index,
                                                offset,
                                                items: items_id,
                                            },
                                            !active && enabled,
                                        ));
                                    }
                                }
                                controls.push(action(
                                    format!("{path}/{items_id}/{media_index}/remove"),
                                    "Remove",
                                    A::RemoveVideoGenerationMedia {
                                        model: model_id,
                                        input: index,
                                        index: media_index,
                                        items: items_id,
                                    },
                                    !active,
                                ));
                            }
                            controls.push(
                                action(
                                    path,
                                    label,
                                    A::SelectVideoGenerationMedia {
                                        model: model_id,
                                        input: index,
                                    },
                                    !active && items.len() < *maximum_items,
                                )
                                .value(if *maximum_items == 1 {
                                    "Choose…"
                                } else {
                                    "Add…"
                                }),
                            );
                            continue;
                        }
                        _ => unreachable!("generation settings match model definitions"),
                    };
                    control.sensitive = !active;
                    control.commit_name = "edit-video-generation".into();
                    controls.push(control);
                }
                if !active {
                    controls.push(action(
                        format!("generation-start/{model_id}"),
                        if generated { "Regenerate" } else { "Generate" },
                        A::GenerateVideo { model: model_id },
                        true,
                    ));
                }
            }
        }
        if let Some(job) = runtime.jobs.get(target) {
            controls.push(
                C::new(
                    if active { K::InfoLoading } else { K::ReadOnly },
                    "generation-status",
                    "Status",
                )
                .value(shrimply_i18n::text(&job.status)),
            );
        }
        if active {
            controls.push(action(
                "generation-cancel",
                "Cancel",
                A::CancelVideoGeneration,
                !runtime.jobs.get(target).is_some_and(|job| job.cancelling),
            ));
        }
        Ok(Some(
            VideoCard::new(
                "video-generation",
                "Video Generation",
                InspectorSection { controls },
            )
            .reset(
                "/video_generation",
                serde_json::to_value(VideoGenerationSettings::default())
                    .expect("generation settings serialize"),
                "reset-video-generation",
            ),
        ))
    }
}
