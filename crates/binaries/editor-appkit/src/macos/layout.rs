use super::{Editor, canvas, timeline};
use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSFont, NSGlassEffectView, NSGlassEffectViewStyle, NSImage,
    NSLayoutAttribute, NSLayoutConstraint, NSLayoutConstraintOrientation,
    NSLayoutPriorityDefaultLow, NSProgressIndicator, NSProgressIndicatorStyle, NSSlider,
    NSSplitViewController, NSSplitViewDividerStyle, NSSplitViewItem, NSStackView,
    NSStackViewDistribution, NSTextField, NSUserInterfaceLayoutOrientation, NSView,
    NSViewController,
};
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};

const PREVIEW_LOADING_INDICATOR_SIZE: f64 = 16.0;
const PLAYBACK_SLIDER_PADDING: f64 = 6.0;
const SOLID_SELECTION_CORNER_RADIUS: f64 = 7.0;

pub const WINDOW_SIZE: NSSize = NSSize::new(1280.0, 800.0);
pub const MINIMUM_WINDOW_SIZE: NSSize = NSSize::new(960.0, 640.0);
const TOP_MIN_HEIGHT: f64 = 260.0;
const TIMELINE_MIN_HEIGHT: f64 = 260.0;
const TIMELINE_FRACTION: f64 = 0.4;
pub const GAP: f64 = 6.0;
pub const TOOLBAR_WIDTH: f64 = 44.0;
pub const BUTTON_SIZE: f64 = 28.0;
const PLAYBAR_HEIGHT: f64 = 44.0;

pub struct Layout {
    pub root: Retained<NSSplitViewController>,
    pub inspector_controller: shrimply_inspector_appkit::Inspector,
    pub canvases: Vec<Retained<canvas::CanvasView>>,
    pub progress: Retained<NSSlider>,
    pub time: Retained<NSTextField>,
    pub speed: Retained<NSTextField>,
    pub play: Retained<NSButton>,
    pub inspector: super::inspector_split::InspectorSplit,
    pub timeline: Retained<NSSplitViewItem>,
    pub preview_layout: Retained<NSStackView>,
    pub viewer: Retained<NSStackView>,
    pub preview_host: Retained<NSView>,
    pub preview_tools: Retained<NSStackView>,
    pub playbar: Retained<NSStackView>,
    pub fullscreen_button: Retained<NSButton>,
    pub controls_overlay: Retained<NSGlassEffectView>,
    pub overlay_constraints: Vec<Retained<NSLayoutConstraint>>,
    pub fullscreen_constraints: Vec<Retained<NSLayoutConstraint>>,
}

pub fn symbol(name: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(label)),
    )
    .unwrap_or_else(|| panic!("macOS must provide the {name} system symbol"))
}

pub fn button(icon: &str, label: &str, mtm: MainThreadMarker) -> Retained<NSButton> {
    let button =
        unsafe { NSButton::buttonWithImage_target_action(&symbol(icon, label), None, None, mtm) };
    button.setBezelStyle(NSBezelStyle::AccessoryBarAction);
    button.setToolTip(Some(&NSString::from_str(label)));
    button.setEnabled(false);
    button
        .widthAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    button
        .heightAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    button
}

pub enum ToggleStyle {
    Solid,
    Grouped,
}

pub fn set_toggle_selected(button: &NSButton, selected: bool, style: ToggleStyle) {
    let state = if selected {
        objc2_app_kit::NSControlStateValueOn
    } else {
        objc2_app_kit::NSControlStateValueOff
    };
    let accent = NSColor::controlAccentColor();
    let (background, foreground, radius) = match style {
        ToggleStyle::Solid => (
            accent.clone(),
            NSColor::whiteColor(),
            SOLID_SELECTION_CORNER_RADIUS,
        ),
        ToggleStyle::Grouped => (NSColor::clearColor(), accent, 0.0),
    };
    let foreground = if selected {
        foreground
    } else {
        NSColor::labelColor()
    };
    // Push-on/push-off buttons change their native state before the action runs;
    // compare the applied tint too so a click still updates the decoration.
    if button.state() == state && button.contentTintColor().as_deref() == Some(&*foreground) {
        return;
    }
    // Keep native content metrics identical in both states; only the decoration changes.
    button.setBordered(false);
    button.setWantsLayer(true);
    let layer = button.layer().expect("layer-backed toggle button");
    layer.setCornerRadius(radius);
    let background = if selected {
        background.CGColor()
    } else {
        NSColor::clearColor().CGColor()
    };
    layer.setBackgroundColor(Some(&background));
    button.setContentTintColor(Some(&foreground));
    button.setState(state);
}

fn circular_glass_button(button: &NSButton, mtm: MainThreadMarker) -> Retained<NSGlassEffectView> {
    button.setBordered(false);
    let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setCornerRadius(BUTTON_SIZE / 2.0);
    glass.setContentView(Some(button));
    glass
        .widthAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    glass
        .heightAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    glass
}

pub fn stack(vertical: bool, mtm: MainThreadMarker) -> Retained<NSStackView> {
    let view = NSStackView::initWithFrame(NSStackView::alloc(mtm), NSRect::ZERO);
    view.setOrientation(if vertical {
        NSUserInterfaceLayoutOrientation::Vertical
    } else {
        NSUserInterfaceLayoutOrientation::Horizontal
    });
    view.setAlignment(if vertical {
        NSLayoutAttribute::Width
    } else {
        NSLayoutAttribute::Height
    });
    view.setSpacing(0.0);
    view
}

pub fn split_item(view: &NSView, mtm: MainThreadMarker) -> Retained<NSSplitViewItem> {
    let controller = NSViewController::new(mtm);
    controller.setView(view);
    NSSplitViewItem::splitViewItemWithViewController(&controller)
}

pub fn build(editor: &Editor) -> Layout {
    let mtm = editor.mtm();
    let session = editor.ivars().session.get().expect("project loaded");
    let inspector_controller = shrimply_inspector_appkit::Inspector::new(
        session.project.clone(),
        session.player_state.clone(),
        session.selection_state.clone(),
        session.property_clipboard.clone(),
        session.preview_focus.clone(),
        session.preferences.clone(),
        mtm,
    );

    // GTK: preview tools on the left, playback strip directly below the viewer.
    let preview_tools = stack(true, mtm);
    preview_tools.setSpacing(GAP);
    preview_tools.setAlignment(NSLayoutAttribute::CenterX);
    preview_tools
        .widthAnchor()
        .constraintEqualToConstant(TOOLBAR_WIDTH)
        .setActive(true);
    preview_tools.setEdgeInsets(NSEdgeInsets {
        top: GAP,
        left: GAP,
        bottom: GAP,
        right: GAP,
    });
    let loading_status = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    loading_status
        .widthAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    loading_status
        .heightAnchor()
        .constraintEqualToConstant(BUTTON_SIZE)
        .setActive(true);
    let loading_done = button("checkmark", "Preview ready", mtm);
    loading_done.setBordered(false);
    loading_done.setTranslatesAutoresizingMaskIntoConstraints(false);
    loading_status.addSubview(&loading_done);
    let loading_spinner =
        NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm), NSRect::ZERO);
    loading_spinner.setStyle(NSProgressIndicatorStyle::Spinning);
    loading_spinner.setIndeterminate(true);
    loading_spinner.setDisplayedWhenStopped(false);
    loading_spinner.setHidden(true);
    loading_spinner.setTranslatesAutoresizingMaskIntoConstraints(false);
    loading_status.addSubview(&loading_spinner);
    for constraint in [
        loading_done
            .centerXAnchor()
            .constraintEqualToAnchor(&loading_status.centerXAnchor()),
        loading_done
            .centerYAnchor()
            .constraintEqualToAnchor(&loading_status.centerYAnchor()),
        loading_spinner
            .centerXAnchor()
            .constraintEqualToAnchor(&loading_status.centerXAnchor()),
        loading_spinner
            .centerYAnchor()
            .constraintEqualToAnchor(&loading_status.centerYAnchor()),
        loading_spinner
            .widthAnchor()
            .constraintEqualToConstant(PREVIEW_LOADING_INDICATOR_SIZE),
        loading_spinner
            .heightAnchor()
            .constraintEqualToConstant(PREVIEW_LOADING_INDICATOR_SIZE),
    ] {
        constraint.setActive(true);
    }
    preview_tools.addArrangedSubview(&loading_status);
    let [frame_rate, speed] = ["--", "x1"].map(|text| {
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        label
            .heightAnchor()
            .constraintEqualToConstant(BUTTON_SIZE)
            .setActive(true);
        preview_tools.addArrangedSubview(&label);
        label
    });
    frame_rate.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        NSFont::systemFontSize(),
        unsafe { objc2_app_kit::NSFontWeightRegular },
    )));
    frame_rate.setToolTip(Some(&NSString::from_str("Frame rate")));
    speed.setToolTip(Some(&NSString::from_str("Playback speed")));
    let guides = button("ruler", "Guides", mtm);
    guides.setBordered(false);
    guides.setEnabled(true);
    guides.setButtonType(objc2_app_kit::NSButtonType::PushOnPushOff);
    preview_tools.addArrangedSubview(&guides);
    preview_tools.addArrangedSubview(&NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO));
    let viewer = stack(false, mtm);
    viewer.addArrangedSubview(&preview_tools);
    let preview_canvas = canvas::new(
        canvas::Content::Preview(Box::new(canvas::preview::State::new(
            guides.clone(),
            loading_done,
            loading_spinner,
            frame_rate,
            session.playback_performance.clone(),
        ))),
        session.clone(),
        editor.ivars().imports.clone(),
        mtm,
    );
    viewer.addArrangedSubview(&preview_canvas);
    unsafe {
        guides.setTarget(Some(&*preview_canvas));
        guides.setAction(Some(sel!(togglePreviewGuides:)));
    }
    for (tool, icon, label) in [
        (canvas::PaintTool::Pen, "pencil.tip", "Pen (B)"),
        (canvas::PaintTool::Fill, "paintbrush", "Fill (F)"),
        (canvas::PaintTool::Eraser, "eraser", "Eraser (E)"),
        (
            canvas::PaintTool::Adjust,
            "slider.horizontal.3",
            "Adjust points",
        ),
        (
            canvas::PaintTool::Transform,
            "arrow.up.left.and.arrow.down.right",
            "Stroke Transform (T)",
        ),
        (canvas::PaintTool::Smaller, "minus", "Smaller paint tool"),
        (canvas::PaintTool::Larger, "plus", "Larger paint tool"),
        (
            canvas::PaintTool::Palette,
            "paintpalette",
            "Next paint color",
        ),
        (
            canvas::PaintTool::OnionPrevious,
            "chevron.left",
            "Previous drawing onion skin",
        ),
        (
            canvas::PaintTool::OnionNext,
            "chevron.right",
            "Next drawing onion skin",
        ),
    ] {
        let button = button(icon, label, mtm);
        button.setEnabled(true);
        button.setButtonType(objc2_app_kit::NSButtonType::PushOnPushOff);
        button.setTag(tool as isize);
        unsafe {
            button.setTarget(Some(&*preview_canvas));
            button.setAction(Some(sel!(changePaintTool:)));
        }
        preview_tools.addArrangedSubview(&button);
        preview_canvas.register_paint_tool(tool, button);
    }

    let playbar = stack(false, mtm);
    playbar.setAlignment(NSLayoutAttribute::CenterY);
    playbar.setDistribution(NSStackViewDistribution::Fill);
    playbar.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    playbar.setSpacing(GAP);
    playbar.setEdgeInsets(NSEdgeInsets {
        top: GAP,
        left: GAP,
        bottom: GAP,
        right: GAP,
    });
    playbar
        .heightAnchor()
        .constraintEqualToConstant(PLAYBAR_HEIGHT)
        .setActive(true);
    let mut play_button = None;
    for (icon, label, action) in [
        ("backward.fill", "Step backward", sel!(stepBackward:)),
        ("play.fill", "Play", sel!(togglePlayback:)),
        ("forward.fill", "Step forward", sel!(stepForward:)),
    ] {
        let control = button(icon, label, mtm);
        control.setEnabled(true);
        unsafe {
            control.setTarget(Some(editor));
            control.setAction(Some(action));
        }
        if label == "Play" {
            play_button = Some(control.clone());
        } else {
            let interval = shrimply_preview_provider_skia::playback::STEP_REPEAT_TICK.as_secs_f32();
            control.setContinuous(true);
            control.setPeriodicDelay_interval(interval, interval);
            control.sendActionOn(
                objc2_app_kit::NSEventMask::LeftMouseDown | objc2_app_kit::NSEventMask::Periodic,
            );
        }
        let glass = circular_glass_button(&control, mtm);
        playbar.addArrangedSubview(&glass);
        if label == "Step forward" {
            playbar.setCustomSpacing_afterView(PLAYBACK_SLIDER_PADDING, &glass);
        }
    }
    let progress = unsafe { NSSlider::sliderWithTarget_action(None, None, mtm) };
    progress.setEnabled(true);
    progress.setContinuous(true);
    progress.sendActionOn(
        objc2_app_kit::NSEventMask::LeftMouseDown
            | objc2_app_kit::NSEventMask::LeftMouseDragged
            | objc2_app_kit::NSEventMask::LeftMouseUp,
    );
    progress.setMinValue(0.0);
    progress.setMaxValue(1.0);
    progress.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    unsafe {
        progress.setTarget(Some(editor));
        progress.setAction(Some(sel!(seek:)));
    }
    playbar.addArrangedSubview(&progress);
    playbar.setCustomSpacing_afterView(PLAYBACK_SLIDER_PADDING, &progress);
    let time = NSTextField::labelWithString(&NSString::from_str("—:—:— / —:—:—"), mtm);
    time.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        NSFont::systemFontSize(),
        unsafe { objc2_app_kit::NSFontWeightRegular },
    )));
    time.setTextColor(Some(&NSColor::secondaryLabelColor()));
    playbar.addArrangedSubview(&time);
    let fullscreen = button(
        "arrow.up.left.and.arrow.down.right",
        "Fullscreen Preview",
        mtm,
    );
    fullscreen.setEnabled(true);
    unsafe {
        fullscreen.setTarget(Some(editor));
        fullscreen.setAction(Some(sel!(togglePreviewFullscreen:)));
    }
    playbar.addArrangedSubview(&circular_glass_button(&fullscreen, mtm));
    let preview_host = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    preview_host.setTranslatesAutoresizingMaskIntoConstraints(false);
    preview_host.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    preview_host.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Vertical,
    );
    viewer.setDistribution(NSStackViewDistribution::Fill);
    preview_canvas.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    preview_canvas.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Vertical,
    );
    viewer.setTranslatesAutoresizingMaskIntoConstraints(false);
    preview_host.addSubview(&viewer);
    for constraint in [
        viewer
            .leadingAnchor()
            .constraintEqualToAnchor(&preview_host.leadingAnchor()),
        viewer
            .trailingAnchor()
            .constraintEqualToAnchor(&preview_host.trailingAnchor()),
        viewer
            .topAnchor()
            .constraintEqualToAnchor(&preview_host.topAnchor()),
        viewer
            .bottomAnchor()
            .constraintEqualToAnchor(&preview_host.bottomAnchor()),
    ] {
        constraint.setActive(true);
    }
    let controls_overlay =
        NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
    controls_overlay.setStyle(NSGlassEffectViewStyle::Regular);
    controls_overlay.setCornerRadius(PLAYBAR_HEIGHT / 2.0);
    controls_overlay.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    controls_overlay.setTranslatesAutoresizingMaskIntoConstraints(false);
    let overlay_constraints = vec![
        playbar
            .leadingAnchor()
            .constraintEqualToAnchor(&controls_overlay.leadingAnchor()),
        playbar
            .trailingAnchor()
            .constraintEqualToAnchor(&controls_overlay.trailingAnchor()),
        playbar
            .topAnchor()
            .constraintEqualToAnchor(&controls_overlay.topAnchor()),
        playbar
            .bottomAnchor()
            .constraintEqualToAnchor(&controls_overlay.bottomAnchor()),
        controls_overlay
            .leadingAnchor()
            .constraintEqualToAnchor(&preview_host.leadingAnchor()),
        controls_overlay
            .trailingAnchor()
            .constraintEqualToAnchor(&preview_host.trailingAnchor()),
        controls_overlay
            .bottomAnchor()
            .constraintEqualToAnchor(&preview_host.bottomAnchor()),
        controls_overlay
            .heightAnchor()
            .constraintEqualToConstant(PLAYBAR_HEIGHT),
    ];
    let preview_layout = stack(true, mtm);
    preview_layout.setDistribution(NSStackViewDistribution::Fill);
    preview_layout.addArrangedSubview(&preview_host);
    preview_layout.addArrangedSubview(&playbar);
    let inspector = super::inspector_split::InspectorSplit::new(
        inspector_controller.view(),
        &preview_layout,
        WINDOW_SIZE,
        mtm,
    );
    let top = split_item(inspector.view(), mtm);
    top.setMinimumThickness(TOP_MIN_HEIGHT);
    let (timeline_view, timeline_canvas, meter_canvas) =
        timeline::build(session.clone(), editor.ivars().imports.clone(), mtm);
    let timeline = split_item(&timeline_view, mtm);
    timeline.setMinimumThickness(TIMELINE_MIN_HEIGHT);
    timeline.setPreferredThicknessFraction(TIMELINE_FRACTION);
    timeline.setCanCollapse(true);
    let root = NSSplitViewController::new(mtm);
    root.splitView().setVertical(false);
    root.splitView()
        .setDividerStyle(NSSplitViewDividerStyle::Thin);
    root.addSplitViewItem(&top);
    root.addSplitViewItem(&timeline);
    root.view()
        .setFrame(NSRect::new(NSPoint::ZERO, WINDOW_SIZE));
    let fullscreen_constraints = vec![
        preview_host
            .leadingAnchor()
            .constraintEqualToAnchor(&root.view().leadingAnchor()),
        preview_host
            .trailingAnchor()
            .constraintEqualToAnchor(&root.view().trailingAnchor()),
        preview_host
            .topAnchor()
            .constraintEqualToAnchor(&root.view().topAnchor()),
        preview_host
            .bottomAnchor()
            .constraintEqualToAnchor(&root.view().bottomAnchor()),
    ];
    Layout {
        root,
        inspector_controller,
        canvases: vec![preview_canvas, timeline_canvas, meter_canvas],
        progress,
        time,
        speed,
        play: play_button.expect("play button created"),
        inspector,
        timeline,
        preview_layout,
        viewer,
        preview_host,
        preview_tools,
        playbar,
        fullscreen_button: fullscreen,
        controls_overlay,
        overlay_constraints,
        fullscreen_constraints,
    }
}
