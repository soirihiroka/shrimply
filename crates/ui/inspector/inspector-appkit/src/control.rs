use objc2::ClassType;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSStackView, NSView, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};
use shrimply_components_appkit::{
    ActionButton, ColorPicker, MultilineTextInput, Number2Picker, Number3Picker, NumberPicker,
    ReadOnlyField, SingleLineTextInput, StringChoice, StringSelector, column_append_intrinsic,
    control_row, row_stack, switch_row,
};
use shrimply_inspector_core::{
    BasicInspectorAction, ControlKind, InspectorControl, InspectorController, InspectorSection,
    InspectorTarget,
};
use shrimply_math_color::Color;
use shrimply_math_core::{Fraction, fraction_as_f64, fraction_new};
use shrimply_project_document::project::{
    COMMON_FRAME_RATES, CanvasSize, MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(super) type Polls = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

#[derive(Clone)]
pub(super) struct Context {
    pub focus: Rc<super::focus::FocusMap>,
    pub preferences: shrimply_editor_state::preferences::SharedPreferences,
    pub server_url: Rc<RefCell<String>>,
    pub controller: InspectorController,
    pub target: InspectorTarget,
    pub dirty: Rc<Cell<bool>>,
    pub force_rebuild: Rc<Cell<bool>>,
    pub polls: Polls,
}

impl Context {
    pub fn apply(&self, action: &BasicInspectorAction) {
        self.refresh(self.controller.apply_basic_action(&self.target, action));
    }

    pub(super) fn finish(&self, result: Result<(), String>) {
        if let Err(error) = result {
            show_error(&error);
        }
    }

    pub(super) fn refresh(&self, result: Result<(), String>) {
        self.finish(result);
        self.dirty.set(true);
    }

    fn set_text(&self, control: &InspectorControl, value: &str) -> bool {
        let result = self
            .controller
            .set_basic_control_value(&self.target, control, value);
        let success = result.is_ok();
        self.finish(result);
        success
    }

    fn set_components(&self, control: &InspectorControl, values: &[f64]) -> bool {
        let result = self
            .controller
            .set_basic_control_components(&self.target, control, values);
        let success = result.is_ok();
        self.finish(result);
        success
    }

    fn set_fraction(&self, control: &InspectorControl, value: Fraction) {
        let result = self
            .controller
            .set_basic_control_fraction(&self.target, control, value);
        self.finish(result);
    }

    pub(super) fn commit(&self, control: &InspectorControl) {
        let result = self.controller.commit_basic_control(&self.target, control);
        self.refresh(result);
    }
}

pub(super) fn append_section(
    column: &NSStackView,
    section: &InspectorSection,
    context: &Context,
    mtm: MainThreadMarker,
) {
    for control in section.controls.iter().filter(|control| control.visible) {
        column_append_intrinsic(column, &view(control, context, mtm));
    }
}

pub(super) fn view(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let view = if shrimply_inspector_core::InspectorGraphKind::for_control(control.kind).is_some() {
        let mut unlabelled = control.clone();
        unlabelled.label.clear();
        let editor = editor_view(&unlabelled, context, mtm);
        super::layered::view(control, &editor, context, mtm)
    } else {
        editor_view(control, context, mtm)
    };
    if let Some(focus) = &control.preview_focus {
        context
            .focus
            .register(&view, &context.target, focus.clone());
    }
    view
}

fn editor_view(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let standard_field = matches!(
        control.kind,
        ControlKind::Boolean
            | ControlKind::LayeredBoolean
            | ControlKind::Number
            | ControlKind::LayeredNumber
            | ControlKind::Fraction
            | ControlKind::Text
            | ControlKind::MultilineText
            | ControlKind::LayeredText
            | ControlKind::Selector
            | ControlKind::OptionalSelector
            | ControlKind::OptionalNumberSelector
            | ControlKind::LayeredSelector
            | ControlKind::AudioCachePreset
            | ControlKind::VisualCacheQuality
            | ControlKind::Color
            | ControlKind::LayeredColor
            | ControlKind::Vector2
            | ControlKind::LayeredVector2
            | ControlKind::Vector3
            | ControlKind::LayeredVector3
    );
    let disabled = !control.editable || !control.sensitive;
    if disabled
        && !standard_field
        && !matches!(
            control.kind,
            ControlKind::ReadOnly
                | ControlKind::FileLocation
                | ControlKind::InfoHeading
                | ControlKind::InfoArtwork
                | ControlKind::InfoLoading
                | ControlKind::Performance
                | ControlKind::VoiceModel
                | ControlKind::Action
                | ControlKind::Analysis
                | ControlKind::AudioCache
                | ControlKind::VisualCache
        )
    {
        return read_only(control, false, mtm);
    }
    let view: Retained<NSView> = match control.kind {
        ControlKind::Boolean | ControlKind::LayeredBoolean => {
            let context = context.clone();
            let edited_control = control.clone();
            switch_row(
                &control.label,
                tooltip(control),
                parse_bool(&control.value),
                move |value| {
                    if context.set_text(&edited_control, &value.to_string()) {
                        context.commit(&edited_control);
                    }
                },
                mtm,
            )
        }
        ControlKind::Number | ControlKind::LayeredNumber => number(control, context, mtm),
        ControlKind::Fraction => fraction(control, context, mtm),
        ControlKind::Text => text(control, context, mtm),
        ControlKind::MultilineText | ControlKind::LayeredText => {
            multiline_text(control, context, mtm)
        }
        ControlKind::VoiceModel
        | ControlKind::Selector
        | ControlKind::OptionalSelector
        | ControlKind::OptionalNumberSelector
        | ControlKind::AudioCachePreset
        | ControlKind::VisualCacheQuality
        | ControlKind::LayeredSelector => selector(control, context, mtm),
        ControlKind::Color | ControlKind::LayeredColor => color(control, context, mtm),
        ControlKind::Vector2 | ControlKind::LayeredVector2 => vector2(control, context, mtm),
        ControlKind::Vector3 | ControlKind::LayeredVector3 => vector3(control, context, mtm),
        ControlKind::LayeredDrawing => read_only(control, false, mtm),
        ControlKind::ReadOnly => read_only(control, false, mtm),
        ControlKind::FileLocation => file_location(control, mtm),
        ControlKind::InfoHeading | ControlKind::InfoArtwork | ControlKind::InfoLoading => {
            super::info::view(control, mtm)
        }
        ControlKind::Performance => shrimply_components_appkit::live_performance(mtm)
            .as_super()
            .into(),
        ControlKind::FontFamilies => super::fonts::view(control, context, mtm),
        ControlKind::ProjectSettings => project_settings(control, context, mtm),
        ControlKind::BeatDetection => beat_detection(control, context, mtm),
        ControlKind::TtsEditor => super::tts::view(control, context, mtm),
        ControlKind::VisualModifierMenu | ControlKind::AudioModifierMenu => {
            let menu_kind = control.kind;
            let context = context.clone();
            let menu = shrimply_components_appkit::modifier_menu(
                shrimply_components_appkit::SearchChoices::from(
                    control
                        .values
                        .iter()
                        .zip(&control.labels)
                        .map(|(value, label)| StringChoice {
                            value: value.clone(),
                            label: label.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
                .with_keywords(control.search_terms.clone()),
                move |kind| {
                    let result =
                        context
                            .controller
                            .add_control_modifier(&context.target, menu_kind, &kind);
                    context.refresh(result);
                },
                mtm,
            );
            menu.view().into()
        }
        ControlKind::AudioCache | ControlKind::VisualCache => cache_button(control, context, mtm),
        ControlKind::Action | ControlKind::Analysis => {
            if control.action
                == Some(shrimply_inspector_core::InspectorControlAction::ToggleCameraAnalysis)
            {
                let controller = context.controller.clone();
                let target = context.target.clone();
                let server_url = context.server_url.clone();
                let dirty = context.dirty.clone();
                let previous =
                    RefCell::new(controller.camera_analysis_state(&target, &server_url.borrow()));
                context.polls.borrow_mut().push(Box::new(move || {
                    let current = controller.camera_analysis_state(&target, &server_url.borrow());
                    if *previous.borrow() != current {
                        *previous.borrow_mut() = current;
                        dirty.set(true);
                    }
                }));
            } else if control.analysis.is_some()
                && let Some(action) = control.action
            {
                let controller = context.controller.clone();
                let target = context.target.clone();
                let server_url = context.server_url.clone();
                let dirty = context.dirty.clone();
                let previous = RefCell::new(controller.control_analysis_presentation(
                    &target,
                    action,
                    &server_url.borrow(),
                ));
                context.polls.borrow_mut().push(Box::new(move || {
                    let current = controller.control_analysis_presentation(
                        &target,
                        action,
                        &server_url.borrow(),
                    );
                    if *previous.borrow() != current {
                        *previous.borrow_mut() = current;
                        dirty.set(true);
                    }
                }));
            }
            let edited = control.clone();
            let context = context.clone();
            let title = control
                .analysis
                .as_ref()
                .map(|a| a.label.as_str())
                .unwrap_or(if control.value.is_empty() {
                    &control.label
                } else {
                    &control.value
                });
            let on_action = move || {
                if let Some(action) = edited.action
                    && let Some(selection) = action.file_selection()
                {
                    super::files::select(&context, action, selection, mtm);
                    return;
                }
                match edited.action {
                    Some(shrimply_inspector_core::InspectorControlAction::ToggleCameraAnalysis) => {
                        let server_url = context.server_url.borrow().clone();
                        context.refresh(
                            context
                                .controller
                                .toggle_camera_analysis(&context.target, server_url),
                        );
                        return;
                    }
                    Some(shrimply_inspector_core::InspectorControlAction::ToggleSam2Analysis {
                        modifier_id,
                        ..
                    }) => {
                        let server_url = context.server_url.borrow().clone();
                        context.refresh(context.controller.toggle_sam2_analysis(
                            &context.target,
                            modifier_id,
                            server_url,
                        ));
                        return;
                    }
                    _ => {}
                }
                let result = edited
                    .action
                    .ok_or_else(|| "inspector action is unavailable".to_string())
                    .and_then(|action| {
                        context
                            .controller
                            .trigger_video_control_action(&context.target, action)
                    });
                context.refresh(result);
            };
            let button: Retained<objc2_app_kit::NSButton> =
                if let Some(analysis) = &control.analysis {
                    use shrimply_components_appkit::{ProgressButton, ProgressButtonState};
                    let button = ProgressButton::new(title, mtm);
                    button.connect_action(on_action, mtm);
                    button.set_state(if !analysis.active() {
                        ProgressButtonState::Idle
                    } else if analysis.progress < 0.0 {
                        ProgressButtonState::Indeterminate
                    } else {
                        ProgressButtonState::Progress(analysis.progress)
                    });
                    Retained::from(button.view())
                } else {
                    Retained::from(ActionButton::new(title, on_action, mtm).view())
                };
            button.setEnabled(control.action_sensitive && control.sensitive);
            let tooltip = control
                .analysis
                .as_ref()
                .map(|analysis| analysis.tooltip.as_str())
                .or_else(|| tooltip(control));
            button.setToolTip(tooltip.map(NSString::from_str).as_deref());
            button.as_super().as_super().into()
        }
    };
    if disabled && standard_field {
        disable_native_controls(&view);
    }
    view
}

// NumberPicker builders gate their own custom mouse handling. Native editors
// keep their normal component and use AppKit's disabled/editable state.
fn disable_native_controls(view: &NSView) {
    if let Some(control) = view.downcast_ref::<objc2_app_kit::NSControl>() {
        control.setEnabled(false);
    }
    if let Some(text) = view.downcast_ref::<objc2_app_kit::NSTextView>() {
        text.setEditable(false);
        text.setTextColor(Some(&objc2_app_kit::NSColor::disabledControlTextColor()));
    }
    for child in view.subviews().iter() {
        disable_native_controls(&child);
    }
}

fn number(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let value = parse_number(&control.value);
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let mut builder = NumberPicker::builder(value)
        .enabled(control.editable && control.sensitive)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .digits(control.number.digits.max(0) as usize)
        .unit_name(control.number.unit)
        .on_change(move |value| {
            editing_context.set_text(&editing, &value.to_string());
        })
        .on_commit(move |_| committing.commit(&committed_control));
    if !control.prefix_icon.is_empty() && control.prefix_icon_rotates {
        builder = builder.rotating_prefix_symbol_with_offset(
            appkit_symbol(&control.prefix_icon),
            control.prefix_icon_rotation_offset_degrees,
        );
    }
    labelled(control, builder.build(mtm), mtm)
}

fn fraction(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let numerator = component_i64(control, 0);
    let denominator = component_i64(control, 1);
    let editing = context.clone();
    let editing_control = control.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let picker = NumberPicker::fraction_builder(fraction_new(numerator, denominator))
        .enabled(control.editable && control.sensitive)
        .drag_step(control.number.drag_step)
        .digits(control.number.digits.max(0) as usize)
        .unit_name(control.number.unit)
        .on_change_fraction(move |value| {
            editing.set_fraction(&editing_control, value);
        })
        .on_commit_fraction(move |_| committing.commit(&committed_control))
        .build(mtm);
    labelled(control, picker, mtm)
}

fn text(control: &InspectorControl, context: &Context, mtm: MainThreadMarker) -> Retained<NSView> {
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let input = SingleLineTextInput::new(
        &control.value,
        None,
        None,
        move |value| {
            editing_context.set_text(&editing, &value);
        },
        move |_| committing.commit(&committed_control),
        mtm,
    );
    labelled(control, input.view().into(), mtm)
}

fn multiline_text(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let input = MultilineTextInput::new(
        &control.value,
        120.0,
        None,
        move |value| {
            editing_context.set_text(&editing, &value);
            true
        },
        move || committing.commit(&committed_control),
        mtm,
    );
    labelled(control, input.view().as_super().into(), mtm)
}

fn selector(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let mut choices = control
        .values
        .iter()
        .zip(&control.labels)
        .map(|(value, label)| StringChoice {
            value: value.clone(),
            label: label.clone(),
        })
        .collect::<Vec<_>>();
    if matches!(
        control.kind,
        ControlKind::OptionalSelector | ControlKind::OptionalNumberSelector
    ) {
        choices.insert(
            0,
            StringChoice {
                value: String::new(),
                label: "None".to_string(),
            },
        );
    }
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let selector = StringSelector::new(
        &control.value,
        choices,
        move |value| {
            if editing_context.set_text(&editing, &value) {
                committing.commit(&committed_control);
            }
        },
        mtm,
    );
    if let Some(button) = selector.view().downcast_ref::<objc2_app_kit::NSControl>() {
        button.setEnabled(control.sensitive);
    }
    selector
        .view()
        .setToolTip(tooltip(control).map(NSString::from_str).as_deref());
    labelled(control, selector.view().into(), mtm)
}

fn color(control: &InspectorControl, context: &Context, mtm: MainThreadMarker) -> Retained<NSView> {
    let components = color_components(control);
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let picker = ColorPicker::new(
        Color::new(components[0], components[1], components[2], components[3]),
        move |color| {
            editing_context.set_components(&editing, &color.to_array().map(f64::from));
        },
        move || {
            committing.commit(&committed_control);
        },
        mtm,
    );
    labelled(control, picker.view().as_super().as_super().into(), mtm)
}

fn vector2(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let values = [component_f64(control, 0), component_f64(control, 1)]
        .map(|value| value / control.store_multiplier);
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let mut builder = Number2Picker::builder(values[0], values[1])
        .enabled(control.editable && control.sensitive)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .digits(control.number.digits.max(0) as usize)
        .unit_name(control.number.unit)
        .on_change(move |values, _| {
            editing_context.set_components(&editing, &values);
        })
        .on_commit(move || committing.commit(&committed_control));
    if let [first, second, ..] = control.prefixes.as_slice() {
        builder = builder.first_prefix(first).second_prefix(second);
    }
    if control.lock {
        builder = builder.enable_lock();
    }
    labelled(control, builder.build_with_handles(mtm).widget, mtm)
}

fn vector3(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let values = [
        component_f64(control, 0),
        component_f64(control, 1),
        component_f64(control, 2),
    ]
    .map(|value| value / control.store_multiplier);
    let editing = control.clone();
    let editing_context = context.clone();
    let committing = context.clone();
    let committed_control = control.clone();
    let mut builder = Number3Picker::builder(values)
        .enabled(control.editable && control.sensitive)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .digits(control.number.digits.max(0) as usize)
        .unit_name(control.number.unit)
        .on_change(move |values, _| {
            editing_context.set_components(&editing, &values);
        })
        .on_commit(move || committing.commit(&committed_control));
    if let [first, second, third, ..] = control.prefixes.as_slice() {
        builder = builder.prefixes([first, second, third]);
    }
    if control.lock {
        builder = builder.enable_lock();
    }
    labelled(control, builder.build_with_handles(mtm).widget, mtm)
}

fn read_only(
    control: &InspectorControl,
    right_aligned: bool,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let value = if control.value.is_empty() {
        control.components.join(", ")
    } else {
        control.value.clone()
    };
    let field = ReadOnlyField::new(&value, right_aligned, mtm);
    labelled(control, field.view().as_super().into(), mtm)
}

fn file_location(control: &InspectorControl, mtm: MainThreadMarker) -> Retained<NSView> {
    let path = control.value.clone();
    let field = ReadOnlyField::with_action(
        &control.value,
        "Reveal in Finder",
        move || {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
            NSWorkspace::sharedWorkspace()
                .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        },
        mtm,
    );
    labelled(control, field.view().as_super().into(), mtm)
}

fn project_settings(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let initial_width = component_f64(control, 0).round() as u32;
    let initial_height = component_f64(control, 1).round() as u32;
    let initial_frame_rate = fraction_new(component_i64(control, 2), component_i64(control, 3));
    let width = Rc::new(Cell::new(initial_width));
    let height = Rc::new(Cell::new(initial_height));
    let frame_rate = Rc::new(Cell::new(initial_frame_rate));
    let actions = row_stack(6.0, mtm);
    actions.setHidden(true);
    let update_actions: Rc<dyn Fn()> = Rc::new({
        let width = width.clone();
        let height = height.clone();
        let frame_rate = frame_rate.clone();
        let actions = actions.clone();
        move || {
            actions.setHidden(
                width.get() == initial_width
                    && height.get() == initial_height
                    && frame_rate.get() == initial_frame_rate,
            );
        }
    });
    let resolution = Number2Picker::builder(f64::from(width.get()), f64::from(height.get()))
        .minimum(f64::from(MIN_CANVAS_DIMENSION))
        .maximum(f64::from(MAX_CANVAS_DIMENSION))
        .drag_step(1.0)
        .digits(0)
        .first_prefix("W")
        .second_prefix("H")
        .unit_name("px")
        .on_change({
            let width = width.clone();
            let height = height.clone();
            let update_actions = update_actions.clone();
            move |values, _| {
                width.set(values[0].round() as u32);
                height.set(values[1].round() as u32);
                update_actions();
            }
        })
        .build_with_handles(mtm);
    let mut choices = COMMON_FRAME_RATES
        .iter()
        .map(|rate| StringChoice {
            value: rate_key(rate.value),
            label: rate.label.to_string(),
        })
        .collect::<Vec<_>>();
    if !COMMON_FRAME_RATES
        .iter()
        .any(|rate| rate.value == frame_rate.get())
    {
        choices.push(StringChoice {
            value: rate_key(frame_rate.get()),
            label: fraction_as_f64(frame_rate.get()).to_string(),
        });
    }
    let fps = StringSelector::new(
        &rate_key(frame_rate.get()),
        choices,
        {
            let frame_rate = frame_rate.clone();
            let update_actions = update_actions.clone();
            move |value| {
                frame_rate.set(parse_rate(&value));
                update_actions();
            }
        },
        mtm,
    );
    actions.addArrangedSubview(&NSView::new(mtm));
    let discard = ActionButton::new(
        "Discard",
        {
            let dirty = context.dirty.clone();
            let force_rebuild = context.force_rebuild.clone();
            move || {
                force_rebuild.set(true);
                dirty.set(true);
            }
        },
        mtm,
    );
    actions.addArrangedSubview(discard.view());
    let apply = ActionButton::new(
        "Apply",
        {
            let context = context.clone();
            move || {
                if !confirm_project_settings() {
                    return;
                }
                context.refresh(context.controller.apply_project_settings(
                    CanvasSize {
                        width: width.get(),
                        height: height.get(),
                    },
                    frame_rate.get(),
                ));
            }
        },
        mtm,
    );
    actions.addArrangedSubview(apply.view());
    let column = shrimply_components_appkit::column_stack(8.0, mtm);
    column_append_intrinsic(&column, &control_row("Resolution", &resolution.widget, mtm));
    column_append_intrinsic(&column, &control_row("Frame rate", fps.view(), mtm));
    column_append_intrinsic(&column, &actions);
    column.into_super()
}

fn labelled(
    control: &InspectorControl,
    view: Retained<NSView>,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    if control.label.is_empty() {
        view
    } else {
        control_row(&control.label, &view, mtm)
    }
}

fn tooltip(control: &InspectorControl) -> Option<&str> {
    (!control.tooltip.is_empty())
        .then_some(control.tooltip.as_str())
        .or_else(|| (!control.subtitle.is_empty()).then_some(control.subtitle.as_str()))
}

fn appkit_symbol(icon: &str) -> &'static str {
    match icon {
        "rotation.svg" => "arrow.up",
        _ => panic!("unsupported AppKit inspector icon: {icon}"),
    }
}

fn parse_bool(value: &str) -> bool {
    value
        .parse()
        .unwrap_or_else(|_| panic!("invalid boolean inspector value: {value}"))
}

fn parse_number(value: &str) -> f64 {
    value
        .parse()
        .unwrap_or_else(|_| panic!("invalid numeric inspector value: {value}"))
}

fn component_f64(control: &InspectorControl, index: usize) -> f64 {
    parse_number(
        control
            .components
            .get(index)
            .unwrap_or_else(|| panic!("missing inspector component {index}")),
    )
}

fn component_i64(control: &InspectorControl, index: usize) -> i64 {
    control
        .components
        .get(index)
        .unwrap_or_else(|| panic!("missing inspector component {index}"))
        .parse()
        .unwrap_or_else(|_| panic!("invalid integer inspector component {index}"))
}

fn color_components(control: &InspectorControl) -> [u8; 4] {
    std::array::from_fn(|index| {
        control
            .components
            .get(index)
            .map(|value| {
                value
                    .parse()
                    .unwrap_or_else(|_| panic!("invalid color component {index}"))
            })
            .unwrap_or(if index == 3 { u8::MAX } else { 0 })
    })
}

fn rate_key(rate: Fraction) -> String {
    format!(
        "{}/{}",
        shrimply_math_core::fraction_numerator(rate),
        shrimply_math_core::fraction_denominator(rate)
    )
}

fn parse_rate(value: &str) -> Fraction {
    let (numerator, denominator) = value
        .split_once('/')
        .unwrap_or_else(|| panic!("invalid frame rate choice: {value}"));
    fraction_new(
        numerator.parse().expect("frame rate numerator"),
        denominator.parse().expect("frame rate denominator"),
    )
}

fn show_error(error: &str) {
    let mtm = MainThreadMarker::new().expect("inspector actions run on the main thread");
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Shrimply"));
    alert.setInformativeText(&NSString::from_str(error));
    alert.runModal();
}

fn beat_detection(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let row = row_stack(6.0, mtm);
    row.addArrangedSubview(&NSView::new(mtm));
    let spinner = shrimply_components_appkit::Spinner::new(mtm);
    row.addArrangedSubview(spinner.view());
    let editing = context.clone();
    let edited = control.clone();
    let toggle = shrimply_components_appkit::Switch::new(
        parse_bool(&control.value),
        Some(&control.subtitle),
        move |value| {
            if editing.set_text(&edited, &value.to_string()) {
                editing.commit(&edited);
            }
        },
        mtm,
    );
    row.addArrangedSubview(toggle.view());
    let controller = context.controller.clone();
    let target = context.target.clone();
    let previous = Cell::new(None);
    let poll = move || {
        let loading = controller.audio_beat_detection_loading(&target);
        if previous.replace(Some(loading)) != Some(loading) {
            spinner.set_active(loading);
        }
    };
    poll();
    context.polls.borrow_mut().push(Box::new(poll));
    control_row(&control.label, &row, mtm)
}

fn confirm_project_settings() -> bool {
    let mtm = MainThreadMarker::new().expect("inspector actions run on the main thread");
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Change Project Settings?"));
    alert.setInformativeText(&NSString::from_str(
        "Changing the frame rate or resolution can affect timing, visual layout, and rendered output.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Apply"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    alert.runModal() == NSAlertFirstButtonReturn
}

fn cache_button(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    use shrimply_components_appkit::{ProgressButton, ProgressButtonState};
    let id = control.target_id.expect("cache control must have an owner");
    let kind = control.kind;
    let button = ProgressButton::new(&control.value, mtm);
    // The polling callback owns the button, so its action must not retain the
    // Context's polling collection and form a cycle.
    let controller = context.controller.clone();
    let target = context.target.clone();
    let dirty = context.dirty.clone();
    button.connect_action(
        move || {
            let result = match kind {
                ControlKind::AudioCache => controller.toggle_audio_cache(&target, id),
                ControlKind::VisualCache => controller.toggle_visual_cache(&target, id),
                _ => unreachable!("cache button kind"),
            };
            if let Err(error) = result {
                show_error(&error);
            }
            dirty.set(true);
        },
        mtm,
    );
    button
        .view()
        .setEnabled(control.sensitive && control.action_sensitive);
    let view: Retained<NSView> = button.view().as_super().as_super().into();
    let controller = context.controller.clone();
    let target = context.target.clone();
    let dirty = context.dirty.clone();
    let previous = RefCell::new(None::<shrimply_inspector_core::CacheControlPresentation>);
    let poll = move || {
        // Initialize once before attachment, then poll only visible controls.
        if previous.borrow().is_some()
            && (button.view().window().is_none() || button.view().isHiddenOrHasHiddenAncestor())
        {
            return;
        }
        let current = shrimply_inspector_core::cache_control_presentation(
            controller.cache_status(&target, kind, id),
            "Click to cancel baking",
        );
        if previous.borrow().as_ref() == Some(&current) {
            return;
        }
        if let Some(old) = previous.borrow().as_ref()
            && old.baking != current.baking
        {
            // Reconcile the format selector's sensitivity only at phase changes.
            dirty.set(true);
            if old.baking && kind == ControlKind::AudioCache {
                controller.refresh_audio_cache();
            }
        }
        button.view().setTitle(&NSString::from_str(current.label));
        button
            .view()
            .setToolTip(Some(&NSString::from_str(&current.tooltip)));
        button.set_state(if !current.baking {
            ProgressButtonState::Idle
        } else if current.progress < 0.0 {
            ProgressButtonState::Indeterminate
        } else {
            ProgressButtonState::Progress(current.progress)
        });
        previous.replace(Some(current));
    };
    poll();
    context.polls.borrow_mut().push(Box::new(poll));
    view
}
