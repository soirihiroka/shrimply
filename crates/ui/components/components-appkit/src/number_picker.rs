use crate::action;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSAffineTransformNSAppKitAdditions;
use objc2_app_kit::NSControlTextEditingDelegate;
use objc2_app_kit::{
    NSButton, NSButtonType, NSColor, NSControlStateValueOn, NSCursor, NSEvent, NSEventMask,
    NSEventType, NSGraphicsContext, NSImage, NSImageView, NSLayoutConstraintOrientation,
    NSLayoutPriorityDefaultHigh, NSLayoutPriorityDefaultLow, NSTextAlignment, NSTextField,
    NSTextFieldDelegate, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_core_graphics::{CGAssociateMouseAndMouseCursorPosition, CGError};
use objc2_foundation::{
    MainThreadMarker, NSAffineTransform, NSNotification, NSObjectProtocol, NSPoint, NSRect,
    NSString,
};
use shrimply_components_core::number::{
    DEFAULT_MAXIMUM, DEFAULT_MINIMUM, NumberConfig, NumberDrag, accepted_value, format_value,
    locked_pair, locked_triple, pair_ratio, parse_fraction, positive_fraction_or, triple_ratios,
};
use shrimply_math_core::{
    FRACTION_ZERO, Fraction, fraction_as_f64, fraction_from_f64, fraction_from_integer,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type NumberCallback = Rc<dyn Fn(Fraction)>;
const NUMBER_MIN_WIDTH: f64 = 64.0;
type PairCallback = Box<dyn Fn([f64; 2], usize)>;
type TripleCallback = Box<dyn Fn([f64; 3], usize)>;

struct RotatingImageViewIvars {
    angle_degrees: Cell<f64>,
}

define_class!(
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RotatingImageViewIvars]
    struct RotatingImageView;

    unsafe impl NSObjectProtocol for RotatingImageView {}

    impl RotatingImageView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            NSGraphicsContext::saveGraphicsState_class();
            let bounds = self.bounds();
            let transform = NSAffineTransform::transform();
            transform.translateXBy_yBy(bounds.size.width / 2.0, bounds.size.height / 2.0);
            transform.rotateByDegrees(-self.ivars().angle_degrees.get());
            transform.translateXBy_yBy(-bounds.size.width / 2.0, -bounds.size.height / 2.0);
            transform.concat();
            unsafe {
                let _: () = msg_send![super(self), drawRect: bounds];
            }
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);

impl RotatingImageView {
    fn set_angle(&self, angle_degrees: f64) {
        if self.ivars().angle_degrees.replace(angle_degrees) != angle_degrees {
            self.as_super().as_super().as_super().setNeedsDisplay(true);
        }
    }
}

struct NumberPickerIvars {
    enabled: bool,
    config: NumberConfig,
    value: Cell<Fraction>,
    display: Retained<NSView>,
    rotating_icon: Retained<RotatingImageView>,
    rotation_offset_degrees: f64,
    value_label: Retained<NSTextField>,
    entry: Retained<NSTextField>,
    on_change: NumberCallback,
    on_commit: NumberCallback,
    drag: Cell<NumberDrag>,
    pointer_locked: Cell<bool>,
    editing: Cell<bool>,
    preview_value: Cell<Option<Fraction>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
}

impl Drop for NumberPickerIvars {
    fn drop(&mut self) {
        if self.pointer_locked.replace(false) {
            let result = CGAssociateMouseAndMouseCursorPosition(true);
            assert_eq!(
                result,
                CGError::Success,
                "could not release the number picker pointer"
            );
            NSCursor::unhide();
        }
    }
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = NumberPickerIvars]
    struct NumberPickerView;

    unsafe impl NSObjectProtocol for NumberPickerView {}
    unsafe impl NSControlTextEditingDelegate for NumberPickerView {}
    unsafe impl NSTextFieldDelegate for NumberPickerView {}

    impl NumberPickerView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool { false }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool { true }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool { false }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<&NSView> {
            let hit: Option<&NSView> = unsafe { msg_send![super(self), hitTest: point] };
            if self.ivars().editing.get() {
                hit
            } else {
                hit.map(|_| self.as_super())
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !self.ivars().enabled { return; }
            self.ivars().drag.set(NumberDrag::begin(self.ivars().value.get()));
            let origin_x = event.locationInWindow().x;
            let window = self
                .window()
                .expect("number picker must be attached to a window");
            let mask = NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp;
            loop {
                let next = window
                    .nextEventMatchingMask(mask)
                    .expect("number interaction ended without a mouse-up event");
                match next.r#type() {
                    NSEventType::LeftMouseDragged => {
                        if self.ivars().pointer_locked.get() {
                            self.update_drag(next.deltaX(), false);
                        } else {
                            self.update_drag(next.locationInWindow().x - origin_x, true);
                        }
                    }
                    NSEventType::LeftMouseUp => {
                        if !self.ivars().pointer_locked.get() {
                            self.update_drag(next.locationInWindow().x - origin_x, true);
                        }
                        if self.ivars().drag.get().moved() {
                            self.finish_drag();
                        } else {
                            self.begin_edit();
                        }
                        break;
                    }
                    _ => unreachable!("number interaction requested only drag and mouse-up events"),
                }
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            if let Some(previous) = self.ivars().tracking_area.borrow_mut().take() {
                self.removeTrackingArea(&previous);
            }
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInKeyWindow
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            self.ivars().tracking_area.replace(Some(area));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if self.ivars().enabled && !self.ivars().editing.get() && !self.ivars().pointer_locked.get() {
                NSCursor::columnResizeCursor().set();
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if !self.ivars().pointer_locked.get() {
                NSCursor::arrowCursor().set();
            }
        }

        #[unsafe(method(commitText:))]
        fn commit_text(&self, _sender: &NSTextField) { self.commit_edit(); }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn text_did_end(&self, _notification: &NSNotification) {
            self.commit_edit();
        }

        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, notification: &NSNotification) {
            if !self.ivars().editing.get() {
                return;
            }
            let field = notification
                .object()
                .expect("number edit notification sender")
                .downcast::<NSTextField>()
                .expect("number edit notification must contain a text field");
            if let Some(value) = parse_fraction(field.stringValue().to_string().trim()) {
                let value = accepted_value(&self.ivars().config, value);
                if self.ivars().preview_value.replace(Some(value)) != Some(value) {
                    (self.ivars().on_change)(value);
                }
            }
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: &objc2_foundation::NSObject) {
            self.release_pointer();
            self.end_edit_without_change();
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn moved_to_window(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidMoveToWindow]; }
            if self.window().is_none() { self.release_pointer(); }
        }
    }
);

impl NumberPickerView {
    fn update_drag(&self, offset_x: f64, absolute: bool) {
        let mut drag = self.ivars().drag.get();
        let moved = if absolute {
            drag.update_absolute(offset_x)
        } else {
            drag.update_relative(offset_x)
        };
        if !moved {
            return;
        }
        self.ivars().drag.set(drag);
        self.lock_pointer();
        self.set_value(drag.value(&self.ivars().config));
    }

    fn finish_drag(&self) {
        assert!(
            self.ivars().drag.get().moved(),
            "finish_drag requires movement"
        );
        self.release_pointer();
        (self.ivars().on_commit)(self.ivars().value.get());
    }

    fn refresh(&self) {
        if !self.ivars().rotating_icon.isHidden() {
            let angle =
                fraction_as_f64(self.ivars().value.get()) + self.ivars().rotation_offset_degrees;
            self.ivars().rotating_icon.set_angle(angle);
        }
        self.ivars()
            .value_label
            .setStringValue(&NSString::from_str(&format_value(
                &self.ivars().config,
                self.ivars().value.get(),
            )));
    }

    fn set_value(&self, value: Fraction) -> bool {
        let value = accepted_value(&self.ivars().config, value);
        if self.ivars().value.replace(value) == value {
            return false;
        }
        self.refresh();
        (self.ivars().on_change)(value);
        true
    }

    fn set_value_silently(&self, value: Fraction) {
        let value = accepted_value(&self.ivars().config, value);
        if self.ivars().value.replace(value) != value {
            self.refresh();
        }
    }

    fn lock_pointer(&self) {
        if self.ivars().pointer_locked.replace(true) {
            return;
        }
        let result = CGAssociateMouseAndMouseCursorPosition(false);
        assert_eq!(
            result,
            CGError::Success,
            "could not capture the number picker pointer"
        );
        NSCursor::hide();
    }

    fn release_pointer(&self) {
        if !self.ivars().pointer_locked.replace(false) {
            return;
        }
        let result = CGAssociateMouseAndMouseCursorPosition(true);
        assert_eq!(
            result,
            CGError::Success,
            "could not release the number picker pointer"
        );
        NSCursor::unhide();
    }

    fn begin_edit(&self) {
        if !self.ivars().enabled || self.ivars().editing.get() {
            return;
        }
        self.ivars().editing.set(true);
        self.ivars().preview_value.set(None);
        self.ivars()
            .entry
            .setStringValue(&NSString::from_str(&format_value(
                &self.ivars().config,
                self.ivars().value.get(),
            )));
        self.ivars().display.removeFromSuperview();
        self.addSubview(&self.ivars().entry);
        for constraint in [
            self.ivars()
                .entry
                .leadingAnchor()
                .constraintEqualToAnchor(&self.leadingAnchor()),
            self.ivars()
                .entry
                .trailingAnchor()
                .constraintEqualToAnchor(&self.trailingAnchor()),
            self.ivars()
                .entry
                .centerYAnchor()
                .constraintEqualToAnchor(&self.centerYAnchor()),
            self.ivars()
                .entry
                .heightAnchor()
                .constraintEqualToConstant(self.ivars().entry.intrinsicContentSize().height),
        ] {
            constraint.setActive(true);
        }
        self.layoutSubtreeIfNeeded();
        assert!(
            self.window()
                .expect("number picker must be attached before editing")
                .makeFirstResponder(Some(&self.ivars().entry)),
            "number picker entry must accept first responder"
        );
    }

    fn commit_edit(&self) {
        if !self.ivars().editing.get() {
            return;
        }
        let changed = parse_fraction(self.ivars().entry.stringValue().to_string().trim())
            .is_some_and(|value| self.set_value(value));
        if changed {
            (self.ivars().on_commit)(self.ivars().value.get());
        }
        self.end_edit_without_change();
    }

    fn end_edit_without_change(&self) {
        if !self.ivars().editing.replace(false) {
            return;
        }
        self.ivars().entry.removeFromSuperview();
        self.addSubview(&self.ivars().display);
        for constraint in [
            self.ivars()
                .display
                .leadingAnchor()
                .constraintEqualToAnchor(&self.leadingAnchor()),
            self.ivars()
                .display
                .trailingAnchor()
                .constraintEqualToAnchor(&self.trailingAnchor()),
            self.ivars()
                .display
                .topAnchor()
                .constraintEqualToAnchor(&self.topAnchor()),
            self.ivars()
                .display
                .bottomAnchor()
                .constraintEqualToAnchor(&self.bottomAnchor()),
        ] {
            constraint.setActive(true);
        }
        let preview = self.ivars().preview_value.take();
        self.refresh();
        // Text edits publish live values without replacing the committed value.
        // Cancellation (or invalid final text) must roll that preview back too.
        let value = self.ivars().value.get();
        if preview.is_some_and(|preview| preview != value) {
            (self.ivars().on_change)(value);
        }
    }
}

pub struct NumberPicker;

impl NumberPicker {
    pub fn builder(value: f64) -> NumberPickerBuilder {
        Self::fraction_builder(fraction_from_f64(value))
    }

    pub fn fraction_builder(value: Fraction) -> NumberPickerBuilder {
        NumberPickerBuilder {
            enabled: true,
            value,
            minimum: fraction_from_integer(DEFAULT_MINIMUM),
            maximum: fraction_from_integer(DEFAULT_MAXIMUM),
            drag_step: fraction_from_integer(1),
            digits: 2,
            prefix: String::new(),
            suffix: String::new(),
            rotating_prefix_symbol: None,
            rotation_offset_degrees: 0.0,
            on_change: None,
            on_commit: None,
        }
    }
}

pub struct NumberPickerBuilder {
    enabled: bool,
    value: Fraction,
    minimum: Fraction,
    maximum: Fraction,
    drag_step: Fraction,
    digits: usize,
    prefix: String,
    suffix: String,
    rotating_prefix_symbol: Option<String>,
    rotation_offset_degrees: f64,
    on_change: Option<Box<dyn Fn(Fraction)>>,
    on_commit: Option<Box<dyn Fn(Fraction)>>,
}

impl NumberPickerBuilder {
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn accepted_range(mut self, minimum: f64, maximum: f64) -> Self {
        self.minimum = fraction_from_f64(minimum);
        self.maximum = fraction_from_f64(maximum);
        self
    }

    pub fn minimum(mut self, value: f64) -> Self {
        self.minimum = fraction_from_f64(value);
        self
    }
    pub fn maximum(mut self, value: f64) -> Self {
        self.maximum = fraction_from_f64(value);
        self
    }
    pub fn drag_step(mut self, value: f64) -> Self {
        self.drag_step = positive_fraction_or(fraction_from_f64(value), self.drag_step);
        self
    }
    pub fn digits(mut self, value: usize) -> Self {
        self.digits = value;
        self
    }
    pub fn prefix(mut self, value: impl Into<String>) -> Self {
        self.prefix = format!("{} ", value.into());
        self
    }
    pub fn rotating_prefix_symbol(mut self, value: impl Into<String>) -> Self {
        self.rotating_prefix_symbol = Some(value.into());
        self
    }
    pub fn rotating_prefix_symbol_with_offset(
        mut self,
        value: impl Into<String>,
        offset_degrees: f64,
    ) -> Self {
        self.rotating_prefix_symbol = Some(value.into());
        self.rotation_offset_degrees = offset_degrees;
        self
    }
    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        self.suffix = format!(" {}", value.into());
        self
    }
    pub fn on_change(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_change = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }
    pub fn on_change_fraction(mut self, callback: impl Fn(Fraction) + 'static) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }
    pub fn on_commit(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_commit = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }
    pub fn on_commit_fraction(mut self, callback: impl Fn(Fraction) + 'static) -> Self {
        self.on_commit = Some(Box::new(callback));
        self
    }

    pub fn build(self, mtm: MainThreadMarker) -> Retained<NSView> {
        self.build_with_handle(mtm).widget
    }

    pub fn build_with_handle(self, mtm: MainThreadMarker) -> NumberPickerParts {
        let config = NumberConfig {
            minimum: self.minimum,
            maximum: self.maximum,
            drag_step: self.drag_step,
            drag_pixels: shrimply_components_core::number::DEFAULT_DRAG_PIXELS,
            digits: self.digits,
            fallback: if self.value == FRACTION_ZERO {
                FRACTION_ZERO
            } else {
                self.value
            },
        };
        let value = accepted_value(&config, self.value);
        let display = NSView::new(mtm);
        let background = NSTextField::labelWithString(&NSString::new(), mtm);
        background.setBordered(true);
        background.setBezeled(true);
        // This empty field only paints the bezel; its intrinsic width must not
        // constrain the numeric editor's width.
        background.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultLow,
            NSLayoutConstraintOrientation::Horizontal,
        );
        let rotating_icon = RotatingImageView::alloc(mtm).set_ivars(RotatingImageViewIvars {
            angle_degrees: Cell::new(f64::NAN),
        });
        let rotating_icon: Retained<RotatingImageView> =
            unsafe { msg_send![super(rotating_icon), initWithFrame: NSRect::ZERO] };
        if let Some(name) = self.rotating_prefix_symbol.as_deref() {
            rotating_icon.setImage(Some(&system_symbol(name, "Rotation")));
        } else {
            rotating_icon.setHidden(true);
        }
        let prefix = NSTextField::labelWithString(&NSString::from_str(self.prefix.trim()), mtm);
        prefix.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let value_label = NSTextField::labelWithString(&NSString::new(), mtm);
        value_label.setAlignment(NSTextAlignment::Right);
        let suffix = NSTextField::labelWithString(&NSString::from_str(self.suffix.trim()), mtm);
        suffix.setTextColor(Some(&NSColor::secondaryLabelColor()));
        for child in [
            &*background,
            rotating_icon.as_super().as_super().as_super(),
            &*prefix,
            &*value_label,
            &*suffix,
        ] {
            child.setTranslatesAutoresizingMaskIntoConstraints(false);
            display.addSubview(child);
        }
        let has_icon = !rotating_icon.isHidden();
        let has_prefix = !self.prefix.trim().is_empty();
        let has_suffix = !self.suffix.trim().is_empty();
        for constraint in [
            background
                .leadingAnchor()
                .constraintEqualToAnchor(&display.leadingAnchor()),
            background
                .trailingAnchor()
                .constraintEqualToAnchor(&display.trailingAnchor()),
            background
                .topAnchor()
                .constraintEqualToAnchor(&display.topAnchor()),
            background
                .bottomAnchor()
                .constraintEqualToAnchor(&display.bottomAnchor()),
            rotating_icon
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&display.leadingAnchor(), 8.0),
            rotating_icon
                .centerYAnchor()
                .constraintEqualToAnchor(&display.centerYAnchor()),
            rotating_icon
                .widthAnchor()
                .constraintEqualToConstant(if has_icon { 14.0 } else { 0.0 }),
            rotating_icon
                .heightAnchor()
                .constraintEqualToConstant(if has_icon { 14.0 } else { 0.0 }),
            prefix.leadingAnchor().constraintEqualToAnchor_constant(
                &rotating_icon.trailingAnchor(),
                if has_icon && has_prefix { 4.0 } else { 0.0 },
            ),
            prefix
                .centerYAnchor()
                .constraintEqualToAnchor(&display.centerYAnchor()),
            value_label
                .leadingAnchor()
                .constraintEqualToAnchor_constant(
                    &prefix.trailingAnchor(),
                    if has_prefix { 4.0 } else { 0.0 },
                ),
            value_label
                .centerYAnchor()
                .constraintEqualToAnchor(&display.centerYAnchor()),
            suffix.leadingAnchor().constraintEqualToAnchor_constant(
                &value_label.trailingAnchor(),
                if has_suffix { 4.0 } else { 0.0 },
            ),
            suffix
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&display.trailingAnchor(), -8.0),
            suffix
                .centerYAnchor()
                .constraintEqualToAnchor(&display.centerYAnchor()),
        ] {
            constraint.setActive(true);
        }
        value_label.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultLow,
            NSLayoutConstraintOrientation::Horizontal,
        );
        let entry = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
        entry.setAlignment(NSTextAlignment::Right);
        entry.setEditable(true);
        entry.setSelectable(true);
        entry.setEnabled(self.enabled);
        if !self.enabled {
            for label in [&prefix, &value_label, &suffix] {
                label.setTextColor(Some(&NSColor::disabledControlTextColor()));
            }
            background.setEnabled(false);
        }
        entry.setTranslatesAutoresizingMaskIntoConstraints(false);
        let view = NumberPickerView::alloc(mtm).set_ivars(NumberPickerIvars {
            enabled: self.enabled,
            config,
            value: Cell::new(value),
            display: display.clone(),
            rotating_icon: rotating_icon.clone(),
            rotation_offset_degrees: self.rotation_offset_degrees,
            value_label: value_label.clone(),
            entry: entry.clone(),
            on_change: self
                .on_change
                .map_or_else(|| Rc::new(|_| {}) as NumberCallback, Rc::from),
            on_commit: self
                .on_commit
                .map_or_else(|| Rc::new(|_| {}) as NumberCallback, Rc::from),
            drag: Cell::new(NumberDrag::begin(value)),
            pointer_locked: Cell::new(false),
            editing: Cell::new(false),
            preview_value: Cell::new(None),
            tracking_area: RefCell::new(None),
        });
        let view: Retained<NumberPickerView> =
            unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
        unsafe {
            entry.setTarget(Some(&view));
            entry.setAction(Some(sel!(commitText:)));
            entry.setDelegate(Some(ProtocolObject::from_ref(&*view)));
        }
        display.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.addSubview(&display);
        for constraint in [
            display
                .leadingAnchor()
                .constraintEqualToAnchor(&view.leadingAnchor()),
            display
                .trailingAnchor()
                .constraintEqualToAnchor(&view.trailingAnchor()),
            display
                .topAnchor()
                .constraintEqualToAnchor(&view.topAnchor()),
            display
                .bottomAnchor()
                .constraintEqualToAnchor(&view.bottomAnchor()),
        ] {
            constraint.setActive(true);
        }
        let minimum_width = view
            .widthAnchor()
            .constraintGreaterThanOrEqualToConstant(NUMBER_MIN_WIDTH);
        minimum_width.setPriority(NSLayoutPriorityDefaultHigh);
        minimum_width.setActive(true);
        view.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultLow,
            NSLayoutConstraintOrientation::Horizontal,
        );
        view.heightAnchor()
            .constraintEqualToConstant(28.0)
            .setActive(true);
        view.refresh();
        NumberPickerParts {
            widget: view.clone().into_super(),
            handle: NumberPickerHandle { view },
        }
    }
}

pub struct NumberPickerParts {
    pub widget: Retained<NSView>,
    pub handle: NumberPickerHandle,
}

#[derive(Clone)]
pub struct NumberPickerHandle {
    view: Retained<NumberPickerView>,
}

impl NumberPickerHandle {
    pub fn set_f64(&self, value: f64) {
        if !self.view.ivars().editing.get() {
            self.view.set_value_silently(fraction_from_f64(value));
        }
    }
    pub fn value(&self) -> Fraction {
        self.view.ivars().value.get()
    }
}

// Widget callbacks must not own the same views that own those callbacks.
// Public handles remain strong; only the internal links between siblings are weak.
fn load_number_handles<const N: usize>(
    shared: &RefCell<Option<[Weak<NumberPickerView>; N]>>,
) -> Option<[NumberPickerHandle; N]> {
    let shared = shared.borrow();
    let views = shared.as_ref()?;
    let mut handles = Vec::with_capacity(N);
    for view in views {
        handles.push(NumberPickerHandle { view: view.load()? });
    }
    Some(handles.try_into().ok().expect("number handle count"))
}

pub struct Number2Picker;

impl Number2Picker {
    pub fn builder(first: f64, second: f64) -> Number2PickerBuilder {
        Number2PickerBuilder {
            first: NumberPicker::builder(first),
            second: NumberPicker::builder(second),
            initial: [fraction_from_f64(first), fraction_from_f64(second)],
            lock: false,
            on_change: None,
            on_commit: None,
        }
    }
}

pub struct Number2PickerBuilder {
    first: NumberPickerBuilder,
    second: NumberPickerBuilder,
    initial: [Fraction; 2],
    lock: bool,
    on_change: Option<PairCallback>,
    on_commit: Option<Box<dyn Fn()>>,
}

impl Number2PickerBuilder {
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.first = self.first.enabled(enabled);
        self.second = self.second.enabled(enabled);
        self
    }

    pub fn minimum(mut self, value: f64) -> Self {
        self.first = self.first.minimum(value);
        self.second = self.second.minimum(value);
        self
    }
    pub fn maximum(mut self, value: f64) -> Self {
        self.first = self.first.maximum(value);
        self.second = self.second.maximum(value);
        self
    }
    pub fn drag_step(mut self, value: f64) -> Self {
        self.first = self.first.drag_step(value);
        self.second = self.second.drag_step(value);
        self
    }
    pub fn digits(mut self, value: usize) -> Self {
        self.first = self.first.digits(value);
        self.second = self.second.digits(value);
        self
    }
    pub fn first_prefix(mut self, value: impl Into<String>) -> Self {
        self.first = self.first.prefix(value);
        self
    }
    pub fn second_prefix(mut self, value: impl Into<String>) -> Self {
        self.second = self.second.prefix(value);
        self
    }
    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.first = self.first.unit_name(value.clone());
        self.second = self.second.unit_name(value);
        self
    }
    pub fn enable_lock(mut self) -> Self {
        self.lock = true;
        self
    }
    pub fn on_change(mut self, callback: impl Fn([f64; 2], usize) + 'static) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }
    pub fn on_commit(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_commit = Some(Box::new(callback));
        self
    }

    pub fn build_with_handles(self, mtm: MainThreadMarker) -> Number2PickerParts {
        let enabled = self.first.enabled && self.second.enabled;
        let handles = Rc::new(RefCell::new(None::<[Weak<NumberPickerView>; 2]>));
        let locked = Rc::new(Cell::new(self.lock));
        let ratio = Rc::new(Cell::new(pair_ratio(self.initial[0], self.initial[1])));
        let callback: Rc<dyn Fn([f64; 2], usize)> = match self.on_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_, _| {}),
        };
        let commit: Rc<dyn Fn()> = match self.on_commit {
            Some(commit) => Rc::from(commit),
            None => Rc::new(|| {}),
        };
        let first = self
            .first
            .on_change_fraction({
                let handles = handles.clone();
                let locked = locked.clone();
                let ratio = ratio.clone();
                let callback = callback.clone();
                move |value| {
                    if let Some(handles) = load_number_handles(&handles) {
                        if locked.get() {
                            handles[1]
                                .set_f64(fraction_as_f64(locked_pair(0, value, ratio.get())[1]));
                        }
                        let mut values = handles
                            .clone()
                            .map(|handle| fraction_as_f64(handle.value()));
                        values[0] = fraction_as_f64(value);
                        callback(values, 0);
                    }
                }
            })
            .on_commit_fraction({
                let commit = commit.clone();
                move |_| commit()
            })
            .build_with_handle(mtm);
        let second = self
            .second
            .on_change_fraction({
                let handles = handles.clone();
                let locked = locked.clone();
                let ratio = ratio.clone();
                let callback = callback.clone();
                move |value| {
                    if let Some(handles) = load_number_handles(&handles) {
                        if locked.get() {
                            handles[0]
                                .set_f64(fraction_as_f64(locked_pair(1, value, ratio.get())[0]));
                        }
                        let mut values = handles
                            .clone()
                            .map(|handle| fraction_as_f64(handle.value()));
                        values[1] = fraction_as_f64(value);
                        callback(values, 1);
                    }
                }
            })
            .on_commit_fraction(move |_| commit())
            .build_with_handle(mtm);
        let pair_handles = [first.handle.clone(), second.handle.clone()];
        handles.replace(Some(
            pair_handles
                .each_ref()
                .map(|handle| Weak::new(&*handle.view)),
        ));
        let lock = if self.lock {
            let button = unsafe {
                NSButton::buttonWithImage_target_action(&lock_symbol(true), None, None, mtm)
            };
            button.setEnabled(enabled);
            button.setButtonType(NSButtonType::PushOnPushOff);
            button.setBordered(false);
            button.setState(NSControlStateValueOn);
            button.setToolTip(Some(&NSString::from_str("Unlock ratio")));
            action::attach(
                &button,
                move |control| {
                    let button = control
                        .downcast_ref::<NSButton>()
                        .expect("pair lock sender");
                    let active = button.state() == NSControlStateValueOn;
                    locked.set(active);
                    button.setImage(Some(&lock_symbol(active)));
                    button.setToolTip(Some(&NSString::from_str(if active {
                        "Unlock ratio"
                    } else {
                        "Lock ratio"
                    })));
                    if active && let Some(handles) = load_number_handles(&handles) {
                        ratio.set(pair_ratio(handles[0].value(), handles[1].value()));
                    }
                },
                mtm,
            );
            Some(button)
        } else {
            None
        };
        let row = number_row(&[&first.widget, &second.widget], lock.as_deref(), mtm);
        Number2PickerParts {
            widget: row,
            first: pair_handles[0].clone(),
            second: pair_handles[1].clone(),
        }
    }
}

pub struct Number2PickerParts {
    pub widget: Retained<NSView>,
    pub first: NumberPickerHandle,
    pub second: NumberPickerHandle,
}

pub struct Number3Picker;

impl Number3Picker {
    pub fn builder(values: [f64; 3]) -> Number3PickerBuilder {
        Number3PickerBuilder {
            builders: values.map(NumberPicker::builder),
            initial: values.map(fraction_from_f64),
            prefixes: [String::new(), String::new(), String::new()],
            lock: false,
            on_change: None,
            on_commit: None,
        }
    }
}

pub struct Number3PickerBuilder {
    builders: [NumberPickerBuilder; 3],
    initial: [Fraction; 3],
    prefixes: [String; 3],
    lock: bool,
    on_change: Option<TripleCallback>,
    on_commit: Option<Box<dyn Fn()>>,
}

impl Number3PickerBuilder {
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.builders = self.builders.map(|builder| builder.enabled(enabled));
        self
    }

    pub fn minimum(mut self, value: f64) -> Self {
        self.builders = self.builders.map(|builder| builder.minimum(value));
        self
    }
    pub fn maximum(mut self, value: f64) -> Self {
        self.builders = self.builders.map(|builder| builder.maximum(value));
        self
    }
    pub fn drag_step(mut self, value: f64) -> Self {
        self.builders = self.builders.map(|builder| builder.drag_step(value));
        self
    }
    pub fn digits(mut self, value: usize) -> Self {
        self.builders = self.builders.map(|builder| builder.digits(value));
        self
    }
    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.builders = self
            .builders
            .map(|builder| builder.unit_name(value.clone()));
        self
    }
    pub fn prefixes(mut self, values: [&str; 3]) -> Self {
        self.prefixes = values.map(str::to_string);
        self
    }
    pub fn enable_lock(mut self) -> Self {
        self.lock = true;
        self
    }
    pub fn on_change(mut self, callback: impl Fn([f64; 3], usize) + 'static) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }
    pub fn on_commit(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_commit = Some(Box::new(callback));
        self
    }
    pub fn build_with_handles(mut self, mtm: MainThreadMarker) -> Number3PickerParts {
        let enabled = self.builders.iter().all(|builder| builder.enabled);
        let shared = Rc::new(RefCell::new(None::<[Weak<NumberPickerView>; 3]>));
        let locked = Rc::new(Cell::new(self.lock));
        let ratios = Rc::new(Cell::new(triple_ratios(self.initial)));
        let callback: Rc<dyn Fn([f64; 3], usize)> = match self.on_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_, _| {}),
        };
        let commit: Rc<dyn Fn()> = match self.on_commit {
            Some(commit) => Rc::from(commit),
            None => Rc::new(|| {}),
        };
        let mut parts = Vec::new();
        for component in 0..3 {
            let shared = shared.clone();
            let locked = locked.clone();
            let ratios = ratios.clone();
            let callback = callback.clone();
            let builder =
                std::mem::replace(&mut self.builders[component], NumberPicker::builder(0.0))
                    .prefix(self.prefixes[component].clone())
                    .on_change_fraction(move |value| {
                        if let Some(handles) = load_number_handles(&shared) {
                            if locked.get() {
                                let next = locked_triple(component, value, ratios.get());
                                for index in 0..3 {
                                    if index != component {
                                        handles[index].set_f64(fraction_as_f64(next[index]));
                                    }
                                }
                            }
                            let mut values = handles
                                .clone()
                                .map(|handle| fraction_as_f64(handle.value()));
                            // During text editing, the callback's preview value precedes
                            // the handle's committed value. Publish the complete live vector.
                            values[component] = fraction_as_f64(value);
                            callback(values, component);
                        }
                    })
                    .on_commit_fraction({
                        let commit = commit.clone();
                        move |_| commit()
                    })
                    .build_with_handle(mtm);
            parts.push(builder);
        }
        let [first, second, third]: [NumberPickerParts; 3] =
            parts.try_into().ok().expect("three number parts");
        let handles = [
            first.handle.clone(),
            second.handle.clone(),
            third.handle.clone(),
        ];
        shared.replace(Some(
            handles.each_ref().map(|handle| Weak::new(&*handle.view)),
        ));
        let lock = if self.lock {
            let button = unsafe {
                NSButton::buttonWithImage_target_action(&lock_symbol(true), None, None, mtm)
            };
            button.setEnabled(enabled);
            button.setButtonType(NSButtonType::PushOnPushOff);
            button.setBordered(false);
            button.setState(NSControlStateValueOn);
            button.setToolTip(Some(&NSString::from_str("Unlock ratio")));
            action::attach(
                &button,
                move |control| {
                    let button = control
                        .downcast_ref::<NSButton>()
                        .expect("vector lock sender");
                    let active = button.state() == NSControlStateValueOn;
                    locked.set(active);
                    button.setImage(Some(&lock_symbol(active)));
                    button.setToolTip(Some(&NSString::from_str(if active {
                        "Unlock ratio"
                    } else {
                        "Lock ratio"
                    })));
                    if active && let Some(handles) = load_number_handles(&shared) {
                        ratios.set(triple_ratios(handles.clone().map(|handle| handle.value())));
                    }
                },
                mtm,
            );
            Some(button)
        } else {
            None
        };
        let row = number_row(
            &[&first.widget, &second.widget, &third.widget],
            lock.as_deref(),
            mtm,
        );
        Number3PickerParts {
            widget: row,
            handles,
        }
    }
}

pub struct Number3PickerParts {
    pub widget: Retained<NSView>,
    pub handles: [NumberPickerHandle; 3],
}

fn number_row(
    fields: &[&NSView],
    lock: Option<&NSButton>,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    assert!(!fields.is_empty(), "number row needs at least one field");
    let row = NSView::new(mtm);
    let mut previous = lock.map(|button| button.as_super().as_super());
    if let Some(lock) = lock {
        lock.setTranslatesAutoresizingMaskIntoConstraints(false);
        lock.setContentHuggingPriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityRequired,
            NSLayoutConstraintOrientation::Horizontal,
        );
        row.addSubview(lock);
        for constraint in [
            lock.leadingAnchor()
                .constraintEqualToAnchor(&row.leadingAnchor()),
            lock.centerYAnchor()
                .constraintEqualToAnchor(&row.centerYAnchor()),
            lock.widthAnchor()
                .constraintEqualToConstant(lock.intrinsicContentSize().width),
        ] {
            constraint.setActive(true);
        }
    }
    for field in fields {
        field.setTranslatesAutoresizingMaskIntoConstraints(false);
        row.addSubview(field);
        let leading = if let Some(previous) = previous {
            field.leadingAnchor().constraintEqualToAnchor_constant(
                &previous.trailingAnchor(),
                f64::from(shrimply_components_core::layout::CONTROL_ROW_GAP),
            )
        } else {
            field
                .leadingAnchor()
                .constraintEqualToAnchor(&row.leadingAnchor())
        };
        for constraint in [
            leading,
            field.topAnchor().constraintEqualToAnchor(&row.topAnchor()),
            field
                .bottomAnchor()
                .constraintEqualToAnchor(&row.bottomAnchor()),
            field
                .widthAnchor()
                .constraintEqualToAnchor(&fields[0].widthAnchor()),
        ] {
            constraint.setActive(true);
        }
        previous = Some(field);
    }
    fields
        .last()
        .expect("number row field")
        .trailingAnchor()
        .constraintEqualToAnchor(&row.trailingAnchor())
        .setActive(true);
    row
}

fn lock_symbol(locked: bool) -> Retained<NSImage> {
    let name = if locked { "lock.fill" } else { "lock.open" };
    system_symbol(name, if locked { "Locked" } else { "Unlocked" })
}

fn system_symbol(name: &str, label: &str) -> Retained<NSImage> {
    crate::controls::symbol(name, label)
}
