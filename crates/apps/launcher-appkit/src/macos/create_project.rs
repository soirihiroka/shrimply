use block2::StackBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSControl, NSFont, NSPopUpButton, NSStepper,
    NSTextAlignment, NSTextField, NSView, NSWindow,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};
use shrimply_component_core::project_settings::{CUSTOM_PRESET_INDEX, ProjectSettings};
use shrimply_math_core::Fraction;
use shrimply_project_core::{
    COMMON_FRAME_RATES, CanvasSize, MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION, PROJECT_PRESETS,
};
use std::cell::{Cell, OnceCell};

pub struct Request {
    pub name: String,
    pub canvas_size: CanvasSize,
    pub fps: Fraction,
}

struct DialogIvars {
    settings: Cell<ProjectSettings>,
    updating: Cell<bool>,
    preset: OnceCell<Retained<NSPopUpButton>>,
    width: OnceCell<Retained<NSTextField>>,
    width_stepper: OnceCell<Retained<NSStepper>>,
    height: OnceCell<Retained<NSTextField>>,
    height_stepper: OnceCell<Retained<NSStepper>>,
    fps: OnceCell<Retained<NSPopUpButton>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DialogIvars]
    struct Dialog;

    unsafe impl NSObjectProtocol for Dialog {}

    impl Dialog {
        #[unsafe(method(presetChanged:))]
        fn preset_changed(&self, sender: &NSPopUpButton) {
            let index = usize::try_from(sender.indexOfSelectedItem())
                .expect("preset index must be non-negative");
            if index == CUSTOM_PRESET_INDEX {
                return;
            }
            let mut settings = self.ivars().settings.get();
            settings.select_preset(index);
            self.ivars().settings.set(settings);
            self.sync_controls();
        }

        #[unsafe(method(widthChanged:))]
        fn width_changed(&self, sender: &NSControl) {
            if self.ivars().updating.get() {
                return;
            }
            let width = u32::try_from(sender.integerValue())
                .unwrap_or(MIN_CANVAS_DIMENSION)
                .clamp(MIN_CANVAS_DIMENSION, MAX_CANVAS_DIMENSION);
            let mut settings = self.ivars().settings.get();
            settings.set_width(width);
            self.ivars().settings.set(settings);
            self.sync_controls();
        }

        #[unsafe(method(heightChanged:))]
        fn height_changed(&self, sender: &NSControl) {
            if self.ivars().updating.get() {
                return;
            }
            let height = u32::try_from(sender.integerValue())
                .unwrap_or(MIN_CANVAS_DIMENSION)
                .clamp(MIN_CANVAS_DIMENSION, MAX_CANVAS_DIMENSION);
            let mut settings = self.ivars().settings.get();
            settings.set_height(height);
            self.ivars().settings.set(settings);
            self.sync_controls();
        }

        #[unsafe(method(frameRateChanged:))]
        fn frame_rate_changed(&self, sender: &NSPopUpButton) {
            if self.ivars().updating.get() {
                return;
            }
            let index = usize::try_from(sender.indexOfSelectedItem())
                .expect("frame-rate index must be non-negative");
            let mut settings = self.ivars().settings.get();
            settings.set_frame_rate(index);
            self.ivars().settings.set(settings);
            self.sync_controls();
        }
    }
);

impl Dialog {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DialogIvars {
            settings: Cell::new(ProjectSettings::default()),
            updating: Cell::new(false),
            preset: OnceCell::new(),
            width: OnceCell::new(),
            width_stepper: OnceCell::new(),
            height: OnceCell::new(),
            height_stepper: OnceCell::new(),
            fps: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn sync_controls(&self) {
        let controls = self.ivars();
        let settings = controls.settings.get();
        controls.updating.set(true);
        controls
            .preset
            .get()
            .expect("preset control must exist")
            .selectItemAtIndex(settings.preset as isize);
        controls
            .width
            .get()
            .expect("width control must exist")
            .setIntegerValue(settings.width as isize);
        controls
            .width_stepper
            .get()
            .expect("width stepper must exist")
            .setIntegerValue(settings.width as isize);
        controls
            .height
            .get()
            .expect("height control must exist")
            .setIntegerValue(settings.height as isize);
        controls
            .height_stepper
            .get()
            .expect("height stepper must exist")
            .setIntegerValue(settings.height as isize);
        controls
            .fps
            .get()
            .expect("frame-rate control must exist")
            .selectItemAtIndex(settings.frame_rate as isize);
        controls.updating.set(false);
    }
}

pub fn show(parent: &NSWindow, mtm: MainThreadMarker) -> Option<Request> {
    let dialog = Dialog::new(mtm);
    let form = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(440.0, 250.0)),
    );

    let name_label = NSTextField::labelWithString(ns_string!("Project Name"), mtm);
    name_label.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    name_label.setFrame(NSRect::new(
        NSPoint::new(0.0, 218.0),
        NSSize::new(120.0, 22.0),
    ));
    form.addSubview(&name_label);
    let name = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(120.0, 214.0), NSSize::new(320.0, 28.0)),
    );
    name.setStringValue(ns_string!("Untitled Project"));
    form.addSubview(&name);

    let preset_label = NSTextField::labelWithString(ns_string!("Preset"), mtm);
    preset_label.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    preset_label.setFrame(NSRect::new(
        NSPoint::new(0.0, 174.0),
        NSSize::new(120.0, 22.0),
    ));
    form.addSubview(&preset_label);
    let preset = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(120.0, 168.0), NSSize::new(320.0, 32.0)),
        false,
    );
    for item in PROJECT_PRESETS {
        preset.addItemWithTitle(&NSString::from_str(item.label));
    }
    preset.addItemWithTitle(ns_string!("Custom"));
    unsafe {
        preset.setTarget(Some(&*dialog));
        preset.setAction(Some(sel!(presetChanged:)));
    }
    form.addSubview(&preset);

    let settings_label = NSTextField::labelWithString(ns_string!("Project Settings"), mtm);
    settings_label.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    settings_label.setFrame(NSRect::new(
        NSPoint::new(0.0, 128.0),
        NSSize::new(200.0, 22.0),
    ));
    form.addSubview(&settings_label);

    let width_label = NSTextField::labelWithString(ns_string!("Width"), mtm);
    width_label.setFrame(NSRect::new(
        NSPoint::new(18.0, 88.0),
        NSSize::new(102.0, 22.0),
    ));
    form.addSubview(&width_label);
    let width = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(250.0, 84.0), NSSize::new(160.0, 28.0)),
    );
    width.setAlignment(NSTextAlignment::Right);
    unsafe {
        width.setTarget(Some(&*dialog));
        width.setAction(Some(sel!(widthChanged:)));
    }
    form.addSubview(&width);
    let width_stepper = NSStepper::initWithFrame(
        NSStepper::alloc(mtm),
        NSRect::new(NSPoint::new(414.0, 84.0), NSSize::new(20.0, 28.0)),
    );
    width_stepper.setMinValue(f64::from(MIN_CANVAS_DIMENSION));
    width_stepper.setMaxValue(f64::from(MAX_CANVAS_DIMENSION));
    width_stepper.setIncrement(1.0);
    unsafe {
        width_stepper.setTarget(Some(&*dialog));
        width_stepper.setAction(Some(sel!(widthChanged:)));
    }
    form.addSubview(&width_stepper);

    let height_label = NSTextField::labelWithString(ns_string!("Height"), mtm);
    height_label.setFrame(NSRect::new(
        NSPoint::new(18.0, 50.0),
        NSSize::new(102.0, 22.0),
    ));
    form.addSubview(&height_label);
    let height = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(250.0, 46.0), NSSize::new(160.0, 28.0)),
    );
    height.setAlignment(NSTextAlignment::Right);
    unsafe {
        height.setTarget(Some(&*dialog));
        height.setAction(Some(sel!(heightChanged:)));
    }
    form.addSubview(&height);
    let height_stepper = NSStepper::initWithFrame(
        NSStepper::alloc(mtm),
        NSRect::new(NSPoint::new(414.0, 46.0), NSSize::new(20.0, 28.0)),
    );
    height_stepper.setMinValue(f64::from(MIN_CANVAS_DIMENSION));
    height_stepper.setMaxValue(f64::from(MAX_CANVAS_DIMENSION));
    height_stepper.setIncrement(1.0);
    unsafe {
        height_stepper.setTarget(Some(&*dialog));
        height_stepper.setAction(Some(sel!(heightChanged:)));
    }
    form.addSubview(&height_stepper);

    let fps_label = NSTextField::labelWithString(ns_string!("Frame Rate"), mtm);
    fps_label.setFrame(NSRect::new(
        NSPoint::new(18.0, 12.0),
        NSSize::new(102.0, 22.0),
    ));
    form.addSubview(&fps_label);
    let fps = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(250.0, 6.0), NSSize::new(190.0, 32.0)),
        false,
    );
    for rate in COMMON_FRAME_RATES {
        fps.addItemWithTitle(&NSString::from_str(rate.label));
    }
    unsafe {
        fps.setTarget(Some(&*dialog));
        fps.setAction(Some(sel!(frameRateChanged:)));
    }
    form.addSubview(&fps);

    dialog
        .ivars()
        .preset
        .set(preset)
        .unwrap_or_else(|_| panic!("preset control must only be created once"));
    dialog
        .ivars()
        .width
        .set(width)
        .unwrap_or_else(|_| panic!("width control must only be created once"));
    dialog
        .ivars()
        .width_stepper
        .set(width_stepper)
        .unwrap_or_else(|_| panic!("width stepper must only be created once"));
    dialog
        .ivars()
        .height
        .set(height)
        .unwrap_or_else(|_| panic!("height control must only be created once"));
    dialog
        .ivars()
        .height_stepper
        .set(height_stepper)
        .unwrap_or_else(|_| panic!("height stepper must only be created once"));
    dialog
        .ivars()
        .fps
        .set(fps)
        .unwrap_or_else(|_| panic!("frame-rate control must only be created once"));
    dialog.sync_controls();

    let alert = NSAlert::new(mtm);
    alert.setMessageText(ns_string!("Create Project"));
    alert.setAccessoryView(Some(&form));
    alert.addButtonWithTitle(ns_string!("Create Project"));
    alert.addButtonWithTitle(ns_string!("Cancel"));
    alert.layout();
    alert.window().makeFirstResponder(Some(&name));

    loop {
        let completion = StackBlock::new(move |response| {
            NSApplication::sharedApplication(mtm).stopModalWithCode(response)
        });
        alert.beginSheetModalForWindow_completionHandler(parent, Some(&completion));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return None;
        }
        let project_name = name.stringValue().to_string().trim().to_string();
        let width = u32::try_from(
            dialog
                .ivars()
                .width
                .get()
                .expect("width control must exist")
                .integerValue(),
        )
        .ok()
        .filter(|width| (MIN_CANVAS_DIMENSION..=MAX_CANVAS_DIMENSION).contains(width));
        let height = u32::try_from(
            dialog
                .ivars()
                .height
                .get()
                .expect("height control must exist")
                .integerValue(),
        )
        .ok()
        .filter(|height| (MIN_CANVAS_DIMENSION..=MAX_CANVAS_DIMENSION).contains(height));
        let frame_rate = usize::try_from(
            dialog
                .ivars()
                .fps
                .get()
                .expect("frame-rate control must exist")
                .indexOfSelectedItem(),
        )
        .ok()
        .and_then(|index| COMMON_FRAME_RATES.get(index));
        let Some((width, height, frame_rate)) = width
            .zip(height)
            .zip(frame_rate)
            .map(|((width, height), frame_rate)| (width, height, frame_rate))
        else {
            alert.setInformativeText(ns_string!("Invalid project settings."));
            continue;
        };
        if project_name.is_empty() {
            alert.setInformativeText(ns_string!("Project name must not be empty."));
            continue;
        }
        return Some(Request {
            name: project_name,
            canvas_size: CanvasSize { width, height },
            fps: frame_rate.value,
        });
    }
}
