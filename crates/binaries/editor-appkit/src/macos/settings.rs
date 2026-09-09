use super::Editor;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAutoresizingMaskOptions, NSBackingStoreType, NSButton,
    NSColor, NSColorWell, NSControl, NSFont, NSFontManager, NSImage, NSModalResponseOK,
    NSOpenPanel, NSPopUpButton, NSScrollView, NSStepper, NSTabViewController,
    NSTabViewControllerTabStyle, NSTabViewItem, NSTextAlignment, NSTextField, NSView,
    NSViewController, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTabbingMode,
    NSWindowToolbarStyle,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, ns_string};
use shrimply_editor_state::preferences::{self, PreferenceId, PreferenceValue, SharedPreferences};
use std::collections::BTreeMap;

const PANE_WIDTH: f64 = 660.0;
const PANE_HEIGHT: f64 = 500.0;
const CONTENT_MARGIN: f64 = 24.0;
const CONTROL_X: f64 = 285.0;
const CONTROL_WIDTH: f64 = PANE_WIDTH - CONTROL_X - CONTENT_MARGIN;
const ROW_HEIGHT: f64 = 38.0;
const SECTION_HEIGHT: f64 = 30.0;
const LABEL_HEIGHT: f64 = 22.0;
const CONTROL_HEIGHT: f64 = 26.0;
const CONTROL_Y_OFFSET: f64 = 2.0;
const BLENDER_CLEAR_WIDTH: f64 = 64.0;
const BLENDER_BUTTON_GAP: f64 = 8.0;
const SERVER_BUTTON_WIDTH: f64 = 90.0;
const SERVER_BUTTON_GAP: f64 = 8.0;
const FEATURES_HEIGHT: f64 = 44.0;
const INTEGRATIONS_HEIGHT: f64 = 720.0;
const APPEARANCE_HEIGHT: f64 = 620.0;
const STEPPER_WIDTH: f64 = 20.0;
const STEPPER_GAP: f64 = 8.0;
const STEPPER_TAG_OFFSET: isize = 100;

const TAG_CAPTION_FONT_SIZE: isize = 1;
const TAG_DEFAULT_VISUAL_DURATION: isize = 2;
const TAG_TIMELINE_SNAP_RADIUS: isize = 3;
const TAG_PREVIEW_PADDING: isize = 4;
const TAG_PREVIEW_SHADOW: isize = 5;
const TAG_DECODER_POOL_SIZE: isize = 6;
const TAG_UPSAMPLE: isize = 7;
const TAG_DOWNSAMPLE: isize = 8;
const TAG_BLENDER_CHOOSE: isize = 201;
const TAG_BLENDER_CLEAR: isize = 202;
const TAG_SERVER_SELECTOR: isize = 210;
const TAG_SERVER_URL: isize = 211;
const TAG_SERVER_REMOVE: isize = 212;
const TAG_SERVER_COMPATIBILITY: isize = 213;
const TAG_SERVER_VERSION: isize = 214;
const TAG_SERVER_PROTOCOL: isize = 215;
const TAG_SERVER_FEATURES: isize = 216;
const TAG_SERVER_DEVICE: isize = 217;
const TAG_SERVER_TORCH: isize = 218;
const TAG_SERVER_CUDA: isize = 219;
const TAG_SERVER_JOBS: isize = 220;
const TAG_SERVER_RESERVATIONS: isize = 221;
const TAG_SERVER_WORKERS: isize = 222;

pub(super) type ServerProbe = (
    String,
    Result<preferences::ComputeServerPresentation, String>,
);

fn numeric_preference(tag: isize) -> PreferenceId {
    match tag {
        TAG_CAPTION_FONT_SIZE => PreferenceId::CaptionFontSize,
        TAG_DEFAULT_VISUAL_DURATION => PreferenceId::DefaultVisualDuration,
        TAG_TIMELINE_SNAP_RADIUS => PreferenceId::TimelineSnapRadius,
        TAG_PREVIEW_PADDING => PreferenceId::PreviewPadding,
        TAG_PREVIEW_SHADOW => PreferenceId::PreviewShadowSize,
        TAG_DECODER_POOL_SIZE => PreferenceId::TemporalDecoderPoolSize,
        _ => panic!("unknown numeric preference control"),
    }
}

pub(super) fn change_numeric(
    store: &SharedPreferences,
    sender: &NSControl,
) -> Result<(), &'static str> {
    use shrimply_math_core::{fraction_from_integer, fraction_round_nonnegative_u64};
    let is_stepper = sender.tag() >= STEPPER_TAG_OFFSET;
    let tag = if is_stepper {
        sender.tag() - STEPPER_TAG_OFFSET
    } else {
        sender.tag()
    };
    let id = numeric_preference(tag);
    let range = preferences::integer_range(id).expect("numeric preference range");
    let value = if is_stepper {
        Some(sender.integerValue() as i64)
    } else {
        shrimply_components_core::number::parse_fraction(sender.stringValue().to_string().trim())
            .map(|value| {
                let value = value.clamp(
                    fraction_from_integer(range.minimum) / fraction_from_integer(range.scale),
                    fraction_from_integer(range.maximum) / fraction_from_integer(range.scale),
                );
                fraction_round_nonnegative_u64(value * fraction_from_integer(range.scale)) as i64
            })
    };
    let result = value.ok_or("Enter a valid number.").and_then(|value| {
        preferences::set_value(
            store,
            id,
            PreferenceValue::Integer(value.clamp(range.minimum, range.maximum)),
        )
    });
    let PreferenceValue::Integer(value) = preferences::value(store, id) else {
        panic!("numeric preference must store an integer");
    };
    // The retained settings window owns this pane throughout its control's main-thread action.
    let content = unsafe { sender.superview() }.expect("numeric preference pane");
    tagged::<NSTextField>(&content, tag, "numeric field")
        .setDoubleValue(value as f64 / range.scale as f64);
    tagged::<NSStepper>(&content, tag + STEPPER_TAG_OFFSET, "numeric stepper")
        .setIntegerValue(value as isize);
    result
}

pub(super) fn change_filter(
    store: &SharedPreferences,
    sender: &NSPopUpButton,
) -> Result<(), &'static str> {
    let id = match sender.tag() {
        TAG_UPSAMPLE => PreferenceId::PreviewUpsampleMethod,
        TAG_DOWNSAMPLE => PreferenceId::PreviewDownsampleMethod,
        _ => panic!("unknown preview filter control"),
    };
    let result = preferences::set_value(
        store,
        id,
        PreferenceValue::Integer(sender.indexOfSelectedItem() as i64),
    );
    let PreferenceValue::Integer(value) = preferences::value(store, id) else {
        panic!("preview filter must store an integer");
    };
    sender.selectItemAtIndex(value as isize);
    result
}

pub(super) fn show(editor: &Editor) -> objc2::rc::Retained<NSWindow> {
    let mtm = editor.mtm();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(PANE_WIDTH, PANE_HEIGHT)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(ns_string!("Appearance"));
    window.setTabbingMode(NSWindowTabbingMode::Disallowed);
    window.setToolbarStyle(NSWindowToolbarStyle::Preference);
    for button in [
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = window.standardWindowButton(button) {
            button.setEnabled(false);
        }
    }

    let store = &editor
        .ivars()
        .session
        .get()
        .expect("project loaded")
        .preferences;
    let snapshot = preferences::snapshot(store);

    let (appearance_root, appearance) = scrolling_pane(mtm, APPEARANCE_HEIGHT);
    let mut y = APPEARANCE_HEIGHT - CONTENT_MARGIN - LABEL_HEIGHT;
    section(&appearance, "Captions", &mut y, mtm);
    numeric_row(
        &appearance,
        editor,
        "Font size",
        TAG_CAPTION_FONT_SIZE,
        &mut y,
    );
    color_row(
        &appearance,
        editor,
        snapshot.caption_background_color,
        &mut y,
        mtm,
    );
    section(&appearance, "Text", &mut y, mtm);
    font_row(
        &appearance,
        editor,
        snapshot.default_text_font_family.name(),
        &mut y,
        mtm,
    );
    section(&appearance, "Preview", &mut y, mtm);
    numeric_row(
        &appearance,
        editor,
        "Padding (pixels)",
        TAG_PREVIEW_PADDING,
        &mut y,
    );
    numeric_row(
        &appearance,
        editor,
        "Shadow size (pixels)",
        TAG_PREVIEW_SHADOW,
        &mut y,
    );
    for (title, tag, id, choices, help) in [
        (
            "Preview upsample method",
            TAG_UPSAMPLE,
            PreferenceId::PreviewUpsampleMethod,
            &["Nearest", "Bilinear"][..],
            "Filter used when the preview is larger than the video",
        ),
        (
            "Preview downsample method",
            TAG_DOWNSAMPLE,
            PreferenceId::PreviewDownsampleMethod,
            &["Nearest", "Bilinear", "Trilinear"][..],
            "Filter used when the preview is smaller than the video",
        ),
    ] {
        label(&appearance, title, y, mtm);
        let popup = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(
                NSPoint::new(CONTROL_X, y - CONTROL_Y_OFFSET),
                NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
            ),
            false,
        );
        for choice in choices {
            popup.addItemWithTitle(&NSString::from_str(choice));
        }
        let PreferenceValue::Integer(value) = preferences::value(store, id) else {
            panic!("preview filter must store an integer");
        };
        popup.selectItemAtIndex(value as isize);
        popup.setTag(tag);
        popup.setToolTip(Some(&NSString::from_str(help)));
        unsafe {
            popup.setTarget(Some(editor));
            popup.setAction(Some(sel!(changePreviewFilter:)));
        }
        appearance.addSubview(&popup);
        y -= ROW_HEIGHT;
    }
    section(&appearance, "Timeline", &mut y, mtm);
    numeric_row(
        &appearance,
        editor,
        "Default visual duration (seconds)",
        TAG_DEFAULT_VISUAL_DURATION,
        &mut y,
    );
    numeric_row(
        &appearance,
        editor,
        "Snap attraction radius (pixels)",
        TAG_TIMELINE_SNAP_RADIUS,
        &mut y,
    );

    let performance = pane(mtm);
    let mut y = initial_y();
    section(&performance, "Performance", &mut y, mtm);
    numeric_row(
        &performance,
        editor,
        "Temporal decoder pool size",
        TAG_DECODER_POOL_SIZE,
        &mut y,
    );
    let help = NSTextField::wrappingLabelWithString(
        ns_string!(
            "Maximum number of open video decoder sessions per renderer. Lower values reduce memory use; higher values keep more clips ready for playback. Applies to preview and new exports."
        ),
        mtm,
    );
    help.setTextColor(Some(&NSColor::secondaryLabelColor()));
    help.setFrame(NSRect::new(
        NSPoint::new(CONTENT_MARGIN, y - FEATURES_HEIGHT),
        NSSize::new(
            PANE_WIDTH - CONTENT_MARGIN * 2.0,
            FEATURES_HEIGHT + LABEL_HEIGHT,
        ),
    ));
    performance.addSubview(&help);
    let (integrations_root, integrations) = scrolling_pane(mtm, INTEGRATIONS_HEIGHT);
    let mut y = INTEGRATIONS_HEIGHT - CONTENT_MARGIN - LABEL_HEIGHT;
    section(&integrations, "Integrations", &mut y, mtm);
    compute_server_rows(&integrations, editor, store, &mut y, mtm);
    section(&integrations, "Local Tools", &mut y, mtm);
    blender_row(
        &integrations,
        editor,
        snapshot.blender_binary.as_deref(),
        &mut y,
        mtm,
    );

    let tabs = NSTabViewController::new(mtm);
    tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
    tabs.setCanPropagateSelectedChildViewControllerTitle(true);
    for item in [
        tab("Appearance", "paintpalette", appearance_root, mtm),
        tab("Performance", "speedometer", performance, mtm),
        tab(
            "Integrations",
            "puzzlepiece.extension",
            integrations_root,
            mtm,
        ),
    ] {
        tabs.addTabViewItem(&item);
    }
    tabs.setSelectedTabViewItemIndex(0);
    window.setContentViewController(Some(&tabs));
    window.center();
    window.makeKeyAndOrderFront(None);
    window
}

fn compute_server_rows(
    content: &NSView,
    editor: &Editor,
    store: &SharedPreferences,
    y: &mut f64,
    mtm: MainThreadMarker,
) {
    let (urls, selected) = preferences::compute_servers(store);
    label(content, "Server", *y, mtm);
    let selector = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
        ),
        false,
    );
    selector.setTag(TAG_SERVER_SELECTOR);
    for url in &urls {
        selector.addItemWithTitle(&NSString::from_str(&format!("{url} — Checking…")));
    }
    selector.selectItemAtIndex(selected as isize);
    unsafe {
        selector.setTarget(Some(editor));
        selector.setAction(Some(sel!(selectComputeServer:)));
    }
    content.addSubview(&selector);
    *y -= ROW_HEIGHT;

    text_row(
        content,
        editor,
        "Server URL",
        &urls[selected],
        TAG_SERVER_URL,
        sel!(changeComputeServer:),
        y,
    );

    label(content, "Manage", *y, mtm);
    let add = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Add…"),
            Some(editor),
            Some(sel!(addComputeServer:)),
            mtm,
        )
    };
    add.setFrame(NSRect::new(
        NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
        NSSize::new(SERVER_BUTTON_WIDTH, CONTROL_HEIGHT),
    ));
    content.addSubview(&add);
    let remove = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Remove"),
            Some(editor),
            Some(sel!(removeComputeServer:)),
            mtm,
        )
    };
    remove.setFrame(NSRect::new(
        NSPoint::new(
            CONTROL_X + SERVER_BUTTON_WIDTH + SERVER_BUTTON_GAP,
            *y - CONTROL_Y_OFFSET,
        ),
        NSSize::new(SERVER_BUTTON_WIDTH, CONTROL_HEIGHT),
    ));
    remove.setTag(TAG_SERVER_REMOVE);
    remove.setEnabled(urls.len() > 1);
    content.addSubview(&remove);
    *y -= ROW_HEIGHT;

    detail_row(
        content,
        "Compatibility",
        TAG_SERVER_COMPATIBILITY,
        "Checking…",
        y,
        mtm,
    );
    detail_row(content, "Version", TAG_SERVER_VERSION, "—", y, mtm);
    detail_row(content, "Protocol", TAG_SERVER_PROTOCOL, "—", y, mtm);
    detail_row(content, "Features", TAG_SERVER_FEATURES, "—", y, mtm);

    section(content, "Compute", y, mtm);
    label(content, "Device", *y, mtm);
    let device = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
        ),
        false,
    );
    device.setTag(TAG_SERVER_DEVICE);
    device.addItemWithTitle(ns_string!("—"));
    device.setEnabled(false);
    unsafe {
        device.setTarget(Some(editor));
        device.setAction(Some(sel!(selectComputeDevice:)));
    }
    content.addSubview(&device);
    *y -= ROW_HEIGHT;
    detail_row(content, "Torch", TAG_SERVER_TORCH, "—", y, mtm);
    detail_row(content, "CUDA", TAG_SERVER_CUDA, "—", y, mtm);
    detail_row(content, "Jobs", TAG_SERVER_JOBS, "—", y, mtm);
    detail_row(
        content,
        "Reserved memory",
        TAG_SERVER_RESERVATIONS,
        "—",
        y,
        mtm,
    );
    detail_row(content, "Workers", TAG_SERVER_WORKERS, "—", y, mtm);
}

fn detail_row(
    content: &NSView,
    title: &str,
    tag: isize,
    value: &str,
    y: &mut f64,
    mtm: MainThreadMarker,
) {
    label(content, title, *y, mtm);
    let tall = matches!(tag, TAG_SERVER_FEATURES | TAG_SERVER_WORKERS);
    let height = if tall { FEATURES_HEIGHT } else { LABEL_HEIGHT };
    let value_label = NSTextField::labelWithString(&NSString::from_str(value), mtm);
    value_label.setFrame(NSRect::new(
        NSPoint::new(CONTROL_X, *y - (height - LABEL_HEIGHT)),
        NSSize::new(CONTROL_WIDTH, height),
    ));
    value_label.setTag(tag);
    value_label.setSelectable(true);
    content.addSubview(&value_label);
    *y -= if tall {
        ROW_HEIGHT + FEATURES_HEIGHT - LABEL_HEIGHT
    } else {
        ROW_HEIGHT
    };
}

fn pane(mtm: MainThreadMarker) -> objc2::rc::Retained<NSView> {
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(PANE_WIDTH, PANE_HEIGHT)),
    );
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    view
}

fn scrolling_pane(mtm: MainThreadMarker, height: f64) -> (Retained<NSView>, Retained<NSView>) {
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(PANE_WIDTH, PANE_HEIGHT)),
    );
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    scroll.setDrawsBackground(false);
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(PANE_WIDTH, height)),
    );
    scroll.setDocumentView(Some(&content));
    let clip = scroll.contentView();
    clip.scrollToPoint(NSPoint::new(0.0, height - PANE_HEIGHT));
    scroll.reflectScrolledClipView(&clip);
    (scroll.into_super(), content)
}

fn tab(
    title: &str,
    symbol: &str,
    view: objc2::rc::Retained<NSView>,
    mtm: MainThreadMarker,
) -> objc2::rc::Retained<NSTabViewItem> {
    let controller = NSViewController::new(mtm);
    let title = NSString::from_str(title);
    controller.setTitle(Some(&title));
    controller.setPreferredContentSize(NSSize::new(PANE_WIDTH, PANE_HEIGHT));
    controller.setView(&view);
    let item = NSTabViewItem::tabViewItemWithViewController(&controller);
    item.setLabel(&title);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&title),
    ) {
        item.setImage(Some(&image));
    }
    item
}

fn initial_y() -> f64 {
    PANE_HEIGHT - CONTENT_MARGIN - LABEL_HEIGHT
}

fn section(content: &NSView, title: &str, y: &mut f64, mtm: MainThreadMarker) {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));
    label.setFrame(NSRect::new(
        NSPoint::new(CONTENT_MARGIN, *y),
        NSSize::new(CONTROL_X - CONTENT_MARGIN, LABEL_HEIGHT),
    ));
    content.addSubview(&label);
    *y -= SECTION_HEIGHT;
}

fn label(content: &NSView, title: &str, y: f64, mtm: MainThreadMarker) {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(CONTENT_MARGIN, y),
        NSSize::new(CONTROL_X - CONTENT_MARGIN * 2.0, LABEL_HEIGHT),
    ));
    content.addSubview(&label);
}

fn numeric_row(content: &NSView, editor: &Editor, title: &str, tag: isize, y: &mut f64) {
    let mtm = content.mtm();
    let store = &editor
        .ivars()
        .session
        .get()
        .expect("project loaded")
        .preferences;
    let id = numeric_preference(tag);
    let range = preferences::integer_range(id).expect("numeric preference range");
    let PreferenceValue::Integer(value) = preferences::value(store, id) else {
        panic!("numeric preference must store an integer");
    };
    label(content, title, *y, mtm);
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH - STEPPER_WIDTH - STEPPER_GAP, CONTROL_HEIGHT),
        ),
    );
    field.setAlignment(NSTextAlignment::Right);
    field.setDoubleValue(value as f64 / range.scale as f64);
    field.setTag(tag);
    field
        .cell()
        .expect("numeric text cell")
        .setSendsActionOnEndEditing(true);
    let stepper = NSStepper::initWithFrame(
        NSStepper::alloc(mtm),
        NSRect::new(
            NSPoint::new(
                PANE_WIDTH - CONTENT_MARGIN - STEPPER_WIDTH,
                *y - CONTROL_Y_OFFSET,
            ),
            NSSize::new(STEPPER_WIDTH, CONTROL_HEIGHT),
        ),
    );
    stepper.setMinValue(range.minimum as f64);
    stepper.setMaxValue(range.maximum as f64);
    stepper.setIncrement(range.step as f64);
    stepper.setValueWraps(false);
    stepper.setIntegerValue(value as isize);
    stepper.setTag(tag + STEPPER_TAG_OFFSET);
    unsafe {
        field.setTarget(Some(editor));
        field.setAction(Some(sel!(changeNumericPreference:)));
        stepper.setTarget(Some(editor));
        stepper.setAction(Some(sel!(changeNumericPreference:)));
    }
    content.addSubview(&field);
    content.addSubview(&stepper);
    *y -= ROW_HEIGHT;
}

fn color_row(
    content: &NSView,
    editor: &Editor,
    color: shrimply_project_document::Color<u8>,
    y: &mut f64,
    mtm: MainThreadMarker,
) {
    label(content, "Caption background color", *y, mtm);
    let well = NSColorWell::initWithFrame(
        NSColorWell::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
        ),
    );
    well.setSupportsAlpha(true);
    well.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(color.r) / f64::from(u8::MAX),
        f64::from(color.g) / f64::from(u8::MAX),
        f64::from(color.b) / f64::from(u8::MAX),
        f64::from(color.a) / f64::from(u8::MAX),
    ));
    unsafe {
        well.setTarget(Some(editor));
        well.setAction(Some(sel!(changeCaptionColor:)));
    }
    content.addSubview(&well);
    *y -= ROW_HEIGHT;
}

fn font_row(content: &NSView, editor: &Editor, current: &str, y: &mut f64, mtm: MainThreadMarker) {
    label(content, "Default text font", *y, mtm);
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
        ),
        false,
    );
    for family in NSFontManager::sharedFontManager(mtm)
        .availableFontFamilies()
        .iter()
    {
        popup.addItemWithTitle(&family);
    }
    popup.selectItemWithTitle(&NSString::from_str(current));
    unsafe {
        popup.setTarget(Some(editor));
        popup.setAction(Some(sel!(changeDefaultFont:)));
    }
    content.addSubview(&popup);
    *y -= ROW_HEIGHT;
}

fn text_row(
    content: &NSView,
    editor: &Editor,
    title: &str,
    value: &str,
    tag: isize,
    action: objc2::runtime::Sel,
    y: &mut f64,
) {
    let mtm = content.mtm();
    label(content, title, *y, mtm);
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
            NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT),
        ),
    );
    field.setStringValue(&NSString::from_str(value));
    field.setTag(tag);
    unsafe {
        field.setTarget(Some(editor));
        field.setAction(Some(action));
    }
    content.addSubview(&field);
    *y -= ROW_HEIGHT;
}

fn blender_row(
    content: &NSView,
    editor: &Editor,
    path: Option<&std::path::Path>,
    y: &mut f64,
    mtm: MainThreadMarker,
) {
    label(content, "Blender binary", *y, mtm);
    let choose = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(
                path.and_then(std::path::Path::file_name)
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or("Choose…"),
            ),
            Some(editor),
            Some(sel!(chooseBlender:)),
            mtm,
        )
    };
    choose.setFrame(NSRect::new(
        NSPoint::new(CONTROL_X, *y - CONTROL_Y_OFFSET),
        NSSize::new(
            CONTROL_WIDTH - BLENDER_CLEAR_WIDTH - BLENDER_BUTTON_GAP,
            CONTROL_HEIGHT,
        ),
    ));
    choose.setTag(TAG_BLENDER_CHOOSE);
    content.addSubview(&choose);
    let clear = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Clear"),
            Some(editor),
            Some(sel!(clearBlender:)),
            mtm,
        )
    };
    clear.setFrame(NSRect::new(
        NSPoint::new(
            PANE_WIDTH - CONTENT_MARGIN - BLENDER_CLEAR_WIDTH,
            *y - CONTROL_Y_OFFSET,
        ),
        NSSize::new(BLENDER_CLEAR_WIDTH, CONTROL_HEIGHT),
    ));
    clear.setTag(TAG_BLENDER_CLEAR);
    clear.setEnabled(path.is_some());
    content.addSubview(&clear);
    *y -= ROW_HEIGHT;
}

fn sync_blender_row(content: &NSView, path: Option<&std::path::Path>, checking: bool) {
    let choose = content
        .viewWithTag(TAG_BLENDER_CHOOSE)
        .and_then(|view| view.downcast::<NSButton>().ok())
        .expect("Blender choose button installed");
    choose.setTitle(&NSString::from_str(if checking {
        "Checking…"
    } else {
        path.and_then(std::path::Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("Choose…")
    }));
    choose.setEnabled(!checking);
    let clear = content
        .viewWithTag(TAG_BLENDER_CLEAR)
        .and_then(|view| view.downcast::<NSButton>().ok())
        .expect("Blender clear button installed");
    clear.setEnabled(path.is_some() && !checking);
}

pub(super) fn sync_blender_window(
    window: &NSWindow,
    path: Option<&std::path::Path>,
    checking: bool,
    mtm: MainThreadMarker,
) {
    let content = integrations_content(window, mtm);
    sync_blender_row(&content, path, checking);
}

pub(super) fn sync_compute_servers_window(
    window: &NSWindow,
    store: &SharedPreferences,
    statuses: &BTreeMap<String, Result<preferences::ComputeServerPresentation, String>>,
    device_error: Option<&str>,
    device_pending: bool,
    mtm: MainThreadMarker,
) {
    let content = integrations_content(window, mtm);
    let (urls, selected) = preferences::compute_servers(store);
    let selected_url = &urls[selected];
    let selector = tagged::<NSPopUpButton>(&content, TAG_SERVER_SELECTOR, "server selector");
    selector.removeAllItems();
    for url in &urls {
        let summary = match statuses.get(url) {
            Some(Ok(status)) => status.summary.as_str(),
            Some(Err(_)) => "Unavailable",
            None => "Checking…",
        };
        selector.addItemWithTitle(&NSString::from_str(&format!("{url} — {summary}")));
    }
    selector.selectItemAtIndex(selected as isize);

    tagged::<NSTextField>(&content, TAG_SERVER_URL, "server URL")
        .setStringValue(&NSString::from_str(selected_url));
    tagged::<NSButton>(&content, TAG_SERVER_REMOVE, "server remove button")
        .setEnabled(urls.len() > 1);

    let compatibility =
        tagged::<NSTextField>(&content, TAG_SERVER_COMPATIBILITY, "server compatibility");
    let version = tagged::<NSTextField>(&content, TAG_SERVER_VERSION, "server version");
    let protocol = tagged::<NSTextField>(&content, TAG_SERVER_PROTOCOL, "server protocol");
    let features = tagged::<NSTextField>(&content, TAG_SERVER_FEATURES, "server features");
    let device = tagged::<NSPopUpButton>(&content, TAG_SERVER_DEVICE, "server device");
    let torch = tagged::<NSTextField>(&content, TAG_SERVER_TORCH, "server Torch");
    let cuda = tagged::<NSTextField>(&content, TAG_SERVER_CUDA, "server CUDA");
    let jobs = tagged::<NSTextField>(&content, TAG_SERVER_JOBS, "server jobs");
    let reservations =
        tagged::<NSTextField>(&content, TAG_SERVER_RESERVATIONS, "server reservations");
    let workers = tagged::<NSTextField>(&content, TAG_SERVER_WORKERS, "server workers");
    device.removeAllItems();
    device.setToolTip(device_error.map(NSString::from_str).as_deref());
    match statuses.get(selected_url) {
        Some(Ok(status)) => {
            compatibility.setStringValue(ns_string!("Compatible"));
            compatibility.setToolTip(None);
            version.setStringValue(&NSString::from_str(&status.version));
            version.setToolTip(
                status
                    .version_detail
                    .as_deref()
                    .map(NSString::from_str)
                    .as_deref(),
            );
            protocol.setStringValue(&NSString::from_str(&status.protocol));
            features.setStringValue(&NSString::from_str(&status.features));
            features.setToolTip(Some(&NSString::from_str(&status.features)));
            torch.setStringValue(&NSString::from_str(&status.torch));
            cuda.setStringValue(&NSString::from_str(&status.cuda));
            jobs.setStringValue(&NSString::from_str(&status.jobs));
            reservations.setStringValue(&NSString::from_str(&status.reservations));
            workers.setStringValue(&NSString::from_str(&status.workers));
            workers.setToolTip(Some(&NSString::from_str(&status.workers)));
            for item in &status.devices {
                device.addItemWithTitle(&NSString::from_str(&item.label));
            }
            if let Some(selected) = status.selected_device {
                device.selectItemAtIndex(selected as isize);
                device.setEnabled(!device_pending);
            } else {
                if status.devices.is_empty() {
                    device.addItemWithTitle(ns_string!("—"));
                }
                device.selectItemAtIndex(0);
                device.setEnabled(false);
            }
        }
        Some(Err(error)) => {
            compatibility.setStringValue(&NSString::from_str(&format!("Unavailable — {error}")));
            compatibility.setToolTip(Some(&NSString::from_str(error)));
            clear_server_details(
                [
                    &version,
                    &protocol,
                    &features,
                    &torch,
                    &cuda,
                    &jobs,
                    &reservations,
                    &workers,
                ],
                &device,
            );
        }
        None => {
            compatibility.setStringValue(ns_string!("Checking…"));
            compatibility.setToolTip(None);
            clear_server_details(
                [
                    &version,
                    &protocol,
                    &features,
                    &torch,
                    &cuda,
                    &jobs,
                    &reservations,
                    &workers,
                ],
                &device,
            );
        }
    }
}

fn clear_server_details(fields: [&NSTextField; 8], device: &NSPopUpButton) {
    for field in fields {
        field.setStringValue(ns_string!("—"));
        field.setToolTip(None);
    }
    device.removeAllItems();
    device.addItemWithTitle(ns_string!("—"));
    device.setEnabled(false);
}

fn tagged<T: objc2::DowncastTarget>(content: &NSView, tag: isize, name: &str) -> Retained<T> {
    content
        .viewWithTag(tag)
        .and_then(|view| view.downcast::<T>().ok())
        .unwrap_or_else(|| panic!("{name} installed"))
}

fn integrations_content(window: &NSWindow, mtm: MainThreadMarker) -> Retained<NSView> {
    let tabs = window
        .contentViewController()
        .and_then(|controller| controller.downcast::<NSTabViewController>().ok())
        .expect("Settings tab controller installed");
    let root = tabs
        .tabViewItems()
        .iter()
        .find(|item| item.label().to_string() == "Integrations")
        .and_then(|item| item.view(mtm))
        .expect("Integrations settings pane installed");
    root.clone()
        .downcast::<NSScrollView>()
        .ok()
        .and_then(|scroll| scroll.documentView())
        .unwrap_or(root)
}

pub(super) fn prompt_compute_server_url(mtm: MainThreadMarker) -> Option<String> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(ns_string!("Add Compute Server"));
    alert.setInformativeText(ns_string!("Enter the URL of a Shrimply compute server."));
    alert.addButtonWithTitle(ns_string!("Add"));
    alert.addButtonWithTitle(ns_string!("Cancel"));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(CONTROL_WIDTH, CONTROL_HEIGHT)),
    );
    field.setPlaceholderString(Some(ns_string!("http://127.0.0.1:8787")));
    alert.setAccessoryView(Some(&field));
    (alert.runModal() == NSAlertFirstButtonReturn).then(|| field.stringValue().to_string())
}

pub(super) fn set_caption_color(store: &SharedPreferences, well: &NSColorWell) {
    let mut red = 0.0;
    let mut green = 0.0;
    let mut blue = 0.0;
    let mut alpha = 0.0;
    unsafe {
        well.color()
            .getRed_green_blue_alpha(&mut red, &mut green, &mut blue, &mut alpha)
    };
    let component = |value: f64| (value.clamp(0.0, 1.0) * f64::from(u8::MAX)).round() as u8;
    preferences::set_value(
        store,
        PreferenceId::CaptionBackgroundColor,
        PreferenceValue::Color(shrimply_project_document::Color::new(
            component(red),
            component(green),
            component(blue),
            component(alpha),
        )),
    )
    .expect("caption background color has a fixed color type");
}

pub(super) fn choose_blender_path(
    mtm: MainThreadMarker,
) -> Result<Option<std::path::PathBuf>, String> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setTitle(Some(ns_string!("Choose Blender Binary")));
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    let path = panel
        .URL()
        .ok_or("Blender selection has no URL")?
        .to_file_path()
        .ok_or("Blender must be a local file")?;
    Ok(Some(path))
}
