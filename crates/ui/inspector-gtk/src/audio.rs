use shrimply_gtk_components::tr;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use shrimply_audio_modifiers::GainModifier;
use shrimply_core::timeline_value::TimelineValue;
use shrimply_math_core::Fraction;

use crate::InspectedItem as SelectedItem;
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::ui::NumberPicker;
use shrimply_gtk_components::ui::switch_row;
use shrimply_inspector_core::InspectorTarget;
use shrimply_project::project::{
    AudioItem, AudioSource, AudioSpeedMethod, Project, RepeatStrategy, default_playback_speed,
    playback_speed_or_default,
};

use super::{
    Inspectable, InspectorContext,
    item::DefaultInspectorItem,
    list,
    modifiers::{ScalarOptions, audio_item_scalar_row},
    section::InspectorSection,
    selector::{enum_selector, selector},
};

impl Inspectable for AudioItem {
    fn title(&self) -> &'static str {
        match &self.source {
            AudioSource::Media => "Audio",
            AudioSource::FoldedSequence(_) => "Folded Sequence",
            AudioSource::Tts(_) => "Text to Speech",
            AudioSource::Generator(_) => "Audio Generator",
        }
    }

    fn add_rows(&self, _section: &InspectorSection, _context: &InspectorContext) {}

    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let beat_detection = adw::ActionRow::builder()
            .title(tr!("Beat Detection").as_ref())
            .subtitle(tr!("Analyze this clip for beat-grid snapping").as_ref())
            .build();
        let spinner = adw::Spinner::new();
        spinner.set_size_request(18, 18);
        spinner.set_visible(self.beat_detection && shrimply_audio::beat::is_loading(self.id));
        beat_detection.add_suffix(&spinner);
        let beat_detection_toggle = gtk::Switch::builder()
            .active(self.beat_detection)
            .valign(gtk::Align::Center)
            .build();
        beat_detection.add_suffix(&beat_detection_toggle);
        beat_detection.set_activatable_widget(Some(&beat_detection_toggle));
        if self.beat_detection {
            let spinner = spinner.downgrade();
            let id = self.id;
            let mut polls_without_loading = 0_u8;
            let mut observed_loading = shrimply_audio::beat::is_loading(id);
            glib::timeout_add_local(Duration::from_millis(33), move || {
                let Some(spinner) = spinner.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let loading = shrimply_audio::beat::is_loading(id);
                spinner.set_visible(loading);
                observed_loading |= loading;
                if observed_loading && !loading {
                    return glib::ControlFlow::Break;
                }
                polls_without_loading = polls_without_loading.saturating_add((!loading) as u8);
                if polls_without_loading >= 60 {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
        let beat_context = context.detached();
        beat_detection_toggle.connect_active_notify(move |toggle| {
            update_beat_detection(&beat_context, toggle.is_active());
        });
        let file_backed = matches!(&self.source, AudioSource::Media | AudioSource::Tts(_))
            && !self.file.as_os_str().is_empty();
        let mut info_rows = if file_backed {
            audio_stream_rows(self, context)
        } else {
            Vec::new()
        };
        if file_backed {
            info_rows.push(beat_detection.upcast());
        }
        let info_items = vec![super::info::item(
            context,
            super::info::ItemInfo {
                leading: info_rows,
                kind: match &self.source {
                    AudioSource::Media => "Audio",
                    AudioSource::FoldedSequence(_) => "Folded Sequence",
                    AudioSource::Tts(_) => "Text to Speech",
                    AudioSource::Generator(_) => "Audio Generator",
                },
                natural_duration: (!matches!(&self.source, AudioSource::Generator(_)))
                    .then_some(self.source_duration),
                start: self.start,
                end: self.end,
                source_offset: Some(self.time_offset),
                dimensions: None,
                file: file_backed.then(|| self.file.clone()),
                source_metadata: if file_backed {
                    super::info::SourceMetadata::Audio(self.track_id)
                } else {
                    super::info::SourceMetadata::None
                },
            },
        )];
        let playback_items = (!matches!(&self.source, AudioSource::Generator(_))).then(|| {
            vec![
                DefaultInspectorItem::new(
                    "speed",
                    "Speed",
                    PlaybackSpeed {
                        speed: self.playback_speed,
                    },
                    Inspectable::controls,
                    |context, value: PlaybackSpeed| {
                        let Some(key) = context.selected_item.clone() else {
                            return;
                        };
                        update_audio_speed(
                            &context.project,
                            &context.player_state,
                            key.clone(),
                            value.speed,
                        );
                        shrimply_project::project::commit_edit(
                            &context.project.borrow(),
                            "audio-speed",
                        );
                        (context.refresh)();
                    },
                )
                .boxed(),
                DefaultInspectorItem::new(
                    "speed-method",
                    "Speed method",
                    self.speed_method,
                    Inspectable::controls,
                    |context, value: AudioSpeedMethod| {
                        let Some(key) = context.selected_item.clone() else {
                            return;
                        };
                        update_audio_speed_method(
                            &context.project,
                            &context.player_state,
                            key.clone(),
                            value,
                        );
                        (context.refresh)();
                    },
                )
                .boxed(),
                DefaultInspectorItem::new(
                    "repeat",
                    "Repeat",
                    AudioRepeatStrategy {
                        strategy: self.repeat_strategy,
                    },
                    Inspectable::controls,
                    |context, value: AudioRepeatStrategy| {
                        let Some(key) = context.selected_item.clone() else {
                            return;
                        };
                        update_audio_repeat_strategy(
                            &context.project,
                            &context.player_state,
                            key.clone(),
                            value.strategy,
                        );
                        (context.refresh)();
                    },
                )
                .boxed(),
            ]
        });
        let mut audio_items = vec![
            DefaultInspectorItem::new(
                "output",
                "Output",
                AudioOutput {
                    enabled: self.enabled,
                    gain: self.gain.as_ref().clone(),
                },
                audio_output_controls,
                reset_audio_output,
            )
            .boxed(),
        ];
        if let AudioSource::Generator(generator) = &self.source {
            audio_items.push(super::audio_generator::item(generator));
        }
        if let AudioSource::Tts(settings) = &self.source {
            let id = self.id;
            audio_items.push(
                DefaultInspectorItem::new(
                    "tts",
                    "Text to Speech",
                    (**settings).clone(),
                    move |settings, context| vec![tts_editor(id, settings, context)],
                    |context, settings| {
                        let Some(key) = context.selected_item.clone() else {
                            return;
                        };
                        if let Err(error) = context.inspector_core.set_tts_settings(
                            &InspectorTarget::Item(key),
                            settings,
                            false,
                        ) {
                            tracing::error!(%error, "Could not reset GTK TTS settings");
                            return;
                        }
                        (context.refresh)();
                    },
                )
                .boxed(),
            );
        }
        audio_items.extend(super::audio_modifiers::items(&self.modifiers, context));
        let mut categories = vec![list::InspectorCategory {
            key: "audio",
            label: "Audio",
            icon: "sound-symbolic",
            items: audio_items,
        }];
        if let Some(playback_items) = playback_items {
            categories.push(list::InspectorCategory {
                key: "playback",
                label: "Playback",
                icon: "playback-speed-symbolic",
                items: playback_items,
            });
        }
        categories.push(list::InspectorCategory {
            key: "info",
            label: "Info",
            icon: "info-outline-symbolic",
            items: info_items,
        });
        list::render_categories(categories, context)
    }
}

struct AudioOutput {
    enabled: bool,
    gain: GainModifier,
}

impl Default for AudioOutput {
    fn default() -> Self {
        Self {
            enabled: true,
            gain: Default::default(),
        }
    }
}

fn audio_output_controls(output: &AudioOutput, context: &InspectorContext) -> Vec<gtk::Widget> {
    let enabled_context = context.detached();
    let toggle = switch_row("Enabled", None, output.enabled, move |enabled| {
        update_audio_item_enabled(&enabled_context, enabled);
    });
    let section = InspectorSection::controls();
    section.add_wide_control(&toggle);
    section.add_wide_control(&audio_item_scalar_row(
        "Level",
        &output.gain.decibels,
        audio_gain,
        audio_gain_mut,
        ScalarOptions {
            minimum: Some(-60.0),
            maximum: Some(36.0),
            unit: Some("dB"),
            rotating: false,
        },
        context,
    ));
    vec![section.into_widget()]
}

fn audio_gain(project: &Project, key: SelectedItem) -> Option<&TimelineValue<f32>> {
    Some(&project.audio_item(&key)?.gain.decibels)
}

fn audio_gain_mut(project: &mut Project, key: SelectedItem) -> Option<&mut TimelineValue<f32>> {
    Some(&mut project.audio_item_mut(&key)?.gain.decibels)
}

fn reset_audio_output(context: &InspectorContext, output: AudioOutput) {
    let Some(key) = &context.selected_item else {
        return;
    };
    let mut project = context.project.borrow_mut();
    let Some(item) = project.audio_item_mut(key) else {
        return;
    };
    item.enabled = output.enabled;
    *item.gain = output.gain;
    shrimply_project::project::commit_edit(&project, "reset-audio-output");
    drop(project);
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            audio: true,
            audio_waveforms: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

fn update_audio_item_enabled(context: &InspectorContext, enabled: bool) {
    let Some(key) = &context.selected_item else {
        return;
    };
    let mut project = context.project.borrow_mut();
    let Some(item) = project.audio_item_mut(key) else {
        return;
    };
    if item.enabled == enabled {
        return;
    }
    item.enabled = enabled;
    shrimply_project::project::commit_edit(&project, "toggle-audio-item-enabled");
    drop(project);
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            audio: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

fn tts_editor(
    id: uuid::Uuid,
    settings: &shrimply_tts::TtsSettings,
    context: &InspectorContext,
) -> gtk::Widget {
    let Some(key) = context.selected_item.clone() else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let target = InspectorTarget::Item(key);
    let changed_controller = context.inspector_core.clone();
    let changed_target = target.clone();
    let commit_controller = context.inspector_core.clone();
    let generated_controller = context.inspector_core.clone();
    shrimply_tts_gtk::editor(
        id,
        context.preferences.clone(),
        settings,
        move |settings| {
            if let Err(error) = changed_controller.set_tts_settings(&changed_target, settings, true)
            {
                tracing::error!(%error, "Could not edit GTK TTS settings");
            }
        },
        move || commit_controller.finish_live_edit(),
        move |generation, model| {
            if let Err(error) =
                generated_controller.apply_tts_generation(&target, id, &model, generation)
            {
                tracing::error!(%error, "Could not apply GTK TTS generation");
            }
        },
    )
}

fn audio_stream_rows(item: &AudioItem, context: &InspectorContext) -> Vec<gtk::Widget> {
    let stream_count = super::info::audio_stream_count(&item.file) as u32;
    if stream_count < 2 {
        return Vec::new();
    }
    let labels = (0..stream_count)
        .map(|stream| {
            shrimply_gtk_components::i18n::text_args(
                "Audio stream %{number}",
                &[("number", (stream + 1).to_string())],
            )
        })
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let stream = adw::ComboRow::builder()
        .title(tr!("Audio Stream").as_ref())
        .model(&gtk::StringList::new(&label_refs))
        .selected(item.track_id.min(stream_count - 1))
        .build();

    if let Some(key) = context.selected_item.clone() {
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let refresh = context.refresh.clone();
        stream.connect_selected_notify(move |stream| {
            update_audio_stream(&project, &player_state, key.clone(), stream.selected());
            refresh();
        });
    } else {
        stream.set_sensitive(false);
    }

    vec![stream.upcast()]
}

fn update_beat_detection(context: &InspectorContext, enabled: bool) {
    let Some(key) = &context.selected_item.clone() else {
        return;
    };
    let mut project = context.project.borrow_mut();
    let Some(item) = project.audio_item_mut(key) else {
        return;
    };
    if item.beat_detection == enabled {
        return;
    }
    item.beat_detection = enabled;
    shrimply_project::project::commit_edit(&project, "toggle-beat-detection");
    drop(project);
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            audio_beats: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

struct PlaybackSpeed {
    speed: Fraction,
}

#[derive(Default)]
struct AudioRepeatStrategy {
    strategy: RepeatStrategy,
}

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self {
            speed: default_playback_speed(),
        }
    }
}

impl Inspectable for PlaybackSpeed {
    fn title(&self) -> &'static str {
        "Speed"
    }

    fn default_action(&self, context: &InspectorContext) -> Option<Box<dyn Fn() + 'static>> {
        let key = context.selected_item.clone()?;
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let refresh = context.refresh.clone();
        Some(Box::new(move || {
            update_audio_speed(
                &project,
                &player_state,
                key.clone(),
                default_playback_speed(),
            );
            shrimply_project::project::commit_edit(&project.borrow(), "audio-speed");
            refresh();
        }))
    }

    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        let Some(key) = context.selected_item.clone() else {
            return;
        };
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let commit_project = context.project.clone();
        let refresh = context.refresh.clone();
        let picker = NumberPicker::fraction_builder(playback_speed_or_default(self.speed))
            .drag_step(0.05)
            .digits(2)
            .unit_name("x")
            .width_chars(9)
            .on_change_fraction(move |value| {
                update_audio_speed(&project, &player_state, key.clone(), value)
            })
            .on_commit(move |_| {
                shrimply_project::project::commit_edit(&commit_project.borrow(), "audio-speed");
                refresh();
            })
            .build();
        section.add_control_row("Value", &picker);
    }
}

impl Inspectable for AudioSpeedMethod {
    fn title(&self) -> &'static str {
        "Pitch"
    }

    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        let Some(key) = context.selected_item.clone() else {
            return;
        };
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let dropdown = selector(
            "Method",
            *self,
            [
                (AudioSpeedMethod::Naive, "Naive"),
                (AudioSpeedMethod::PreservePitch, "Preserve pitch"),
            ],
            move |method| update_audio_speed_method(&project, &player_state, key.clone(), method),
        );
        section.add_wide_control(&dropdown);
    }
}

impl Inspectable for AudioRepeatStrategy {
    fn title(&self) -> &'static str {
        "Repeat"
    }

    fn add_rows(&self, section: &InspectorSection, context: &InspectorContext) {
        let Some(key) = context.selected_item.clone() else {
            return;
        };
        let project = context.project.clone();
        let player_state = context.player_state.clone();
        let dropdown = enum_selector("Strategy", self.strategy, move |strategy| {
            update_audio_repeat_strategy(&project, &player_state, key.clone(), strategy);
        });
        section.add_wide_control(&dropdown);
    }
}

fn update_audio_speed(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    value: Fraction,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.audio_item_mut(&key) else {
        return;
    };
    let value = playback_speed_or_default(value);
    if item.playback_speed == value {
        return;
    }

    item.playback_speed = value;
    let duration = project.duration();
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: true,
            audio_waveforms: true,
            ..ProjectChange::default()
        },
    );
}

fn update_audio_stream(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    track_id: u32,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.audio_item_mut(&key) else {
        return;
    };
    if item.track_id == track_id {
        return;
    }

    item.track_id = track_id;
    shrimply_project::project::commit_edit(&project, "audio-stream");
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            audio: true,
            audio_beats: true,
            audio_waveforms: true,
            ..ProjectChange::default()
        },
    );
}

fn update_audio_speed_method(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    method: AudioSpeedMethod,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.audio_item_mut(&key) else {
        return;
    };
    if item.speed_method == method {
        return;
    }

    item.speed_method = method;
    shrimply_project::project::commit_edit(&project, "audio-speed-method");
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            audio: true,
            ..ProjectChange::default()
        },
    );
}

fn update_audio_repeat_strategy(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    key: SelectedItem,
    strategy: RepeatStrategy,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.audio_item_mut(&key) else {
        return;
    };
    if item.repeat_strategy == strategy {
        return;
    }

    item.repeat_strategy = strategy;
    shrimply_project::project::commit_edit(&project, "audio-repeat-strategy");
    drop(project);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            audio: true,
            audio_waveforms: true,
            ..ProjectChange::default()
        },
    );
}
