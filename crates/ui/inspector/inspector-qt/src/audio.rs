use core::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList, QUrl};
use shrimply_inspector_core::{
    InspectorTarget,
    tts::{TtsEditorControl, TtsInputEdit},
};
use shrimply_math_core::fraction_as_f64;

type TtsTable<'a> = (
    &'a str,
    &'a [shrimply_tts::TableColumn],
    &'a [std::collections::BTreeMap<String, String>],
);

#[cxx_qt::bridge]
pub mod tts_qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, audio_id, cxx_name = "audioId")]
        #[qproperty(i32, revision)]
        #[qproperty(bool, ready)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, generating)]
        #[qproperty(QString, status)]
        #[qproperty(QString, status_tooltip, cxx_name = "statusTooltip")]
        #[qproperty(QString, generate_label, cxx_name = "generateLabel")]
        type TtsEditorBackend = super::TtsEditorBackendRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut TtsEditorBackend>);
        #[qinvokable]
        fn poll(self: Pin<&mut TtsEditorBackend>);
        #[qinvokable]
        #[cxx_name = "modelValue"]
        fn model_value(self: &TtsEditorBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "modelValues"]
        fn model_values(self: &TtsEditorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "modelLabels"]
        fn model_labels(self: &TtsEditorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "setModel"]
        fn set_model(self: Pin<&mut TtsEditorBackend>, value: &QString);
        #[qinvokable]
        #[cxx_name = "controlCount"]
        fn control_count(self: &TtsEditorBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "controlKind"]
        fn control_kind(self: &TtsEditorBackend, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "controlLabel"]
        fn control_label(self: &TtsEditorBackend, control: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "controlValue"]
        fn control_value(self: &TtsEditorBackend, control: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "controlMaximumLength"]
        fn control_maximum_length(self: &TtsEditorBackend, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "controlChoiceValues"]
        fn control_choice_values(self: &TtsEditorBackend, control: i32) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlChoiceLabels"]
        fn control_choice_labels(self: &TtsEditorBackend, control: i32) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlMinimum"]
        fn control_minimum(self: &TtsEditorBackend, control: i32) -> f64;
        #[qinvokable]
        #[cxx_name = "controlMaximum"]
        fn control_maximum(self: &TtsEditorBackend, control: i32) -> f64;
        #[qinvokable]
        #[cxx_name = "controlStep"]
        fn control_step(self: &TtsEditorBackend, control: i32) -> f64;
        #[qinvokable]
        #[cxx_name = "controlDigits"]
        fn control_digits(self: &TtsEditorBackend, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "setControlValue"]
        fn set_control_value(self: Pin<&mut TtsEditorBackend>, control: i32, value: &QString);
        #[qinvokable]
        #[cxx_name = "setControlToggle"]
        fn set_control_toggle(self: Pin<&mut TtsEditorBackend>, control: i32, value: bool);
        #[qinvokable]
        #[cxx_name = "setControlNumber"]
        fn set_control_number(
            self: Pin<&mut TtsEditorBackend>,
            control: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "commitControl"]
        fn commit_control(self: Pin<&mut TtsEditorBackend>);
        #[qinvokable]
        #[cxx_name = "chooseControlAudio"]
        fn choose_control_audio(self: Pin<&mut TtsEditorBackend>, control: i32);
        #[qinvokable]
        #[cxx_name = "clearControlAudio"]
        fn clear_control_audio(self: Pin<&mut TtsEditorBackend>, control: i32);
        #[qinvokable]
        #[cxx_name = "showControlAudio"]
        fn show_control_audio(self: Pin<&mut TtsEditorBackend>, control: i32);
        #[qinvokable]
        #[cxx_name = "tableColumnCount"]
        fn table_column_count(self: &TtsEditorBackend, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "tableRowCount"]
        fn table_row_count(self: &TtsEditorBackend, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "tableColumnLabel"]
        fn table_column_label(self: &TtsEditorBackend, control: i32, column: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "tableColumnMaximumLength"]
        fn table_column_maximum_length(self: &TtsEditorBackend, control: i32, column: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "tableValue"]
        fn table_value(self: &TtsEditorBackend, control: i32, row: i32, column: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "setTableValue"]
        fn set_table_value(
            self: Pin<&mut TtsEditorBackend>,
            control: i32,
            row: i32,
            column: i32,
            value: &QString,
        );
        #[qinvokable]
        #[cxx_name = "addTableRow"]
        fn add_table_row(self: Pin<&mut TtsEditorBackend>, control: i32);
        #[qinvokable]
        #[cxx_name = "removeTableRow"]
        fn remove_table_row(self: Pin<&mut TtsEditorBackend>, control: i32, row: i32);
        #[qinvokable]
        fn generate(self: Pin<&mut TtsEditorBackend>);
        #[qinvokable]
        fn cancel(self: Pin<&mut TtsEditorBackend>);

        #[qsignal]
        fn error(self: Pin<&mut TtsEditorBackend>, message: QString);
        #[qsignal]
        #[cxx_name = "openPath"]
        fn open_path(self: Pin<&mut TtsEditorBackend>, url: QUrl);
    }

    impl cxx_qt::Initialize for TtsEditorBackend {}
}

#[derive(Clone, PartialEq)]
struct TtsEditorView {
    target: InspectorTarget,
    server_url: String,
    editor: shrimply_inspector_core::tts::EditorPresentation,
    generation: shrimply_inspector_core::tts::GenerationPresentation,
}

fn build_tts_view(audio_id: uuid::Uuid) -> Result<TtsEditorView, String> {
    super::with_controller(|controller| {
        let target = controller.target();
        if !matches!(&target, InspectorTarget::Item(address @ shrimply_project_document::project::ItemAddress::Audio {..}) if address.item_id() == audio_id)
        {
            return Err("TTS editor selection changed".to_string());
        }
        let preferences = super::PREFERENCES.with_borrow(|preferences| {
            shrimply_editor_state::preferences::snapshot(
                preferences
                    .as_ref()
                    .expect("Qt TTS editor preferences are installed"),
            )
        });
        Ok(TtsEditorView {
            editor: controller.tts_editor(
                &target,
                &preferences.compute_server_url,
                &preferences.last_tts_model,
            )?,
            generation: controller.tts_generation(&target),
            target,
            server_url: preferences.compute_server_url,
        })
    })
}

pub struct TtsEditorBackendRust {
    audio_id: QString,
    revision: i32,
    ready: bool,
    busy: bool,
    generating: bool,
    status: QString,
    status_tooltip: QString,
    generate_label: QString,
    view: Option<TtsEditorView>,
}

impl Default for TtsEditorBackendRust {
    fn default() -> Self {
        Self {
            audio_id: QString::default(),
            revision: 0,
            ready: false,
            busy: true,
            generating: false,
            status: QString::default(),
            status_tooltip: QString::default(),
            generate_label: QString::default(),
            view: None,
        }
    }
}

impl cxx_qt::Initialize for tts_qobject::TtsEditorBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl tts_qobject::TtsEditorBackend {
    pub fn refresh(mut self: Pin<&mut Self>) {
        let view = uuid::Uuid::parse_str(&self.audio_id().to_string())
            .map_err(|_| "TTS editor audio ID is invalid".to_string())
            .and_then(build_tts_view);
        let (ready, busy, generating, status, label) = match &view {
            Ok(view) => (
                view.editor.model.is_some(),
                view.editor.loading || view.generation.running,
                view.generation.running,
                if view.generation.status.is_empty() {
                    view.editor.catalog_status.clone()
                } else {
                    view.generation.status.clone()
                },
                if view.editor.generated {
                    "Regenerate"
                } else {
                    "Generate"
                },
            ),
            Err(error) => (false, false, false, error.clone(), "Generate"),
        };
        self.as_mut().rust_mut().view = view.ok();
        self.as_mut().set_ready(ready);
        self.as_mut().set_busy(busy);
        self.as_mut().set_generating(generating);
        self.as_mut().set_status(QString::from(&status));
        self.as_mut().set_status_tooltip(QString::from(status));
        self.as_mut()
            .set_generate_label(shrimply_i18n_qt::text(label));
        let revision = self.revision().wrapping_add(1);
        self.as_mut().set_revision(revision);
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        if super::CONTROLLER.with_borrow(|controller| {
            controller
                .as_ref()
                .is_some_and(shrimply_inspector_document::poll)
        }) {
            super::mark_dirty();
        }
        let view = uuid::Uuid::parse_str(&self.audio_id().to_string())
            .ok()
            .and_then(|id| build_tts_view(id).ok());
        if view.as_ref() != self.rust().view.as_ref() {
            self.as_mut().refresh();
        }
    }

    pub fn model_value(&self) -> QString {
        self.rust()
            .view
            .as_ref()
            .and_then(|view| view.editor.model.as_ref())
            .map_or_else(QString::default, |model| QString::from(model.id.as_str()))
    }

    pub fn model_values(&self) -> QStringList {
        self.rust()
            .view
            .as_ref()
            .map(|view| {
                view.editor
                    .models
                    .iter()
                    .map(|model| QString::from(&model.value))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn model_labels(&self) -> QStringList {
        self.rust()
            .view
            .as_ref()
            .map(|view| {
                view.editor
                    .models
                    .iter()
                    .map(|model| QString::from(&model.label))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_model(mut self: Pin<&mut Self>, value: &QString) {
        let value = value.to_string();
        let result = self
            .rust()
            .view
            .as_ref()
            .ok_or_else(|| "TTS editor is not ready".to_string())
            .and_then(|view| {
                super::with_controller(|controller| {
                    controller.select_tts_model(&view.target, &view.server_url, &value)
                })?;
                super::PREFERENCES.with_borrow(|preferences| {
                    shrimply_editor_state::preferences::set_last_tts_model(
                        preferences
                            .as_ref()
                            .expect("Qt TTS editor preferences are installed"),
                        &value,
                    )
                });
                Ok(())
            });
        self.as_mut().finish(result);
    }

    pub fn control_count(&self) -> i32 {
        self.rust()
            .view
            .as_ref()
            .and_then(|view| i32::try_from(view.editor.controls.len()).ok())
            .unwrap_or_default()
    }

    pub fn control_kind(&self, control: i32) -> i32 {
        self.control(control).map_or(-1, |control| match control {
            TtsEditorControl::Text {
                multiline: false, ..
            } => 0,
            TtsEditorControl::Text {
                multiline: true, ..
            } => 1,
            TtsEditorControl::Select { .. } => 2,
            TtsEditorControl::Audio { .. } => 3,
            TtsEditorControl::Toggle { .. } => 4,
            TtsEditorControl::Number { .. } => 5,
            TtsEditorControl::Table { .. } => 6,
        })
    }

    pub fn control_label(&self, control: i32) -> QString {
        self.control(control)
            .map_or_else(QString::default, |control| {
                QString::from(control_label(control))
            })
    }

    pub fn control_value(&self, control: i32) -> QString {
        self.control(control)
            .map_or_else(QString::default, |control| {
                QString::from(match control {
                    TtsEditorControl::Text { value, .. }
                    | TtsEditorControl::Select { value, .. } => value.clone(),
                    TtsEditorControl::Audio { path, .. } => path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    TtsEditorControl::Toggle { value, .. } => value.to_string(),
                    TtsEditorControl::Number { value, .. } => fraction_as_f64(*value).to_string(),
                    TtsEditorControl::Table { .. } => String::new(),
                })
            })
    }

    pub fn control_maximum_length(&self, control: i32) -> i32 {
        self.control(control)
            .and_then(|control| match control {
                TtsEditorControl::Text { max_length, .. } => i32::try_from(*max_length).ok(),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn control_choice_values(&self, control: i32) -> QStringList {
        self.control(control)
            .and_then(|control| match control {
                TtsEditorControl::Select { choices, .. } => Some(
                    choices
                        .iter()
                        .map(|choice| QString::from(&choice.value))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn control_choice_labels(&self, control: i32) -> QStringList {
        self.control(control)
            .and_then(|control| match control {
                TtsEditorControl::Select { choices, .. } => Some(
                    choices
                        .iter()
                        .map(|choice| QString::from(&choice.label))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn control_minimum(&self, control: i32) -> f64 {
        self.number(control)
            .map_or(0.0, |(_, minimum, _, _)| fraction_as_f64(minimum))
    }

    pub fn control_maximum(&self, control: i32) -> f64 {
        self.number(control)
            .map_or(0.0, |(_, _, maximum, _)| fraction_as_f64(maximum))
    }

    pub fn control_step(&self, control: i32) -> f64 {
        self.number(control)
            .map_or(1.0, |(_, _, _, step)| fraction_as_f64(step))
    }

    pub fn control_digits(&self, control: i32) -> i32 {
        self.control(control)
            .and_then(|control| match control {
                TtsEditorControl::Number { digits, .. } => i32::try_from(*digits).ok(),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn set_control_value(mut self: Pin<&mut Self>, control: i32, value: &QString) {
        let value = value.to_string();
        let result = match self.control(control) {
            Some(TtsEditorControl::Text { .. }) => {
                self.edit_settings(control, TtsInputEdit::Text(value))
            }
            Some(TtsEditorControl::Select { .. }) => {
                self.edit_settings(control, TtsInputEdit::Select(value))
            }
            _ => Err("TTS control does not accept text".to_string()),
        };
        self.as_mut().finish(result);
    }

    pub fn set_control_toggle(mut self: Pin<&mut Self>, control: i32, value: bool) {
        let result = self.edit_settings(control, TtsInputEdit::Toggle(value));
        self.as_mut().finish(result);
    }

    pub fn set_control_number(
        mut self: Pin<&mut Self>,
        control: i32,
        numerator: i64,
        denominator: i64,
    ) {
        let result = if denominator <= 0 {
            Err("TTS number denominator must be positive".to_string())
        } else {
            self.edit_settings(
                control,
                TtsInputEdit::Number(shrimply_math_core::fraction_new(numerator, denominator)),
            )
        };
        self.as_mut().finish(result);
    }

    pub fn commit_control(mut self: Pin<&mut Self>) {
        super::finish_live_edit();
        self.as_mut().refresh();
    }

    pub fn choose_control_audio(mut self: Pin<&mut Self>, control: i32) {
        let Some(TtsEditorControl::Audio { key, label, .. }) = self.control(control).cloned()
        else {
            self.as_mut()
                .error(QString::from("TTS control is not an audio input"));
            return;
        };
        let Some(path) = shrimply_components_qt::file_picker::open(
            &format!("tts-{key}"),
            &label,
            "All files (*)",
        ) else {
            return;
        };
        let result = self.edit_settings(control, TtsInputEdit::Audio(Some(path)));
        self.as_mut().finish(result);
    }

    pub fn clear_control_audio(mut self: Pin<&mut Self>, control: i32) {
        let result = self.edit_settings(control, TtsInputEdit::Audio(None));
        self.as_mut().finish(result);
    }

    pub fn show_control_audio(mut self: Pin<&mut Self>, control: i32) {
        let result = self
            .control(control)
            .and_then(|control| match control {
                TtsEditorControl::Audio { path, .. } => path.clone(),
                _ => None,
            })
            .ok_or_else(|| "No TTS reference audio is selected".to_string())
            .and_then(|path| shrimply_components_qt::desktop_open::prepare(&path, None));
        match result {
            Ok(shrimply_components_qt::desktop_open::Action::Open(path)) => self
                .as_mut()
                .open_path(QUrl::from_local_file(&QString::from(
                    path.to_string_lossy().as_ref(),
                ))),
            Ok(shrimply_components_qt::desktop_open::Action::FocusRevealed(_)) => {}
            Err(error) => self.as_mut().error(QString::from(error)),
        }
    }

    pub fn table_column_count(&self, control: i32) -> i32 {
        self.table(control)
            .and_then(|(_, columns, _)| i32::try_from(columns.len()).ok())
            .unwrap_or_default()
    }

    pub fn table_row_count(&self, control: i32) -> i32 {
        self.table(control)
            .and_then(|(_, _, rows)| i32::try_from(rows.len()).ok())
            .unwrap_or_default()
    }

    pub fn table_column_label(&self, control: i32, column: i32) -> QString {
        self.table_column(control, column)
            .map_or_else(QString::default, |column| QString::from(&column.label))
    }

    pub fn table_column_maximum_length(&self, control: i32, column: i32) -> i32 {
        self.table_column(control, column)
            .and_then(|column| i32::try_from(column.max_length).ok())
            .unwrap_or_default()
    }

    pub fn table_value(&self, control: i32, row: i32, column: i32) -> QString {
        self.table(control)
            .and_then(|(_, columns, rows)| {
                rows.get(usize::try_from(row).ok()?)?
                    .get(&columns.get(usize::try_from(column).ok()?)?.key)
            })
            .map_or_else(QString::default, QString::from)
    }

    pub fn set_table_value(
        mut self: Pin<&mut Self>,
        control: i32,
        row: i32,
        column: i32,
        value: &QString,
    ) {
        let result = usize::try_from(row)
            .map_err(|_| "TTS table row is invalid".to_string())
            .and_then(|row| {
                usize::try_from(column)
                    .map_err(|_| "TTS table column is invalid".to_string())
                    .and_then(|column| {
                        self.edit_settings(
                            control,
                            TtsInputEdit::TableCell {
                                row,
                                column,
                                value: value.to_string(),
                            },
                        )
                    })
            });
        self.as_mut().finish(result);
    }

    pub fn add_table_row(mut self: Pin<&mut Self>, control: i32) {
        let result = self.edit_settings(control, TtsInputEdit::AddTableRow);
        self.as_mut().finish(result);
    }

    pub fn remove_table_row(mut self: Pin<&mut Self>, control: i32, row: i32) {
        let result = usize::try_from(row)
            .map_err(|_| "TTS table row is invalid".to_string())
            .and_then(|row| self.edit_settings(control, TtsInputEdit::RemoveTableRow(row)));
        self.as_mut().finish(result);
    }

    pub fn generate(mut self: Pin<&mut Self>) {
        let result = self
            .rust()
            .view
            .as_ref()
            .ok_or_else(|| "TTS editor is not ready".to_string())
            .and_then(|view| {
                super::with_controller(|controller| {
                    controller.toggle_tts_generation(
                        &view.target,
                        &view.server_url,
                        view.editor
                            .model
                            .as_ref()
                            .ok_or_else(|| "TTS model is unavailable".to_string())?,
                    )
                })
            });
        self.as_mut().finish(result);
    }

    pub fn cancel(mut self: Pin<&mut Self>) {
        let result = self
            .rust()
            .view
            .as_ref()
            .ok_or_else(|| "TTS editor is not ready".to_string())
            .and_then(|view| {
                super::with_controller(|controller| controller.cancel_tts_generation(&view.target))
            });
        self.as_mut().finish(result);
    }

    fn control(&self, control: i32) -> Option<&TtsEditorControl> {
        self.rust()
            .view
            .as_ref()?
            .editor
            .controls
            .get(usize::try_from(control).ok()?)
    }

    fn number(
        &self,
        control: i32,
    ) -> Option<(
        shrimply_math_core::Fraction,
        shrimply_math_core::Fraction,
        shrimply_math_core::Fraction,
        shrimply_math_core::Fraction,
    )> {
        match self.control(control)? {
            TtsEditorControl::Number {
                value,
                minimum,
                maximum,
                step,
                ..
            } => Some((*value, *minimum, *maximum, *step)),
            _ => None,
        }
    }

    fn table(&self, control: i32) -> Option<TtsTable<'_>> {
        match self.control(control)? {
            TtsEditorControl::Table {
                key, columns, rows, ..
            } => Some((key, columns, rows)),
            _ => None,
        }
    }

    fn table_column(&self, control: i32, column: i32) -> Option<&shrimply_tts::TableColumn> {
        self.table(control)?.1.get(usize::try_from(column).ok()?)
    }

    fn edit_settings(&self, control: i32, edit: TtsInputEdit) -> Result<(), String> {
        let view = self
            .rust()
            .view
            .as_ref()
            .ok_or_else(|| "TTS editor is not ready".to_string())?;
        let control = view
            .editor
            .controls
            .get(usize::try_from(control).map_err(|_| "TTS control index is invalid".to_string())?)
            .ok_or_else(|| "TTS control is unavailable".to_string())?;
        super::with_controller(|controller| {
            controller
                .edit_tts_input(
                    &view.target,
                    view.editor
                        .model
                        .as_ref()
                        .ok_or_else(|| "TTS model is unavailable".to_string())?,
                    control_key(control),
                    edit,
                )
                .map(|_| ())
        })
    }

    fn finish(mut self: Pin<&mut Self>, result: Result<(), String>) {
        match result {
            Ok(()) => self.as_mut().refresh(),
            Err(error) => self.as_mut().error(QString::from(error)),
        }
    }
}

fn control_label(control: &TtsEditorControl) -> &str {
    match control {
        TtsEditorControl::Text { label, .. }
        | TtsEditorControl::Select { label, .. }
        | TtsEditorControl::Audio { label, .. }
        | TtsEditorControl::Toggle { label, .. }
        | TtsEditorControl::Number { label, .. }
        | TtsEditorControl::Table { label, .. } => label,
    }
}

fn control_key(control: &TtsEditorControl) -> &str {
    match control {
        TtsEditorControl::Text { key, .. }
        | TtsEditorControl::Select { key, .. }
        | TtsEditorControl::Audio { key, .. }
        | TtsEditorControl::Toggle { key, .. }
        | TtsEditorControl::Number { key, .. }
        | TtsEditorControl::Table { key, .. } => key,
    }
}
