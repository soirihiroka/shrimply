use std::{cell::RefCell, path::PathBuf, rc::Rc};

use objc2::ClassType;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{NSStackView, NSTextField, NSView, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};
use shrimply_components_appkit::{
    ActionButton, MultilineTextInput, NumberPicker, SingleLineTextInput, Spinner, StringChoice,
    StringSelector, column_append_intrinsic, column_stack, control_row, row_stack, switch_row,
};
use shrimply_inspector_core::{InspectorControl, tts::TtsEditorControl, tts::TtsInputEdit};
use shrimply_math_core::fraction_as_f64;
use shrimply_tts::TtsModel;

use super::control::Context;

const FIELD_GAP: f64 = 8.0;
const TEXT_HEIGHT: f64 = 100.0;

#[derive(Clone)]
struct Editor {
    context: Context,
    model: Rc<TtsModel>,
}

impl Editor {
    fn edit(&self, key: &str, edit: TtsInputEdit) -> bool {
        match &edit {
            TtsInputEdit::Text(_) | TtsInputEdit::Number(_) | TtsInputEdit::TableCell { .. } => {}
            TtsInputEdit::Select(_)
            | TtsInputEdit::Toggle(_)
            | TtsInputEdit::Audio(_)
            | TtsInputEdit::AddTableRow
            | TtsInputEdit::RemoveTableRow(_) => self.context.end_editing(),
        }
        let result =
            self.context
                .controller
                .edit_tts_input(&self.context.target, &self.model, key, edit);
        let success = result.is_ok();
        match result {
            Ok(live) => {
                if !live {
                    self.context.dirty.set(true);
                }
            }
            Err(error) => self.context.finish(Err(error)),
        }
        success
    }

    fn commit(&self) {
        self.context.controller.finish_live_edit();
        self.context.dirty.set(true);
    }
}

pub(super) fn view(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let column = column_stack(FIELD_GAP, mtm);
    let Some(presentation) = &control.tts else {
        column_append_intrinsic(&column, &label(&control.subtitle, mtm));
        return column.as_super().into();
    };
    let selected = presentation
        .model
        .as_ref()
        .map_or("", |model| model.id.as_str());
    let model_context = context.clone();
    let selector = StringSelector::new(
        selected,
        presentation
            .models
            .iter()
            .map(|choice| StringChoice {
                value: choice.value.clone(),
                label: choice.label.clone(),
            })
            .collect(),
        move |id| {
            model_context.end_editing();
            let result = model_context.controller.select_tts_model(
                &model_context.target,
                &model_context.server_url.borrow(),
                &id,
            );
            if result.is_ok() {
                shrimply_editor_state::preferences::set_last_tts_model(
                    &model_context.preferences,
                    &id,
                );
            }
            model_context.refresh(result);
        },
        mtm,
    );
    column_append_intrinsic(&column, &control_row("Model", selector.view(), mtm));
    if !presentation.catalog_status.is_empty() {
        column_append_intrinsic(&column, &label(&presentation.catalog_status, mtm));
    }
    if presentation.can_retry {
        let retry_context = context.clone();
        let retry = ActionButton::new(
            "Retry",
            move || {
                retry_context.end_editing();
                retry_context
                    .controller
                    .retry_tts_models(&retry_context.server_url.borrow());
                retry_context.dirty.set(true);
            },
            mtm,
        );
        column_append_intrinsic(&column, retry.view());
    }
    if let Some(model) = &presentation.model {
        let editor = Editor {
            context: context.clone(),
            model: Rc::new(model.clone()),
        };
        for field in &presentation.controls {
            append_field(&column, field, &editor, mtm);
        }
        let generating = editor.clone();
        let button = ActionButton::new(
            if presentation.generated {
                "Regenerate"
            } else {
                "Generate"
            },
            move || {
                generating.context.end_editing();
                let was_running = generating
                    .context
                    .controller
                    .tts_generation(&generating.context.target)
                    .running;
                let result = generating.context.controller.toggle_tts_generation(
                    &generating.context.target,
                    &generating.context.server_url.borrow(),
                    &generating.model,
                );
                if !was_running && result.is_ok() {
                    shrimply_editor_state::preferences::set_last_tts_model(
                        &generating.context.preferences,
                        &generating.model.id,
                    );
                }
                generating.context.finish(result);
            },
            mtm,
        );
        let status = label("Ready", mtm);
        let spinner = Spinner::new(mtm);
        let status_row = row_stack(FIELD_GAP, mtm);
        status_row.addArrangedSubview(spinner.view());
        status_row.addArrangedSubview(&status);
        column_append_intrinsic(&column, button.view());
        column_append_intrinsic(&column, &status_row);
        let button = Weak::new(button.view());
        let status = Weak::new(&*status);
        let controller = context.controller.clone();
        let target = context.target.clone();
        let generated = presentation.generated;
        let previous = RefCell::new(None);
        let poll = move || {
            let (Some(button), Some(status)) = (button.load(), status.load()) else {
                return;
            };
            let state = controller.tts_generation(&target);
            if previous.borrow().as_ref() == Some(&state) {
                return;
            }
            spinner.set_active(state.running);
            button.setTitle(&NSString::from_str(if state.running {
                "Cancel"
            } else if generated {
                "Regenerate"
            } else {
                "Generate"
            }));
            button.setEnabled(!state.cancelling);
            status.setStringValue(&NSString::from_str(if state.status.is_empty() {
                "Ready"
            } else {
                &state.status
            }));
            status.setToolTip(Some(&NSString::from_str(&state.status)));
            previous.replace(Some(state));
        };
        poll();
        editor.context.polls.borrow_mut().push(Box::new(poll));
    }
    column.as_super().into()
}

fn append_field(
    column: &NSStackView,
    field: &TtsEditorControl,
    editor: &Editor,
    mtm: MainThreadMarker,
) {
    match field {
        TtsEditorControl::Text {
            key,
            label,
            value,
            multiline,
            max_length,
        } => {
            let changed = editor.clone();
            let committed = editor.clone();
            let key = key.clone();
            if *multiline {
                let input = MultilineTextInput::new(
                    value,
                    TEXT_HEIGHT,
                    Some(*max_length),
                    move |value| changed.edit(&key, TtsInputEdit::Text(value)),
                    move || committed.commit(),
                    mtm,
                );
                column_append_intrinsic(column, &control_row(label, input.view(), mtm));
            } else {
                let input = SingleLineTextInput::new(
                    value,
                    None,
                    Some(*max_length),
                    move |value| {
                        changed.edit(&key, TtsInputEdit::Text(value));
                    },
                    move |_| committed.commit(),
                    mtm,
                );
                column_append_intrinsic(column, &control_row(label, input.view(), mtm));
            }
        }
        TtsEditorControl::Select {
            key,
            label,
            value,
            choices,
        } => {
            let changed = editor.clone();
            let key = key.clone();
            let input = StringSelector::new(
                value,
                choices
                    .iter()
                    .map(|choice| StringChoice {
                        value: choice.value.clone(),
                        label: choice.label.clone(),
                    })
                    .collect(),
                move |value| {
                    changed.edit(&key, TtsInputEdit::Select(value));
                },
                mtm,
            );
            column_append_intrinsic(column, &control_row(label, input.view(), mtm));
        }
        TtsEditorControl::Toggle { key, label, value } => {
            let changed = editor.clone();
            let key = key.clone();
            let input = switch_row(
                label,
                None,
                *value,
                move |value| {
                    changed.edit(&key, TtsInputEdit::Toggle(value));
                },
                mtm,
            );
            column_append_intrinsic(column, &input);
        }
        TtsEditorControl::Number {
            key,
            label,
            value,
            minimum,
            maximum,
            step,
            digits,
        } => {
            let changed = editor.clone();
            let committed = editor.clone();
            let key = key.clone();
            let input = NumberPicker::fraction_builder(*value)
                .minimum(fraction_as_f64(*minimum))
                .maximum(fraction_as_f64(*maximum))
                .drag_step(fraction_as_f64(*step))
                .digits(*digits)
                .on_change_fraction(move |value| {
                    changed.edit(&key, TtsInputEdit::Number(value));
                })
                .on_commit_fraction(move |_| committed.commit())
                .build(mtm);
            column_append_intrinsic(column, &control_row(label, &input, mtm));
        }
        TtsEditorControl::Audio { key, label, path } => {
            audio(column, key, label, path, editor, mtm)
        }
        TtsEditorControl::Table {
            key,
            label: title,
            columns,
            rows,
        } => {
            column_append_intrinsic(column, &label(title, mtm));
            for (row_index, row) in rows.iter().enumerate() {
                let fields = column_stack(FIELD_GAP, mtm);
                for (column_index, definition) in columns.iter().enumerate() {
                    let changed = editor.clone();
                    let committed = editor.clone();
                    let key = key.clone();
                    let input = SingleLineTextInput::new(
                        row.get(&definition.key).map_or("", String::as_str),
                        None,
                        Some(definition.max_length),
                        move |value| {
                            changed.edit(
                                &key,
                                TtsInputEdit::TableCell {
                                    row: row_index,
                                    column: column_index,
                                    value,
                                },
                            );
                        },
                        move |_| committed.commit(),
                        mtm,
                    );
                    column_append_intrinsic(
                        &fields,
                        &control_row(&definition.label, input.view(), mtm),
                    );
                }
                let removed = editor.clone();
                let removed_key = key.clone();
                let remove = ActionButton::symbol(
                    "minus.circle",
                    "Remove row",
                    move || {
                        removed.edit(&removed_key, TtsInputEdit::RemoveTableRow(row_index));
                    },
                    mtm,
                );
                column_append_intrinsic(&fields, remove.view());
                column_append_intrinsic(column, &fields);
            }
            let added = editor.clone();
            let key = key.clone();
            let add = ActionButton::new(
                "Add row",
                move || {
                    added.edit(&key, TtsInputEdit::AddTableRow);
                },
                mtm,
            );
            column_append_intrinsic(column, add.view());
        }
    }
}

fn audio(
    column: &NSStackView,
    key: &str,
    title: &str,
    path: &Option<PathBuf>,
    editor: &Editor,
    mtm: MainThreadMarker,
) {
    let row = row_stack(FIELD_GAP, mtm);
    let chosen = editor.clone();
    let chosen_key = key.to_string();
    let choose = ActionButton::new(
        "Choose audio…",
        move || {
            chosen.context.end_editing();
            let chosen = chosen.clone();
            let key = chosen_key.clone();
            super::files::choose(
                shrimply_inspector_core::file_selection::FileSelection {
                    title: "Choose reference audio",
                    extensions: &[],
                },
                move |result| match result {
                    Ok(Some(path)) => {
                        chosen.edit(&key, TtsInputEdit::Audio(Some(path)));
                    }
                    Ok(None) => {}
                    Err(error) => chosen.context.finish(Err(error)),
                },
                mtm,
            );
        },
        mtm,
    );
    row.addArrangedSubview(choose.view());
    if let Some(path) = path {
        let path_label = label(&path.display().to_string(), mtm);
        column_append_intrinsic(column, &path_label);
        let reveal_path = path.clone();
        let reveal = ActionButton::symbol(
            "folder",
            "Reveal audio",
            move || {
                if let Some(url) = NSURL::from_file_path(&reveal_path) {
                    NSWorkspace::sharedWorkspace()
                        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
                }
            },
            mtm,
        );
        row.addArrangedSubview(reveal.view());
        let cleared = editor.clone();
        let key = key.to_string();
        let clear = ActionButton::symbol(
            "xmark",
            "Clear audio",
            move || {
                cleared.edit(&key, TtsInputEdit::Audio(None));
            },
            mtm,
        );
        row.addArrangedSubview(clear.view());
    }
    column_append_intrinsic(column, &control_row(title, &row, mtm));
}

fn label(value: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(value), mtm);
    label.setUsesSingleLineMode(true);
    label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    label.setToolTip(Some(&NSString::from_str(value)));
    label.setContentCompressionResistancePriority_forOrientation(
        objc2_app_kit::NSLayoutPriorityDefaultLow,
        objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
    );
    label
}
