#![expect(
    clippy::too_many_arguments,
    reason = "CXX-Qt QML slot signatures are fixed by the public QML API"
)]

use core::pin::Pin;
use std::collections::HashMap;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QColor, QString, QStringList, QUrl};
use shrimply_inspector_core::InspectorTarget;

use crate::item::{InspectorAction, InspectorListItem};
use crate::list::{InspectorDocument, InspectorListState};
use crate::section::{ControlKind, ControlRowRole, InspectorControl};
use crate::value_backend::{boolean_action, fraction_value};

mod read;
mod write;

const MISSING_CONTROL_INDEX: i32 = -1;

type AnalysisControlKey = (usize, usize, usize);

#[derive(Clone, Debug, PartialEq)]
struct CachedAnalysisControl {
    target: InspectorTarget,
    action: shrimply_inspector_core::InspectorControlAction,
    presentation: shrimply_inspector_core::AnalysisControlPresentation,
}

fn analysis_presentation(
    control: &InspectorControl,
    target: Option<&InspectorTarget>,
) -> Option<shrimply_inspector_core::AnalysisControlPresentation> {
    match control.action? {
        shrimply_inspector_core::InspectorControlAction::ToggleSam2Analysis {
            modifier_id,
            generation,
            prompt_signature,
            can_analyze,
        } => {
            let InspectorTarget::Item(address) = target? else {
                return None;
            };
            Some(shrimply_inspector_core::sam2_analysis_control(
                &shrimply_visual_core::sam2::analysis::AnalysisTarget {
                    address: address.clone(),
                    modifier_id,
                },
                generation,
                prompt_signature,
                can_analyze,
            ))
        }
        shrimply_inspector_core::InspectorControlAction::ToggleTransparentFillAnalysis {
            modifier_id,
        } => super::transparent_fill_analysis_control(target?, modifier_id).ok(),
        shrimply_inspector_core::InspectorControlAction::ToggleCameraAnalysis => {
            super::camera_analysis_control(target?).ok()
        }
        _ => None,
    }
}

fn analysis_tooltip(status: &shrimply_inspector_core::AnalysisControlPresentation) -> QString {
    match &status.tooltip {
        shrimply_inspector_core::AnalysisTooltip::MessageKey(message) => {
            shrimply_i18n_qt::text(message)
        }
        shrimply_inspector_core::AnalysisTooltip::RawError(error) => QString::from(error.as_str()),
    }
}

fn analysis_control_cache(
    document: &InspectorDocument,
) -> HashMap<AnalysisControlKey, CachedAnalysisControl> {
    let mut cache = HashMap::new();
    for (category_index, category) in document.categories.iter().enumerate() {
        for (item_index, item) in category.items.iter().enumerate() {
            let section = match item {
                InspectorListItem::Item(item) => &item.section,
                InspectorListItem::Flat(section) => section,
            };
            for (control_index, control) in section.controls.iter().enumerate() {
                if control.kind != ControlKind::Analysis {
                    continue;
                }
                let Some(action) = control.action else {
                    continue;
                };
                let Some(presentation) = control.analysis.clone() else {
                    continue;
                };
                cache.insert(
                    (category_index, item_index, control_index),
                    CachedAnalysisControl {
                        target: document.target.clone(),
                        action,
                        presentation,
                    },
                );
            }
        }
    }
    cache
}

#[cxx_qt::bridge]
pub(crate) mod qobject {
    #[qenum(InspectorBackend)]
    enum InspectorControlKind {
        Boolean,
        Number,
        Fraction,
        Text,
        MultilineText,
        ReadOnly,
        Color,
        LayeredColor,
        LayeredText,
        LayeredDrawing,
        FontFamilies,
        Selector,
        Vector2,
        Vector3,
        LayeredNumber,
        LayeredVector2,
        LayeredVector3,
        ProjectSettings,
        Performance,
        LayeredBoolean,
        LayeredSelector,
        AudioCache,
        AudioCachePreset,
        VisualCache,
        VisualCacheQuality,
        Analysis,
        ModifierMenu,
        TtsEditor,
        BeatDetection,
        InfoHeading,
        InfoArtwork,
        FileLocation,
        InfoLoading,
        Action,
    }

    #[qenum(InspectorBackend)]
    enum InspectorControlRowRole {
        Standalone,
        Primary,
        Auxiliary,
        TrailingAction,
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, ready)]
        #[qproperty(i32, revision)]
        #[qproperty(i32, cache_revision, cxx_name = "cacheRevision")]
        #[qproperty(i32, analysis_revision, cxx_name = "analysisRevision")]
        #[qproperty(i32, document_revision, cxx_name = "documentRevision")]
        #[qproperty(i32, expression_revision, cxx_name = "expressionRevision")]
        #[qproperty(i32, graph_revision, cxx_name = "graphRevision")]
        #[qproperty(i32, playhead_revision, cxx_name = "playheadRevision")]
        #[qproperty(i32, transform_revision, cxx_name = "transformRevision")]
        #[qproperty(i32, font_browser_revision, cxx_name = "fontBrowserRevision")]
        #[qproperty(i32, active_category, cxx_name = "activeCategory")]
        #[qproperty(f64, scroll_position, cxx_name = "scrollPosition")]
        #[qproperty(QString, title)]
        type InspectorBackend = super::InspectorBackendRust;

        #[qinvokable]
        fn poll(self: Pin<&mut InspectorBackend>, scroll_position: f64);
        #[qinvokable]
        #[cxx_name = "pollAnalysisControl"]
        fn poll_analysis_control(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
        );
        #[qinvokable]
        #[cxx_name = "targetChangePending"]
        fn target_change_pending(self: &InspectorBackend) -> bool;
        #[qinvokable]
        #[cxx_name = "minimumWidth"]
        fn minimum_width(self: &InspectorBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "destructiveBackground"]
        fn destructive_background(self: &InspectorBackend) -> QColor;
        #[qinvokable]
        #[cxx_name = "destructiveForeground"]
        fn destructive_foreground(self: &InspectorBackend) -> QColor;
        #[qinvokable]
        #[cxx_name = "keyframeClipboardMarker"]
        fn keyframe_clipboard_marker(self: &InspectorBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "keyframeSnappingEnabled"]
        fn keyframe_snapping_enabled(self: &InspectorBackend) -> bool;
        #[qinvokable]
        #[cxx_name = "keyframeSnappingRadius"]
        fn keyframe_snapping_radius(self: &InspectorBackend) -> f64;
        #[qinvokable]
        #[cxx_name = "categoryKeys"]
        fn category_keys(self: &InspectorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "categoryLabel"]
        fn category_label(self: &InspectorBackend, category: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "categoryIcon"]
        fn category_icon(self: &InspectorBackend, category: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "activateCategory"]
        fn activate_category(self: Pin<&mut InspectorBackend>, category: i32);

        #[qinvokable]
        #[cxx_name = "itemKeys"]
        fn item_keys(self: &InspectorBackend, category: i32) -> QStringList;
        #[qinvokable]
        #[cxx_name = "itemIsCard"]
        fn item_is_card(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemIdentity"]
        fn item_identity(self: &InspectorBackend, category: i32, item: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "itemTitle"]
        fn item_title(self: &InspectorBackend, category: i32, item: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "itemFocusAvailable"]
        fn item_focus_available(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemFocused"]
        fn item_focused(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "focusItem"]
        fn focus_item(self: Pin<&mut InspectorBackend>, category: i32, item: i32);
        #[qinvokable]
        #[cxx_name = "focusItemBody"]
        fn focus_item_body(self: Pin<&mut InspectorBackend>, category: i32, item: i32);
        #[qinvokable]
        #[cxx_name = "focusControl"]
        fn focus_control(self: Pin<&mut InspectorBackend>, category: i32, item: i32, control: i32);
        #[qinvokable]
        #[cxx_name = "itemExpanded"]
        fn item_expanded(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "setItemExpanded"]
        fn set_item_expanded(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            expanded: bool,
        );
        #[qinvokable]
        #[cxx_name = "itemResetAvailable"]
        fn item_reset_available(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "resetItem"]
        fn reset_item(self: Pin<&mut InspectorBackend>, category: i32, item: i32);

        #[qinvokable]
        #[cxx_name = "itemHasToggle"]
        fn item_has_toggle(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemToggleActive"]
        fn item_toggle_active(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemToggleTooltip"]
        fn item_toggle_tooltip(self: &InspectorBackend, category: i32, item: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "setItemToggle"]
        fn set_item_toggle(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            active: bool,
        );
        #[qinvokable]
        #[cxx_name = "itemHasButtonToggle"]
        fn item_has_button_toggle(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemButtonToggleActive"]
        fn item_button_toggle_active(self: &InspectorBackend, category: i32, item: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "itemButtonToggleIcon"]
        fn item_button_toggle_icon(self: &InspectorBackend, category: i32, item: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "itemButtonToggleTooltip"]
        fn item_button_toggle_tooltip(self: &InspectorBackend, category: i32, item: i32)
        -> QString;
        #[qinvokable]
        #[cxx_name = "setItemButtonToggle"]
        fn set_item_button_toggle(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            active: bool,
        );
        #[qinvokable]
        #[cxx_name = "itemActionCount"]
        fn item_action_count(self: &InspectorBackend, category: i32, item: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "itemActionIcon"]
        fn item_action_icon(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            action: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "itemActionTooltip"]
        fn item_action_tooltip(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            action: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "itemActionSensitive"]
        fn item_action_sensitive(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            action: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "triggerItemAction"]
        fn trigger_item_action(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            action: i32,
        );

        #[qinvokable]
        #[cxx_name = "controlCount"]
        fn control_count(self: &InspectorBackend, category: i32, item: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "controlKind"]
        fn control_kind(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> InspectorControlKind;
        #[qinvokable]
        #[cxx_name = "controlRowRole"]
        fn control_row_role(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> InspectorControlRowRole;
        #[qinvokable]
        #[cxx_name = "controlRowMember"]
        fn control_row_member(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            role: InspectorControlRowRole,
        ) -> i32;
        #[qinvokable]
        #[cxx_name = "controlLabel"]
        fn control_label(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlTransformLive"]
        fn control_transform_live(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlSubtitle"]
        fn control_subtitle(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlTooltip"]
        fn control_tooltip(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlValue"]
        fn control_value(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlComponent"]
        fn control_component(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            component: i32,
        ) -> f64;
        #[qinvokable]
        #[cxx_name = "controlColor"]
        fn control_color(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlComponentText"]
        fn control_component_text(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            component: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlEditable"]
        fn control_editable(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlSensitive"]
        fn control_sensitive(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlVisible"]
        fn control_visible(self: &InspectorBackend, category: i32, item: i32, control: i32)
        -> bool;
        #[qinvokable]
        #[cxx_name = "controlBusy"]
        fn control_busy(self: &InspectorBackend, category: i32, item: i32, control: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "controlMinimum"]
        fn control_minimum(self: &InspectorBackend, category: i32, item: i32, control: i32) -> f64;
        #[qinvokable]
        #[cxx_name = "controlMaximum"]
        fn control_maximum(self: &InspectorBackend, category: i32, item: i32, control: i32) -> f64;
        #[qinvokable]
        #[cxx_name = "controlDragStep"]
        fn control_drag_step(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> f64;
        #[qinvokable]
        #[cxx_name = "controlDigits"]
        fn control_digits(self: &InspectorBackend, category: i32, item: i32, control: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "controlUnit"]
        fn control_unit(self: &InspectorBackend, category: i32, item: i32, control: i32)
        -> QString;
        #[qinvokable]
        #[cxx_name = "controlWidthCharacters"]
        fn control_width_characters(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> i32;
        #[qinvokable]
        #[cxx_name = "controlPrefixIcon"]
        fn control_prefix_icon(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlHasAction"]
        fn control_has_action(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlActionIcon"]
        fn control_action_icon(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlActionSensitive"]
        fn control_action_sensitive(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlActionTooltip"]
        fn control_action_tooltip(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlDragPayload"]
        fn control_drag_payload(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlPrefixIconRotates"]
        fn control_prefix_icon_rotates(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlPrefixIconRotationOffset"]
        fn control_prefix_icon_rotation_offset(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> f64;
        #[qinvokable]
        #[cxx_name = "controlPrefix"]
        fn control_prefix(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            component: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlLock"]
        fn control_lock(self: &InspectorBackend, category: i32, item: i32, control: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "controlWithAlpha"]
        fn control_with_alpha(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlChoiceValues"]
        fn control_choice_values(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlChoiceLabels"]
        fn control_choice_labels(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlChoiceIcons"]
        fn control_choice_icons(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlChoiceSearchTerms"]
        fn control_choice_search_terms(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "openFontBrowser"]
        fn open_font_browser(self: Pin<&mut InspectorBackend>);
        #[qinvokable]
        #[cxx_name = "searchFontBrowser"]
        fn search_font_browser(self: Pin<&mut InspectorBackend>, query: &QString);
        #[qinvokable]
        #[cxx_name = "requestFontBrowserPreviews"]
        fn request_font_browser_previews(self: Pin<&mut InspectorBackend>, first: i32, count: i32);
        #[qinvokable]
        #[cxx_name = "fontBrowserCount"]
        fn font_browser_count(self: &InspectorBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "fontBrowserLabel"]
        fn font_browser_label(self: &InspectorBackend, choice: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "fontBrowserValue"]
        fn font_browser_value(self: &InspectorBackend, choice: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "fontBrowserGoogle"]
        fn font_browser_google(self: &InspectorBackend, choice: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "fontBrowserPreviewSource"]
        fn font_browser_preview_source(self: &InspectorBackend, choice: i32) -> QUrl;
        #[qinvokable]
        #[cxx_name = "fontBrowserStatus"]
        fn font_browser_status(self: &InspectorBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "fontBrowserBusy"]
        fn font_browser_busy(self: &InspectorBackend) -> bool;
        #[qinvokable]
        #[cxx_name = "fontListWithChoice"]
        fn font_list_with_choice(
            self: &InspectorBackend,
            value: &QString,
            index: i32,
            family: &QString,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "moveFontListValue"]
        fn move_font_list_value(
            self: &InspectorBackend,
            value: &QString,
            index: i32,
            offset: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "removeFontListValue"]
        fn remove_font_list_value(self: &InspectorBackend, value: &QString, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "activateControlFont"]
        fn activate_control_font(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            family: &QString,
            value: &QString,
        );
        #[qinvokable]
        #[cxx_name = "cancelControlFontActivation"]
        fn cancel_control_font_activation(self: &InspectorBackend);
        #[qinvokable]
        #[cxx_name = "controlKeyframes"]
        fn control_keyframes(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlExpression"]
        fn control_expression(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "controlExpressionSource"]
        fn control_expression_source(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "controlExpressionResult"]
        fn control_expression_result(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "expressionDiagnostic"]
        fn expression_diagnostic(self: Pin<&mut InspectorBackend>, source: &QString)
        -> QStringList;
        #[qinvokable]
        #[cxx_name = "expressionDiagnosticDebounce"]
        fn expression_diagnostic_debounce(self: &InspectorBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "expressionCompletionDebounce"]
        fn expression_completion_debounce(self: &InspectorBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "controlExpressionCompletion"]
        fn control_expression_completion(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            source: &QString,
            cursor: i32,
            automatic: bool,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphPointTimes"]
        fn control_graph_point_times(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphPointValues"]
        fn control_graph_point_values(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphSegments"]
        fn control_graph_segments(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphTiming"]
        fn control_graph_timing(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphPlayhead"]
        fn control_graph_playhead(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
        ) -> QStringList;

        #[qinvokable]
        #[cxx_name = "setControlValue"]
        fn set_control_value(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            value: &QString,
        );
        #[qinvokable]
        #[cxx_name = "setControlFraction"]
        fn set_control_fraction(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "setControlComponents"]
        fn set_control_components(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            first: f64,
            second: f64,
            third: f64,
            changed: i32,
        );
        #[qinvokable]
        #[cxx_name = "setControlColor"]
        fn set_control_color(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            red: f64,
            green: f64,
            blue: f64,
            alpha: f64,
        );
        #[qinvokable]
        #[cxx_name = "commitControl"]
        fn commit_control(self: Pin<&mut InspectorBackend>, category: i32, item: i32, control: i32);
        #[qinvokable]
        #[cxx_name = "setControlKeyframes"]
        fn set_control_keyframes(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            enabled: bool,
        );
        #[qinvokable]
        #[cxx_name = "setControlExpression"]
        fn set_control_expression(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            enabled: bool,
        );
        #[qinvokable]
        #[cxx_name = "setControlExpressionSource"]
        fn set_control_expression_source(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            source: &QString,
        );
        #[qinvokable]
        #[cxx_name = "seekControlGraph"]
        fn seek_control_graph(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "moveControlGraphKeys"]
        fn move_control_graph_keys(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            old_times: &QStringList,
            times: &QStringList,
            values: &QStringList,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "deleteControlGraphKeys"]
        fn delete_control_graph_keys(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            times: &QStringList,
        );
        #[qinvokable]
        #[cxx_name = "addControlGraphKey"]
        fn add_control_graph_key(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "copyControlGraphKeys"]
        fn copy_control_graph_keys(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            times: &QStringList,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "pasteControlGraphKeys"]
        fn paste_control_graph_keys(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "setControlGraphInterpolation"]
        fn set_control_graph_interpolation(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            owner_id: &QString,
            interpolation: i32,
        );
        #[qinvokable]
        #[cxx_name = "textInterpolationLabels"]
        fn text_interpolation_labels(self: &InspectorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "textInterpolationTooltips"]
        fn text_interpolation_tooltips(self: &InspectorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "controlGraphTextInterpolation"]
        fn control_graph_text_interpolation(
            self: &InspectorBackend,
            category: i32,
            item: i32,
            control: i32,
            owner_id: &QString,
        ) -> i32;
        #[qinvokable]
        #[cxx_name = "setControlGraphTextInterpolation"]
        fn set_control_graph_text_interpolation(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
            owner_id: &QString,
            interpolation: i32,
        );
        #[qinvokable]
        #[cxx_name = "toggleControlGraphPlayback"]
        fn toggle_control_graph_playback(self: Pin<&mut InspectorBackend>);
        #[qinvokable]
        #[cxx_name = "triggerControlAction"]
        fn trigger_control_action(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
        );
        #[qinvokable]
        #[cxx_name = "triggerSecondaryControlAction"]
        fn trigger_secondary_control_action(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
        );
        #[qinvokable]
        #[cxx_name = "showControlPath"]
        fn show_control_path(
            self: Pin<&mut InspectorBackend>,
            category: i32,
            item: i32,
            control: i32,
        );
        #[qinvokable]
        #[cxx_name = "applyProjectSettings"]
        fn apply_project_settings(
            self: Pin<&mut InspectorBackend>,
            width: i32,
            height: i32,
            fps_numerator: &QString,
            fps_denominator: &QString,
        );

        #[qsignal]
        #[cxx_name = "showError"]
        fn show_error(self: Pin<&mut InspectorBackend>, body: QString);
        #[qsignal]
        #[cxx_name = "showConfirmation"]
        fn show_confirmation(self: Pin<&mut InspectorBackend>, body: QString);
        #[qsignal]
        #[cxx_name = "openPath"]
        fn open_path(self: Pin<&mut InspectorBackend>, url: QUrl);
    }

    impl cxx_qt::Initialize for InspectorBackend {}
}

#[derive(Default)]
pub struct InspectorBackendRust {
    ready: bool,
    revision: i32,
    cache_revision: i32,
    analysis_revision: i32,
    document_revision: i32,
    expression_revision: i32,
    graph_revision: i32,
    playhead_revision: i32,
    transform_revision: i32,
    font_browser_revision: i32,
    active_category: i32,
    scroll_position: f64,
    title: QString,
    document: Option<InspectorDocument>,
    list_state: InspectorListState,
    stabilization_generating: Option<bool>,
    resolved_transform: Option<shrimply_project_document::project::ResolvedTransform>,
    transform_live: Option<shrimply_inspector_core::transform::TransformLivePresentation>,
    analysis_controls: HashMap<AnalysisControlKey, CachedAnalysisControl>,
    expression_diagnostic_cache: shrimply_inspector_core::rhai_editor::DiagnosticCache,
}

impl cxx_qt::Initialize for qobject::InspectorBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::InspectorBackend {
    fn transform_active(&self) -> bool {
        let Some(document) = self.document() else {
            return false;
        };
        self.category(*self.active_category())
            .is_some_and(|category| {
                category.key == "visual"
                    && category.items.iter().any(|entry| {
                        let InspectorListItem::Item(item) = entry else {
                            return false;
                        };
                        self.rust()
                            .list_state
                            .expanded(&document.target, &item.presentation.key)
                            && crate::graph_backend::has_transform_controls(&item.section)
                    })
            })
    }

    fn cached_analysis_control(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> Option<&shrimply_inspector_core::AnalysisControlPresentation> {
        let key = analysis_control_key(category, item, control)?;
        let document = self.document()?;
        let control = self.control(category, item, control)?;
        let cached = self.rust().analysis_controls.get(&key)?;
        (control.kind == ControlKind::Analysis
            && control.action == Some(cached.action)
            && cached.target == document.target)
            .then_some(&cached.presentation)
    }

    fn perform(mut self: Pin<&mut Self>, action: Option<InspectorAction>) {
        let Some(action) = action else {
            return;
        };
        let Some(target) = self.document().map(|document| document.target.clone()) else {
            return;
        };
        self.as_mut()
            .finish_confirmation(super::perform_action(&target, action));
    }

    pub(crate) fn finish_confirmation(
        mut self: Pin<&mut Self>,
        result: Result<Option<String>, String>,
    ) {
        match result {
            Ok(Some(message)) => self.as_mut().show_confirmation(QString::from(message)),
            Ok(None) => {}
            Err(error) => self.as_mut().show_error(QString::from(error)),
        }
    }

    pub(crate) fn finish(mut self: Pin<&mut Self>, result: Result<(), String>) {
        if let Err(error) = result {
            self.as_mut().show_error(QString::from(error));
        }
    }

    fn document(&self) -> Option<&InspectorDocument> {
        self.rust().document.as_ref()
    }

    fn category(&self, category: i32) -> Option<&crate::list::InspectorCategory> {
        self.document()?.categories.get(index(category)?)
    }

    fn item(&self, category: i32, item: i32) -> Option<&InspectorListItem> {
        self.category(category)?.items.get(index(item)?)
    }

    fn card(&self, category: i32, item: i32) -> Option<&crate::item::InspectorItem> {
        match self.item(category, item)? {
            InspectorListItem::Item(item) => Some(item.as_ref()),
            InspectorListItem::Flat(_) => None,
        }
    }

    fn section(&self, category: i32, item: i32) -> Option<&crate::section::InspectorSection> {
        match self.item(category, item)? {
            InspectorListItem::Item(item) => Some(&item.section),
            InspectorListItem::Flat(section) => Some(section),
        }
    }

    pub(crate) fn control(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> Option<&InspectorControl> {
        self.section(category, item)?.controls.get(index(control)?)
    }

    pub(crate) fn control_target(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> Option<(shrimply_inspector_core::InspectorTarget, InspectorControl)> {
        Some((
            self.document()?.target.clone(),
            self.control(category, item, control)?.clone(),
        ))
    }

    fn action(&self, category: i32, item: i32, action: i32) -> Option<&crate::item::HeaderAction> {
        self.card(category, item)?.actions.get(index(action)?)
    }
}

fn index(index: i32) -> Option<usize> {
    usize::try_from(index).ok()
}

fn analysis_control_key(category: i32, item: i32, control: i32) -> Option<AnalysisControlKey> {
    Some((index(category)?, index(item)?, index(control)?))
}

fn count(value: Option<usize>) -> i32 {
    value
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn strings(values: Option<&[String]>) -> QStringList {
    values
        .unwrap_or_default()
        .iter()
        .map(|value| QString::from(value.as_str()))
        .collect()
}

fn time_parts(time: shrimply_project_document::project::Time) -> [i64; 2] {
    [
        shrimply_property_model::timeline_value::fraction_numerator(time.seconds),
        shrimply_property_model::timeline_value::fraction_denominator(time.seconds),
    ]
}

pub(crate) fn time_text(time: shrimply_project_document::project::Time) -> String {
    let [numerator, denominator] = time_parts(time);
    format!("{numerator}/{denominator}")
}
