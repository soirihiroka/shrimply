use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QColor, QString, QStringList};
use shrimply_components_core::{color, layered, number, project_settings, selector, text};
use shrimply_math_color::Color;
use shrimply_math_core::{
    Fraction, fraction_as_f64, fraction_as_label, fraction_denominator, fraction_from_f64,
    fraction_is_finite, fraction_new, fraction_numerator,
};
use shrimply_project_types::{
    COMMON_FRAME_RATES, CanvasSize, MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION, PROJECT_PRESETS,
};
use std::cell::RefCell;

thread_local! {
    static RECENT_COLORS: RefCell<Vec<Color<u8>>> = RefCell::new(load_recent_colors());
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("drag_input.h");
        #[namespace = "shrimply"]
        fn register_drag_input();
        type QQuickItem;
        #[namespace = "shrimply"]
        unsafe fn activate_combo_index(control: *mut QQuickItem, index: i32);

        include!("frame_graph.h");
        #[namespace = "shrimply"]
        fn force_component_opengl();

        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("color_settings.h");
        #[namespace = "shrimply"]
        fn load_recent_colors() -> QStringList;
        #[namespace = "shrimply"]
        fn save_recent_colors(colors: &QStringList);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(f64, value)]
        #[qproperty(QString, display_text, cxx_name = "displayText")]
        #[qproperty(bool, editing)]
        #[qproperty(f64, drag_threshold, cxx_name = "dragThreshold")]
        type NumberInputBackend = super::NumberInputBackendRust;

        #[qinvokable]
        fn configure(
            self: Pin<&mut NumberInputBackend>,
            value: f64,
            minimum: f64,
            maximum: f64,
            step: f64,
            drag_pixels: f64,
            digits: i32,
        );
        #[qinvokable]
        #[cxx_name = "setExternalValue"]
        fn set_external_value(self: Pin<&mut NumberInputBackend>, value: f64);
        #[qinvokable]
        #[cxx_name = "setExternalFraction"]
        fn set_external_fraction(
            self: Pin<&mut NumberInputBackend>,
            numerator: &QString,
            denominator: &QString,
        );
        #[qinvokable]
        #[cxx_name = "beginEdit"]
        fn begin_edit(self: Pin<&mut NumberInputBackend>);
        #[qinvokable]
        #[cxx_name = "previewText"]
        fn preview_text(self: Pin<&mut NumberInputBackend>, value: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "commitText"]
        fn commit_text(self: Pin<&mut NumberInputBackend>, value: &QString);
        #[qinvokable]
        #[cxx_name = "beginDrag"]
        fn begin_drag(self: Pin<&mut NumberInputBackend>);
        #[qinvokable]
        fn drag(self: Pin<&mut NumberInputBackend>, offset: f64);
        #[qinvokable]
        #[cxx_name = "endDrag"]
        fn end_drag(self: Pin<&mut NumberInputBackend>);

        #[qsignal]
        fn edited(self: Pin<&mut NumberInputBackend>, value: f64);
        #[qsignal]
        fn committed(self: Pin<&mut NumberInputBackend>, value: f64);
        #[qsignal]
        #[cxx_name = "fractionEdited"]
        fn fraction_edited(self: Pin<&mut NumberInputBackend>, numerator: i64, denominator: i64);
        #[qsignal]
        #[cxx_name = "fractionCommitted"]
        fn fraction_committed(self: Pin<&mut NumberInputBackend>, numerator: i64, denominator: i64);

        #[qobject]
        #[qml_element]
        #[qproperty(f64, first)]
        #[qproperty(f64, second)]
        #[qproperty(f64, third)]
        #[qproperty(bool, locked)]
        type NumberGroupBackend = super::NumberGroupBackendRust;

        #[qinvokable]
        fn configure(
            self: Pin<&mut NumberGroupBackend>,
            first: f64,
            second: f64,
            third: f64,
            dimensions: i32,
            locked: bool,
        );
        #[qinvokable]
        #[cxx_name = "setBounds"]
        fn set_group_bounds(self: Pin<&mut NumberGroupBackend>, minimum: f64, maximum: f64);
        #[qinvokable]
        fn edit(self: Pin<&mut NumberGroupBackend>, axis: i32, value: f64);
        #[qinvokable]
        #[cxx_name = "setExternalValue"]
        fn set_group_external_value(self: Pin<&mut NumberGroupBackend>, axis: i32, value: f64);
        #[qinvokable]
        #[cxx_name = "updateLocked"]
        fn update_lock(self: Pin<&mut NumberGroupBackend>, locked: bool);

        #[qsignal]
        #[cxx_name = "valueEdited"]
        fn group_value_edited(self: Pin<&mut NumberGroupBackend>, axis: i32, value: f64);
        #[qsignal]
        #[cxx_name = "groupEdited"]
        fn group_edited(self: Pin<&mut NumberGroupBackend>, axis: i32);

        #[qobject]
        #[qml_element]
        #[qproperty(QColor, color)]
        #[qproperty(QColor, draft)]
        #[qproperty(QString, hex)]
        #[qproperty(f32, hue)]
        #[qproperty(f32, saturation)]
        #[qproperty(f32, brightness)]
        #[qproperty(f32, alpha)]
        #[qproperty(bool, with_alpha, cxx_name = "withAlpha")]
        #[qproperty(bool, screen_picking, cxx_name = "screenPicking")]
        #[qproperty(i32, palette_count, cxx_name = "paletteCount")]
        #[qproperty(i32, recent_count, cxx_name = "recentCount")]
        type ColorPickerBackend = super::ColorPickerBackendRust;

        #[qinvokable]
        fn configure(self: Pin<&mut ColorPickerBackend>, color: &QColor, with_alpha: bool);
        #[qinvokable]
        #[cxx_name = "setHsva"]
        fn set_hsva(
            self: Pin<&mut ColorPickerBackend>,
            hue: f32,
            saturation: f32,
            brightness: f32,
            alpha: f32,
        );
        #[qinvokable]
        #[cxx_name = "applyHex"]
        fn apply_hex(self: Pin<&mut ColorPickerBackend>, value: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "paletteColor"]
        fn palette_color(self: &ColorPickerBackend, index: i32) -> QColor;
        #[qinvokable]
        #[cxx_name = "paletteLabel"]
        fn palette_label(self: &ColorPickerBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "recentColor"]
        fn recent_color(self: &ColorPickerBackend, index: i32) -> QColor;
        #[qinvokable]
        #[cxx_name = "recentLabel"]
        fn recent_label(self: &ColorPickerBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "chooseColor"]
        fn choose_color(self: Pin<&mut ColorPickerBackend>, color: &QColor);
        #[qinvokable]
        #[cxx_name = "pickScreenColor"]
        fn pick_screen_color(self: Pin<&mut ColorPickerBackend>);
        #[qinvokable]
        fn confirm(self: Pin<&mut ColorPickerBackend>);

        #[qsignal]
        fn selected(self: Pin<&mut ColorPickerBackend>, color: QColor);
        #[qsignal]
        #[cxx_name = "screenColorFailed"]
        fn screen_color_failed(self: Pin<&mut ColorPickerBackend>, message: QString);

        #[qobject]
        #[qml_element]
        #[qproperty(QString, text)]
        #[qproperty(bool, dirty)]
        #[qproperty(i32, typo_count, cxx_name = "typoCount")]
        #[qproperty(QString, typo_ranges, cxx_name = "typoRanges")]
        type TextInputBackend = super::TextInputBackendRust;

        #[qinvokable]
        fn configure(self: Pin<&mut TextInputBackend>, value: &QString, maximum_length: i32);
        #[qinvokable]
        fn edit(self: Pin<&mut TextInputBackend>, value: &QString) -> QString;
        #[qinvokable]
        fn commit(self: Pin<&mut TextInputBackend>);
        #[qinvokable]
        #[cxx_name = "typoMessage"]
        fn typo_message(self: &TextInputBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "typoAt"]
        fn typo_at(self: &TextInputBackend, utf16_offset: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "typoStart"]
        fn typo_start(self: &TextInputBackend, index: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "typoLength"]
        fn typo_length(self: &TextInputBackend, index: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "typoCorrection"]
        fn typo_correction(self: &TextInputBackend, typo: i32, correction: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "typoCorrectionCount"]
        fn typo_correction_count(self: &TextInputBackend, typo: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "applyCorrection"]
        fn apply_correction(
            self: Pin<&mut TextInputBackend>,
            typo: i32,
            correction: i32,
        ) -> QString;

        #[qsignal]
        fn changed(self: Pin<&mut TextInputBackend>, value: QString);
        #[qsignal]
        fn committed(self: Pin<&mut TextInputBackend>, value: QString);

        #[qobject]
        #[qml_element]
        type SelectorBackend = super::SelectorBackendRust;

        #[qinvokable]
        #[cxx_name = "selectedIndex"]
        fn selected_index(self: &SelectorBackend, values: &QStringList, value: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "matchingIndex"]
        fn matching_index(self: &SelectorBackend, values: &QStringList, value: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "matchesQuery"]
        fn matches_query(self: &SelectorBackend, label: &QString, query: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "nextMatchingIndex"]
        fn next_matching_index(
            self: &SelectorBackend,
            labels: &QStringList,
            query: &QString,
            current: i32,
            direction: i32,
        ) -> i32;
        #[qinvokable]
        #[cxx_name = "rankedMatchingIndices"]
        fn ranked_matching_indices(
            self: &SelectorBackend,
            labels: &QStringList,
            keyword_groups: &QStringList,
            query: &QString,
        ) -> QStringList;
        #[qinvokable]
        #[cxx_name = "valueAt"]
        fn value_at(self: &SelectorBackend, values: &QStringList, index: i32) -> QString;
        #[qinvokable]
        fn searchable(self: &SelectorBackend, count: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "menuUsesBottomSheet"]
        fn menu_uses_bottom_sheet(self: &SelectorBackend, touch: bool) -> bool;
        #[qinvokable]
        #[cxx_name = "activateComboIndex"]
        unsafe fn activate_combo(self: &SelectorBackend, control: *mut QQuickItem, index: i32);

        #[qobject]
        #[qml_element]
        #[qproperty(i32, preset)]
        #[qproperty(i32, width)]
        #[qproperty(i32, height)]
        #[qproperty(i32, frame_rate, cxx_name = "frameRate")]
        #[qproperty(i32, preset_count, cxx_name = "presetCount")]
        #[qproperty(i32, frame_rate_count, cxx_name = "frameRateCount")]
        #[qproperty(bool, changed)]
        type ProjectSettingsBackend = super::ProjectSettingsBackendRust;

        #[qinvokable]
        fn discard(self: Pin<&mut ProjectSettingsBackend>);
        #[qinvokable]
        #[cxx_name = "settingsText"]
        fn settings_text(self: &ProjectSettingsBackend, field: &QString) -> QString;
        #[qinvokable]
        fn configure(
            self: Pin<&mut ProjectSettingsBackend>,
            width: i32,
            height: i32,
            fps_numerator: &QString,
            fps_denominator: &QString,
        );
        #[qinvokable]
        #[cxx_name = "presetLabel"]
        fn preset_label(self: &ProjectSettingsBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "frameRateLabel"]
        fn frame_rate_label(self: &ProjectSettingsBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "frameRateValues"]
        fn frame_rate_values(self: &ProjectSettingsBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "frameRateLabels"]
        fn frame_rate_labels(self: &ProjectSettingsBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "selectPreset"]
        fn select_preset(self: Pin<&mut ProjectSettingsBackend>, index: i32);
        #[qinvokable]
        #[cxx_name = "setWidthValue"]
        fn set_width_value(self: Pin<&mut ProjectSettingsBackend>, width: i32);
        #[qinvokable]
        #[cxx_name = "setHeightValue"]
        fn set_height_value(self: Pin<&mut ProjectSettingsBackend>, height: i32);
        #[qinvokable]
        #[cxx_name = "setFrameRateValue"]
        fn set_frame_rate_value(self: Pin<&mut ProjectSettingsBackend>, index: i32);
        #[qinvokable]
        #[cxx_name = "fpsNumerator"]
        fn fps_numerator(self: &ProjectSettingsBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "fpsDenominator"]
        fn fps_denominator(self: &ProjectSettingsBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "minimumDimension"]
        fn minimum_dimension(self: &ProjectSettingsBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "maximumDimension"]
        fn maximum_dimension(self: &ProjectSettingsBackend) -> i32;

        #[qobject]
        #[qml_element]
        #[qproperty(QStringList, titles)]
        #[qproperty(QStringList, subtitles)]
        type LivePerformanceBackend = super::LivePerformanceBackendRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut LivePerformanceBackend>);
        #[qinvokable]
        #[cxx_name = "refreshInterval"]
        fn refresh_interval(self: &LivePerformanceBackend) -> i32;
        #[qinvokable]
        fn clear(self: Pin<&mut LivePerformanceBackend>);
        #[qinvokable]
        #[cxx_name = "reportJson"]
        fn report_json(self: &LivePerformanceBackend) -> QString;

        #[qobject]
        #[qml_element]
        #[qproperty(i32, active_component, cxx_name = "activeComponent")]
        type LayeredPropertyBackend = super::LayeredPropertyBackendRust;

        #[qinvokable]
        #[cxx_name = "setModes"]
        fn set_modes(self: Pin<&mut LayeredPropertyBackend>, keyframes: bool, expression: bool);
        #[qinvokable]
        #[cxx_name = "editValue"]
        fn edit_layered_value(self: Pin<&mut LayeredPropertyBackend>, value: f64);
        #[qinvokable]
        #[cxx_name = "editPair"]
        fn edit_layered_pair(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            component: i32,
        );
        #[qinvokable]
        #[cxx_name = "editTriple"]
        fn edit_layered_triple(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            third: f64,
            component: i32,
        );
        #[qinvokable]
        #[cxx_name = "configurePair"]
        fn configure_layered_pair(self: Pin<&mut LayeredPropertyBackend>, first: f64, second: f64);
        #[qinvokable]
        #[cxx_name = "configureTriple"]
        fn configure_layered_triple(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            third: f64,
        );
        #[qinvokable]
        #[cxx_name = "selectComponent"]
        fn select_layered_component(self: Pin<&mut LayeredPropertyBackend>, component: i32);
        #[qinvokable]
        #[cxx_name = "selectTripleComponent"]
        fn select_layered_triple_component(self: Pin<&mut LayeredPropertyBackend>, component: i32);

        #[qsignal]
        #[cxx_name = "baseEdited"]
        fn base_edited(self: Pin<&mut LayeredPropertyBackend>, value: f64);
        #[qsignal]
        #[cxx_name = "keyframeEdited"]
        fn keyframe_edited(self: Pin<&mut LayeredPropertyBackend>, value: f64);
        #[qsignal]
        #[cxx_name = "basePairEdited"]
        fn base_pair_edited(self: Pin<&mut LayeredPropertyBackend>, first: f64, second: f64);
        #[qsignal]
        #[cxx_name = "keyframePairEdited"]
        fn keyframe_pair_edited(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            component: i32,
            first_changed: bool,
            second_changed: bool,
        );
        #[qsignal]
        #[cxx_name = "baseTripleEdited"]
        fn base_triple_edited(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            third: f64,
        );
        #[qsignal]
        #[cxx_name = "keyframeTripleEdited"]
        fn keyframe_triple_edited(
            self: Pin<&mut LayeredPropertyBackend>,
            first: f64,
            second: f64,
            third: f64,
            component: i32,
        );

        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type ComponentTranslations = super::ComponentTranslationsRust;

        #[qinvokable]
        fn text(self: &ComponentTranslations, key: &QString) -> QString;
    }

    impl cxx_qt::Initialize for NumberInputBackend {}
    impl cxx_qt::Initialize for NumberGroupBackend {}
    impl cxx_qt::Initialize for ColorPickerBackend {}
    impl cxx_qt::Threading for ColorPickerBackend {}
    impl cxx_qt::Initialize for TextInputBackend {}
    impl cxx_qt::Initialize for SelectorBackend {}
    impl cxx_qt::Initialize for ProjectSettingsBackend {}
    impl cxx_qt::Initialize for LivePerformanceBackend {}
    impl cxx_qt::Initialize for LayeredPropertyBackend {}
    impl cxx_qt::Initialize for ComponentTranslations {}
}

pub struct NumberInputBackendRust {
    value: f64,
    display_text: QString,
    editing: bool,
    drag_threshold: f64,
    config: number::NumberConfig,
    accepted: Fraction,
    drag_start: Fraction,
    drag_moved: bool,
}

impl Default for NumberInputBackendRust {
    fn default() -> Self {
        let accepted = fraction_from_f64(0.0);
        let config = number::NumberConfig::new(accepted);
        Self {
            value: 0.0,
            display_text: QString::from(number::format_value(&config, accepted)),
            editing: false,
            drag_threshold: number::DRAG_THRESHOLD_PIXELS,
            config,
            accepted,
            drag_start: accepted,
            drag_moved: false,
        }
    }
}

impl cxx_qt::Initialize for qobject::NumberInputBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::NumberInputBackend {
    pub fn configure(
        mut self: Pin<&mut Self>,
        value: f64,
        minimum: f64,
        maximum: f64,
        step: f64,
        drag_pixels: f64,
        digits: i32,
    ) {
        let fallback = fraction_from_f64(value);
        let config = number::NumberConfig {
            minimum: fraction_from_f64(minimum),
            maximum: fraction_from_f64(maximum),
            drag_step: number::positive_fraction_or(
                fraction_from_f64(step),
                fraction_from_f64(1.0),
            ),
            drag_pixels: if drag_pixels > 0.0 {
                drag_pixels
            } else {
                number::DEFAULT_DRAG_PIXELS
            },
            digits: usize::try_from(digits.max(0)).unwrap_or_default(),
            fallback: number::finite_fraction_or(fallback, fraction_from_f64(0.0)),
        };
        self.as_mut().rust_mut().config = config;
        self.set_accepted(fallback, false, false);
    }

    pub fn set_external_value(mut self: Pin<&mut Self>, value: f64) {
        if !self.editing() {
            self.as_mut()
                .set_accepted(fraction_from_f64(value), false, false);
        }
    }

    pub fn set_external_fraction(
        mut self: Pin<&mut Self>,
        numerator: &QString,
        denominator: &QString,
    ) {
        let (Ok(numerator), Ok(denominator)) = (
            numerator.to_string().parse::<i64>(),
            denominator.to_string().parse::<i64>(),
        ) else {
            return;
        };
        let value = fraction_new(numerator, denominator);
        if fraction_is_finite(value) && !self.editing() {
            self.as_mut().set_accepted(value, false, false);
        }
    }

    pub fn begin_edit(mut self: Pin<&mut Self>) {
        self.as_mut().set_editing(true);
        let text = number::format_value(&self.rust().config, self.rust().accepted);
        self.as_mut().set_display_text(QString::from(text));
    }

    pub fn preview_text(mut self: Pin<&mut Self>, value: &QString) -> bool {
        let Some(value) = number::parse_fraction(value.to_string().trim()) else {
            return false;
        };
        let value = number::accepted_value(&self.rust().config, value);
        self.as_mut().edited(fraction_as_f64(value));
        self.as_mut()
            .fraction_edited(fraction_numerator(value), fraction_denominator(value));
        true
    }

    pub fn commit_text(mut self: Pin<&mut Self>, value: &QString) {
        if let Some(value) = number::parse_fraction(value.to_string().trim()) {
            self.as_mut().set_accepted(value, true, true);
        } else {
            let accepted = self.rust().accepted;
            self.as_mut().set_accepted(accepted, false, false);
        }
        self.as_mut().set_editing(false);
    }

    pub fn begin_drag(mut self: Pin<&mut Self>) {
        let accepted = self.rust().accepted;
        let rust = self.as_mut().rust_mut().get_mut();
        rust.drag_start = accepted;
        rust.drag_moved = false;
    }

    pub fn drag(mut self: Pin<&mut Self>, offset: f64) {
        if offset.abs() < number::DRAG_THRESHOLD_PIXELS && !self.rust().drag_moved {
            return;
        }
        self.as_mut().rust_mut().drag_moved = true;
        let value = number::dragged_value(&self.rust().config, self.rust().drag_start, offset);
        self.as_mut().set_accepted(value, true, false);
    }

    pub fn end_drag(mut self: Pin<&mut Self>) {
        if self.rust().drag_moved {
            let value = self.rust().accepted;
            self.as_mut().committed(fraction_as_f64(value));
            self.as_mut()
                .fraction_committed(fraction_numerator(value), fraction_denominator(value));
        }
    }

    fn set_accepted(mut self: Pin<&mut Self>, value: Fraction, notify: bool, commit: bool) {
        let value = number::accepted_value(&self.rust().config, value);
        let changed = self.rust().accepted != value;
        self.as_mut().rust_mut().accepted = value;
        let numeric = fraction_as_f64(value);
        self.as_mut().set_value(numeric);
        let display = number::format_value(&self.rust().config, value);
        self.as_mut().set_display_text(QString::from(display));
        if changed && notify {
            self.as_mut().edited(numeric);
            self.as_mut()
                .fraction_edited(fraction_numerator(value), fraction_denominator(value));
        }
        if commit && changed {
            self.as_mut().committed(numeric);
            self.as_mut()
                .fraction_committed(fraction_numerator(value), fraction_denominator(value));
        }
    }
}

pub struct NumberGroupBackendRust {
    first: f64,
    second: f64,
    third: f64,
    locked: bool,
    dimensions: usize,
    values: [Fraction; 3],
    ratios: [Fraction; 2],
    minimum: Fraction,
    maximum: Fraction,
}

impl Default for NumberGroupBackendRust {
    fn default() -> Self {
        let values = [fraction_from_f64(0.0); 3];
        Self {
            first: 0.0,
            second: 0.0,
            third: 0.0,
            locked: false,
            dimensions: 2,
            values,
            ratios: number::triple_ratios(values),
            minimum: shrimply_math_core::fraction_from_integer(number::DEFAULT_MINIMUM),
            maximum: shrimply_math_core::fraction_from_integer(number::DEFAULT_MAXIMUM),
        }
    }
}

impl cxx_qt::Initialize for qobject::NumberGroupBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::NumberGroupBackend {
    pub fn configure(
        mut self: Pin<&mut Self>,
        first: f64,
        second: f64,
        third: f64,
        dimensions: i32,
        locked: bool,
    ) {
        let values = [
            fraction_from_f64(first),
            fraction_from_f64(second),
            fraction_from_f64(third),
        ]
        .map(|value| number::clamped_value(value, self.rust().minimum, self.rust().maximum));
        let rust = self.as_mut().rust_mut().get_mut();
        rust.values = values;
        rust.dimensions = usize::try_from(dimensions.clamp(2, 3)).unwrap_or(2);
        rust.ratios = group_ratios(rust.dimensions, values);
        self.as_mut().update_lock(locked);
        self.as_mut().publish_values();
    }

    pub fn edit(mut self: Pin<&mut Self>, axis: i32, value: f64) {
        let Ok(axis) = usize::try_from(axis) else {
            return;
        };
        if axis >= self.rust().dimensions {
            return;
        }
        let previous = self.rust().values;
        let value = number::clamped_value(
            fraction_from_f64(value),
            self.rust().minimum,
            self.rust().maximum,
        );
        if *self.locked() {
            let values = if self.rust().dimensions == 2 {
                let pair = number::locked_pair(axis, value, self.rust().ratios[0]);
                [pair[0], pair[1], self.rust().values[2]]
            } else {
                number::locked_triple(axis, value, self.rust().ratios)
            }
            .map(|value| number::clamped_value(value, self.rust().minimum, self.rust().maximum));
            self.as_mut().rust_mut().values = values;
        } else {
            self.as_mut().rust_mut().values[axis] = value;
        }
        self.as_mut().publish_values();
        let order = match axis {
            0 => [0, 1, 2],
            1 => [1, 0, 2],
            _ => [2, 0, 1],
        };
        for changed_axis in order.into_iter().take(self.rust().dimensions) {
            let value = self.rust().values[changed_axis];
            if value != previous[changed_axis] {
                self.as_mut().group_value_edited(
                    i32::try_from(changed_axis).unwrap_or_default(),
                    fraction_as_f64(value),
                );
            }
        }
        self.as_mut()
            .group_edited(i32::try_from(axis).unwrap_or_default());
    }

    pub fn set_group_bounds(mut self: Pin<&mut Self>, minimum: f64, maximum: f64) {
        let minimum = fraction_from_f64(minimum);
        let maximum = fraction_from_f64(maximum);
        let values = self
            .rust()
            .values
            .map(|value| number::clamped_value(value, minimum, maximum));
        let rust = self.as_mut().rust_mut().get_mut();
        rust.minimum = minimum;
        rust.maximum = maximum;
        rust.values = values;
        rust.ratios = group_ratios(rust.dimensions, values);
        self.as_mut().publish_values();
    }

    pub fn set_group_external_value(mut self: Pin<&mut Self>, axis: i32, value: f64) {
        let Ok(axis) = usize::try_from(axis) else {
            return;
        };
        if axis >= self.rust().dimensions {
            return;
        }
        let value = number::clamped_value(
            fraction_from_f64(value),
            self.rust().minimum,
            self.rust().maximum,
        );
        if self.rust().values[axis] == value {
            return;
        }
        self.as_mut().rust_mut().values[axis] = value;
        let values = self.rust().values;
        let dimensions = self.rust().dimensions;
        self.as_mut().rust_mut().ratios = group_ratios(dimensions, values);
        self.as_mut().publish_values();
    }

    pub fn update_lock(mut self: Pin<&mut Self>, locked: bool) {
        if locked && !self.locked() {
            let values = self.rust().values;
            let dimensions = self.rust().dimensions;
            self.as_mut().rust_mut().ratios = group_ratios(dimensions, values);
        }
        self.as_mut().set_locked(locked);
    }

    fn publish_values(mut self: Pin<&mut Self>) {
        let values = self.rust().values.map(fraction_as_f64);
        self.as_mut().set_first(values[0]);
        self.as_mut().set_second(values[1]);
        self.as_mut().set_third(values[2]);
    }
}

pub struct ColorPickerBackendRust {
    color: QColor,
    draft: QColor,
    hex: QString,
    hue: f32,
    saturation: f32,
    brightness: f32,
    alpha: f32,
    with_alpha: bool,
    screen_picking: bool,
    palette_count: i32,
    recent_count: i32,
    hsva: color::Hsva,
}

impl Default for ColorPickerBackendRust {
    fn default() -> Self {
        let value = Color::BLACK;
        Self {
            color: qcolor(value),
            draft: qcolor(value),
            hex: QString::from(color::color_hex(value, true)),
            hue: 0.0,
            saturation: 0.0,
            brightness: 0.0,
            alpha: 1.0,
            with_alpha: true,
            screen_picking: false,
            palette_count: color::PALETTE.len() as i32,
            recent_count: 0,
            hsva: color::Hsva::from_color(value),
        }
    }
}

impl cxx_qt::Initialize for qobject::ColorPickerBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::ColorPickerBackend {
    pub fn configure(mut self: Pin<&mut Self>, value: &QColor, with_alpha: bool) {
        let mut value = color_from_qcolor(value);
        if !with_alpha {
            value.a = u8::MAX;
        }
        self.as_mut().set_with_alpha(with_alpha);
        let recent_count = RECENT_COLORS.with_borrow(|colors| colors.len() as i32);
        self.as_mut().set_recent_count(recent_count);
        self.as_mut().set_color(qcolor(value));
        self.as_mut().publish_hsva(color::Hsva::from_color(value));
    }

    pub fn set_hsva(
        mut self: Pin<&mut Self>,
        hue: f32,
        saturation: f32,
        brightness: f32,
        alpha: f32,
    ) {
        let hsva = color::Hsva {
            hue: hue.clamp(0.0, 360.0),
            saturation: saturation.clamp(0.0, 1.0),
            value: brightness.clamp(0.0, 1.0),
            alpha: if *self.with_alpha() {
                alpha.clamp(0.0, 1.0)
            } else {
                1.0
            },
        };
        self.as_mut().publish_hsva(hsva);
    }

    pub fn apply_hex(mut self: Pin<&mut Self>, value: &QString) -> bool {
        let Some(value) = color::parse_hex(&value.to_string(), *self.with_alpha()) else {
            return false;
        };
        self.as_mut().publish_color(value);
        true
    }

    pub fn palette_color(&self, index: i32) -> QColor {
        index_of(index)
            .and_then(|index| color::PALETTE.get(index))
            .map_or_else(QColor::default, |(_, value)| qcolor(*value))
    }

    pub fn palette_label(&self, index: i32) -> QString {
        index_of(index)
            .and_then(|index| color::PALETTE.get(index))
            .map_or_else(QString::default, |(label, _)| shrimply_i18n_qt::text(label))
    }

    pub fn recent_color(&self, index: i32) -> QColor {
        RECENT_COLORS.with_borrow(|colors| {
            index_of(index)
                .and_then(|index| colors.get(index))
                .copied()
                .map_or_else(QColor::default, qcolor)
        })
    }

    pub fn recent_label(&self, index: i32) -> QString {
        RECENT_COLORS.with_borrow(|colors| {
            index_of(index)
                .and_then(|index| colors.get(index))
                .map_or_else(QString::default, |value| {
                    QString::from(color::color_hex(*value, true))
                })
        })
    }

    pub fn choose_color(mut self: Pin<&mut Self>, value: &QColor) {
        let mut value = color_from_qcolor(value);
        if !self.with_alpha() {
            value.a = u8::MAX;
        }
        self.as_mut().publish_color(value);
    }

    pub fn pick_screen_color(mut self: Pin<&mut Self>) {
        if *self.screen_picking() {
            return;
        }
        self.as_mut().set_screen_picking(true);
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = shrimply_cross_ui_core::screen_color::pick_blocking();
            let _ = qt_thread.queue(move |mut backend| {
                backend.as_mut().set_screen_picking(false);
                match result {
                    Ok([red, green, blue]) => {
                        backend.as_mut().publish_color(Color::from_srgba([
                            red as f32,
                            green as f32,
                            blue as f32,
                            1.0,
                        ]));
                    }
                    Err(error) => backend.as_mut().screen_color_failed(QString::from(error)),
                }
            });
        });
    }

    pub fn confirm(mut self: Pin<&mut Self>) {
        let value = self.rust().hsva.color();
        let count = RECENT_COLORS.with_borrow_mut(|colors| {
            color::remember_color(colors, value);
            save_recent_colors(colors);
            colors.len() as i32
        });
        self.as_mut().set_recent_count(count);
        let selected = qcolor(value);
        let changed = color_from_qcolor(self.color()) != value;
        self.as_mut().set_color(selected.clone());
        if changed {
            self.as_mut().selected(selected);
        }
    }

    fn publish_color(mut self: Pin<&mut Self>, value: Color<u8>) {
        let hsva = self.rust().hsva.update_color(value);
        self.as_mut().publish_hsva(hsva);
    }

    fn publish_hsva(mut self: Pin<&mut Self>, hsva: color::Hsva) {
        let value = hsva.color();
        self.as_mut().rust_mut().hsva = hsva;
        self.as_mut().set_hue(hsva.hue);
        self.as_mut().set_saturation(hsva.saturation);
        self.as_mut().set_brightness(hsva.value);
        self.as_mut().set_alpha(hsva.alpha);
        self.as_mut().set_draft(qcolor(value));
        let with_alpha = *self.with_alpha();
        self.as_mut()
            .set_hex(QString::from(color::color_hex(value, with_alpha)));
    }
}

fn load_recent_colors() -> Vec<Color<u8>> {
    qobject::load_recent_colors()
        .iter()
        .filter_map(|value| color::parse_hex(&value.to_string(), true))
        .take(color::RECENT_LIMIT)
        .collect()
}

fn save_recent_colors(colors: &[Color<u8>]) {
    let values = colors
        .iter()
        .map(|value| QString::from(color::color_hex(*value, true)))
        .collect::<QStringList>();
    qobject::save_recent_colors(&values);
}

#[derive(Default)]
pub struct TextInputBackendRust {
    text: QString,
    dirty: bool,
    typo_count: i32,
    typo_ranges: QString,
    maximum_length: Option<usize>,
    marks: Vec<text::TypoMark>,
}

impl cxx_qt::Initialize for qobject::TextInputBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::TextInputBackend {
    pub fn configure(mut self: Pin<&mut Self>, value: &QString, maximum_length: i32) {
        let maximum_length = usize::try_from(maximum_length)
            .ok()
            .filter(|value| *value > 0);
        self.as_mut().rust_mut().maximum_length = maximum_length;
        self.as_mut().set_text_value(
            text::limited_text(&value.to_string(), maximum_length),
            false,
        );
    }

    pub fn edit(mut self: Pin<&mut Self>, value: &QString) -> QString {
        let value = text::limited_text(&value.to_string(), self.rust().maximum_length);
        if value == self.text().to_string() {
            return QString::from(value);
        }
        self.as_mut().set_text_value(value.clone(), true);
        self.as_mut().changed(QString::from(value.clone()));
        QString::from(value)
    }

    pub fn commit(mut self: Pin<&mut Self>) {
        if *self.dirty() {
            self.as_mut().set_dirty(false);
            let value = self.text().clone();
            self.as_mut().committed(value);
        }
    }

    pub fn typo_message(&self, index: i32) -> QString {
        self.mark(index)
            .map_or_else(QString::default, |mark| QString::from(&mark.message))
    }

    pub fn typo_at(&self, utf16_offset: i32) -> i32 {
        let utf16_offset = usize::try_from(utf16_offset).unwrap_or_default();
        let text = self.text().to_string();
        let mut remaining = utf16_offset;
        let char_offset = text
            .chars()
            .take_while(|character| {
                let width = character.len_utf16();
                if remaining < width {
                    false
                } else {
                    remaining -= width;
                    true
                }
            })
            .count();
        self.rust()
            .marks
            .iter()
            .position(|mark| {
                let start = usize::try_from(mark.start).unwrap_or_default();
                let end = usize::try_from(mark.end).unwrap_or_default();
                char_offset >= start && char_offset < end
            })
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    }

    pub fn typo_correction(&self, typo: i32, correction: i32) -> QString {
        self.mark(typo)
            .and_then(|mark| index_of(correction).and_then(|index| mark.corrections.get(index)))
            .map_or_else(QString::default, QString::from)
    }

    pub fn typo_correction_count(&self, typo: i32) -> i32 {
        self.mark(typo)
            .and_then(|mark| i32::try_from(mark.corrections.len()).ok())
            .unwrap_or_default()
    }

    pub fn typo_start(&self, index: i32) -> i32 {
        self.utf16_mark(index).map_or(-1, |(start, _)| start)
    }

    pub fn typo_length(&self, index: i32) -> i32 {
        self.utf16_mark(index).map_or(0, |(_, length)| length)
    }

    pub fn apply_correction(mut self: Pin<&mut Self>, typo: i32, correction: i32) -> QString {
        let Some(mark) = self.mark(typo).cloned() else {
            return self.text().clone();
        };
        let Some(correction) = index_of(correction)
            .and_then(|index| mark.corrections.get(index))
            .cloned()
        else {
            return self.text().clone();
        };
        let mut chars = self.text().to_string().chars().collect::<Vec<_>>();
        let start = usize::try_from(mark.start)
            .unwrap_or_default()
            .min(chars.len());
        let end = usize::try_from(mark.end)
            .unwrap_or_default()
            .min(chars.len());
        chars.splice(start..end, correction.chars());
        let value = chars.into_iter().collect::<String>();
        self.as_mut().set_text_value(value.clone(), true);
        self.as_mut().changed(QString::from(value.clone()));
        QString::from(value)
    }

    fn mark(&self, index: i32) -> Option<&text::TypoMark> {
        index_of(index).and_then(|index| self.rust().marks.get(index))
    }

    fn utf16_mark(&self, index: i32) -> Option<(i32, i32)> {
        let mark = self.mark(index)?;
        let (start, length) = utf16_range(&self.text().to_string(), mark);
        Some((
            i32::try_from(start).unwrap_or(i32::MAX),
            i32::try_from(length).unwrap_or(i32::MAX),
        ))
    }

    fn set_text_value(mut self: Pin<&mut Self>, value: String, dirty: bool) {
        let marks = text::typo_marks(&value);
        let count = i32::try_from(marks.len()).unwrap_or(i32::MAX);
        let typo_ranges = marks
            .iter()
            .map(|mark| utf16_range(&value, mark))
            .map(|(start, length)| format!("{start}:{length}"))
            .collect::<Vec<_>>()
            .join(",");
        self.as_mut().rust_mut().marks = marks;
        self.as_mut().set_text(QString::from(value));
        self.as_mut().set_dirty(dirty);
        self.as_mut().set_typo_count(count);
        self.as_mut().set_typo_ranges(QString::from(typo_ranges));
    }
}

fn utf16_range(value: &str, mark: &text::TypoMark) -> (usize, usize) {
    let start = usize::try_from(mark.start).unwrap_or_default();
    let end = usize::try_from(mark.end).unwrap_or(start);
    (
        value.chars().take(start).map(char::len_utf16).sum(),
        value
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(char::len_utf16)
            .sum(),
    )
}

#[derive(Default)]
pub struct SelectorBackendRust;

impl cxx_qt::Initialize for qobject::SelectorBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::SelectorBackend {
    /// # Safety
    /// `control` must be a live Qt Quick ComboBox on its GUI thread.
    pub unsafe fn activate_combo(&self, control: *mut qobject::QQuickItem, index: i32) {
        unsafe { qobject::activate_combo_index(control, index) };
    }

    pub fn menu_uses_bottom_sheet(&self, touch: bool) -> bool {
        shrimply_components_core::layout::menu_uses_bottom_sheet(if touch {
            "touch"
        } else {
            "pointer"
        })
    }

    pub fn selected_index(&self, values: &QStringList, value: &QString) -> i32 {
        let choices = selector::identity_choices(values.iter().map(ToString::to_string).collect());
        i32::try_from(selector::selected_index(&value.to_string(), &choices)).unwrap_or_default()
    }

    pub fn matching_index(&self, values: &QStringList, value: &QString) -> i32 {
        let choices = selector::identity_choices(values.iter().map(ToString::to_string).collect());
        selector::matching_index(&value.to_string(), &choices)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    }

    pub fn matches_query(&self, label: &QString, query: &QString) -> bool {
        selector::matches_query(&label.to_string(), &query.to_string())
    }

    pub fn next_matching_index(
        &self,
        labels: &QStringList,
        query: &QString,
        current: i32,
        direction: i32,
    ) -> i32 {
        selector::adjacent_matching_index(
            &labels.iter().map(ToString::to_string).collect::<Vec<_>>(),
            &query.to_string(),
            usize::try_from(current).ok(),
            direction >= 0,
        )
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(-1)
    }

    pub fn ranked_matching_indices(
        &self,
        labels: &QStringList,
        keyword_groups: &QStringList,
        query: &QString,
    ) -> QStringList {
        selector::ranked_matching_indices(
            &labels.iter().map(ToString::to_string).collect::<Vec<_>>(),
            &keyword_groups
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            &query.to_string(),
        )
        .into_iter()
        .map(|index| QString::from(index.to_string()))
        .collect()
    }

    pub fn value_at(&self, values: &QStringList, index: i32) -> QString {
        isize::try_from(index)
            .ok()
            .and_then(|index| values.get(index))
            .cloned()
            .unwrap_or_default()
    }

    pub fn searchable(&self, count: i32) -> bool {
        selector::searchable(usize::try_from(count).unwrap_or_default())
    }
}

pub struct ProjectSettingsBackendRust {
    preset: i32,
    width: i32,
    height: i32,
    frame_rate: i32,
    preset_count: i32,
    frame_rate_count: i32,
    changed: bool,
    settings: project_settings::ProjectSettings,
}

impl Default for ProjectSettingsBackendRust {
    fn default() -> Self {
        let settings = project_settings::ProjectSettings::default();
        Self {
            preset: settings.preset as i32,
            width: settings.width as i32,
            height: settings.height as i32,
            frame_rate: settings.frame_rate as i32,
            preset_count: (PROJECT_PRESETS.len() + 1) as i32,
            frame_rate_count: COMMON_FRAME_RATES.len() as i32,
            changed: false,
            settings,
        }
    }
}

impl cxx_qt::Initialize for qobject::ProjectSettingsBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

#[derive(Default)]
pub struct LivePerformanceBackendRust {
    titles: QStringList,
    subtitles: QStringList,
    rows: shrimply_components_core::performance::PerformanceRows,
}

#[derive(Default)]
pub struct LayeredPropertyBackendRust {
    active_component: i32,
    controller: layered::LayeredPropertyController,
    pair: [f64; 2],
    triple: [f64; 3],
}

impl cxx_qt::Initialize for qobject::LayeredPropertyBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::LayeredPropertyBackend {
    pub fn set_modes(self: Pin<&mut Self>, keyframes: bool, expression: bool) {
        self.rust().controller.set_keyframes(keyframes);
        self.rust().controller.set_expression(expression);
    }

    pub fn edit_layered_value(mut self: Pin<&mut Self>, value: f64) {
        match self.rust().controller.edit(value) {
            layered::LayeredEdit::Base(value) => self.as_mut().base_edited(value),
            layered::LayeredEdit::Keyframe(value) => self.as_mut().keyframe_edited(value),
        }
    }

    pub fn edit_layered_pair(mut self: Pin<&mut Self>, first: f64, second: f64, component: i32) {
        let component = usize::try_from(component).expect("non-negative layered value component");
        let next = [first, second];
        let changes = layered::component_changes(self.rust().pair, next);
        self.as_mut().rust_mut().pair = next;
        self.rust().controller.select_component::<2>(component);
        self.as_mut()
            .set_active_component(i32::try_from(component).expect("layered value component index"));
        match self.rust().controller.edit_component(next, component) {
            layered::LayeredEdit::Base(([first, second], _)) => {
                self.as_mut().base_pair_edited(first, second);
            }
            layered::LayeredEdit::Keyframe(([first, second], _)) => {
                self.as_mut().keyframe_pair_edited(
                    first,
                    second,
                    i32::try_from(component).expect("layered value component index"),
                    changes.iter().any(|(component, _)| *component == 0),
                    changes.iter().any(|(component, _)| *component == 1),
                );
            }
        }
    }

    pub fn configure_layered_pair(mut self: Pin<&mut Self>, first: f64, second: f64) {
        self.as_mut().rust_mut().pair = [first, second];
    }

    pub fn edit_layered_triple(
        mut self: Pin<&mut Self>,
        first: f64,
        second: f64,
        third: f64,
        component: i32,
    ) {
        let component = usize::try_from(component).expect("non-negative layered value component");
        let next = [first, second, third];
        self.as_mut().rust_mut().triple = next;
        self.rust().controller.select_component::<3>(component);
        self.as_mut()
            .set_active_component(i32::try_from(component).expect("layered value component index"));
        match self.rust().controller.edit_component(next, component) {
            layered::LayeredEdit::Base(([first, second, third], _)) => {
                self.as_mut().base_triple_edited(first, second, third);
            }
            layered::LayeredEdit::Keyframe(([first, second, third], _)) => {
                self.as_mut().keyframe_triple_edited(
                    first,
                    second,
                    third,
                    i32::try_from(component).expect("layered value component index"),
                );
            }
        }
    }

    pub fn configure_layered_triple(mut self: Pin<&mut Self>, first: f64, second: f64, third: f64) {
        self.as_mut().rust_mut().triple = [first, second, third];
    }

    pub fn select_layered_component(mut self: Pin<&mut Self>, component: i32) {
        let component = usize::try_from(component).expect("non-negative layered value component");
        self.rust().controller.select_component::<2>(component);
        self.as_mut()
            .set_active_component(i32::try_from(component).expect("layered value component index"));
    }

    pub fn select_layered_triple_component(mut self: Pin<&mut Self>, component: i32) {
        let component = usize::try_from(component).expect("non-negative layered value component");
        self.rust().controller.select_component::<3>(component);
        self.as_mut()
            .set_active_component(i32::try_from(component).expect("layered value component index"));
    }
}

impl cxx_qt::Initialize for qobject::LivePerformanceBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::LivePerformanceBackend {
    pub fn refresh_interval(&self) -> i32 {
        i32::try_from(shrimply_components_core::performance::REFRESH_INTERVAL.as_millis())
            .expect("live performance refresh interval must fit i32")
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        if !self.as_mut().rust_mut().rows.refresh() {
            return;
        }
        let (titles, subtitles) = {
            let rows = self.rust().rows.rows();
            (
                rows.iter()
                    .map(|row| QString::from(&row.title))
                    .collect::<QStringList>(),
                rows.iter()
                    .map(|row| QString::from(&row.subtitle))
                    .collect::<QStringList>(),
            )
        };
        self.as_mut().set_titles(titles);
        self.as_mut().set_subtitles(subtitles);
    }

    pub fn clear(self: Pin<&mut Self>) {
        shrimply_components_core::performance::clear();
    }

    pub fn report_json(&self) -> QString {
        QString::from(shrimply_components_core::performance::report_json())
    }
}

impl qobject::ProjectSettingsBackend {
    pub fn settings_text(&self, field: &QString) -> QString {
        let text = match field.to_string().as_str() {
            "fps" => "FPS",
            "resolution" => "Resolution",
            "discard" => project_settings::SETTINGS_DISCARD,
            "apply" => project_settings::SETTINGS_APPLY,
            "cancel" => project_settings::SETTINGS_CANCEL,
            "title" => project_settings::SETTINGS_TITLE,
            "warning" => project_settings::SETTINGS_WARNING,
            field => panic!("unknown project settings label: {field}"),
        };
        shrimply_i18n_qt::text(text)
    }

    pub fn discard(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().settings.discard();
        self.as_mut().publish_settings();
    }

    pub fn minimum_dimension(&self) -> i32 {
        i32::try_from(MIN_CANVAS_DIMENSION).expect("minimum canvas dimension must fit i32")
    }

    pub fn maximum_dimension(&self) -> i32 {
        i32::try_from(MAX_CANVAS_DIMENSION).expect("maximum canvas dimension must fit i32")
    }

    pub fn preset_label(&self, index: i32) -> QString {
        match index_of(index) {
            Some(index) if index == project_settings::CUSTOM_PRESET_INDEX => {
                shrimply_i18n_qt::text("Custom")
            }
            Some(index) => PROJECT_PRESETS
                .get(index)
                .map_or_else(QString::default, |preset| {
                    shrimply_i18n_qt::text(preset.label)
                }),
            None => QString::default(),
        }
    }

    pub fn frame_rate_label(&self, index: i32) -> QString {
        let Some(index) = index_of(index) else {
            return QString::default();
        };
        if let Some(rate) = COMMON_FRAME_RATES.get(index) {
            return QString::from(rate.label);
        }
        self.rust()
            .settings
            .custom_frame_rate
            .filter(|_| index == COMMON_FRAME_RATES.len())
            .map_or_else(QString::default, |rate| {
                QString::from(fraction_as_label(rate))
            })
    }

    pub fn frame_rate_values(&self) -> QStringList {
        (0..*self.frame_rate_count())
            .map(|index| QString::from(index.to_string()))
            .collect()
    }

    pub fn frame_rate_labels(&self) -> QStringList {
        (0..*self.frame_rate_count())
            .map(|index| self.frame_rate_label(index))
            .collect()
    }

    pub fn configure(
        mut self: Pin<&mut Self>,
        width: i32,
        height: i32,
        fps_numerator: &QString,
        fps_denominator: &QString,
    ) {
        let (Ok(width), Ok(height), Ok(fps_numerator), Ok(fps_denominator)) = (
            u32::try_from(width),
            u32::try_from(height),
            fps_numerator.to_string().parse::<i64>(),
            fps_denominator.to_string().parse::<i64>(),
        ) else {
            return;
        };
        if fps_numerator <= 0 || fps_denominator <= 0 {
            return;
        }
        let settings = project_settings::ProjectSettings::from_values(
            CanvasSize { width, height },
            fraction_new(fps_numerator, fps_denominator),
        );
        self.as_mut().rust_mut().settings = settings;
        self.as_mut().set_frame_rate_count(
            i32::try_from(
                COMMON_FRAME_RATES.len() + usize::from(settings.custom_frame_rate.is_some()),
            )
            .expect("project frame-rate option count"),
        );
        self.as_mut().publish_settings();
    }

    pub fn select_preset(mut self: Pin<&mut Self>, index: i32) {
        let Some(index) = index_of(index) else {
            return;
        };
        self.as_mut().rust_mut().settings.select_preset(index);
        self.as_mut().publish_settings();
    }

    pub fn set_width_value(mut self: Pin<&mut Self>, width: i32) {
        self.as_mut()
            .rust_mut()
            .settings
            .set_width(u32::try_from(width).expect("project width must be non-negative"));
        self.as_mut().publish_settings();
    }

    pub fn set_height_value(mut self: Pin<&mut Self>, height: i32) {
        self.as_mut()
            .rust_mut()
            .settings
            .set_height(u32::try_from(height).expect("project height must be non-negative"));
        self.as_mut().publish_settings();
    }

    pub fn set_frame_rate_value(mut self: Pin<&mut Self>, index: i32) {
        let Some(index) = index_of(index) else {
            return;
        };
        if index < COMMON_FRAME_RATES.len() {
            self.as_mut().rust_mut().settings.set_frame_rate(index);
        } else if index != COMMON_FRAME_RATES.len()
            || self.rust().settings.custom_frame_rate.is_none()
        {
            return;
        }
        self.as_mut().publish_settings();
    }

    pub fn fps_numerator(&self) -> QString {
        QString::from(fraction_numerator(self.selected_frame_rate()).to_string())
    }

    pub fn fps_denominator(&self) -> QString {
        QString::from(fraction_denominator(self.selected_frame_rate()).to_string())
    }

    fn selected_frame_rate(&self) -> Fraction {
        self.rust()
            .settings
            .settings()
            .expect("project settings must include a frame rate")
            .1
    }

    fn publish_settings(mut self: Pin<&mut Self>) {
        let settings = self.rust().settings;
        self.as_mut().set_preset(settings.preset as i32);
        self.as_mut().set_width(settings.width as i32);
        self.as_mut().set_height(settings.height as i32);
        self.as_mut().set_frame_rate(settings.frame_rate as i32);
        self.as_mut().set_changed(settings.changed());
    }
}

#[derive(Default)]
pub struct ComponentTranslationsRust;

impl cxx_qt::Initialize for qobject::ComponentTranslations {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::ComponentTranslations {
    pub fn text(&self, key: &QString) -> QString {
        shrimply_i18n_qt::text(&key.to_string())
    }
}

fn qcolor(color: Color<u8>) -> QColor {
    QColor::from_rgba_f(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    )
}

fn color_from_qcolor(color: &QColor) -> Color<u8> {
    fn channel(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    Color::new(
        channel(color.red_f()),
        channel(color.green_f()),
        channel(color.blue_f()),
        channel(color.alpha_f()),
    )
}

fn index_of(index: i32) -> Option<usize> {
    usize::try_from(index).ok()
}

fn group_ratios(dimensions: usize, values: [Fraction; 3]) -> [Fraction; 2] {
    if dimensions == 2 {
        [
            number::pair_ratio(values[0], values[1]),
            fraction_from_f64(1.0),
        ]
    } else {
        number::triple_ratios(values)
    }
}
