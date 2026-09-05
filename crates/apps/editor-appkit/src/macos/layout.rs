use super::{Editor, timeline};
use objc2::rc::Retained;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{
    NSBezelStyle, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSGlassEffectView,
    NSGlassEffectViewStyle, NSImage, NSImageScaling, NSImageView, NSLayoutAttribute, NSSlider,
    NSSplitViewController, NSSplitViewDividerStyle, NSSplitViewItem, NSStackView, NSTextField,
    NSTitlePosition, NSUserInterfaceLayoutOrientation, NSView, NSViewController,
};
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};

pub const WINDOW_SIZE: NSSize = NSSize::new(1280.0, 800.0);
pub const MINIMUM_WINDOW_SIZE: NSSize = NSSize::new(960.0, 640.0);
const INSPECTOR_MIN_WIDTH: f64 = 280.0;
const PREVIEW_MIN_WIDTH: f64 = 480.0;
const TOP_MIN_HEIGHT: f64 = 260.0;
const TIMELINE_MIN_HEIGHT: f64 = 260.0;
const INSPECTOR_FRACTION: f64 = 0.3;
const TIMELINE_FRACTION: f64 = 0.4;
pub const PADDING: f64 = 12.0;
pub const GAP: f64 = 6.0;
pub const TOOLBAR_WIDTH: f64 = 44.0;
pub const BUTTON_SIZE: f64 = 28.0;
const SYMBOL_SIZE: f64 = 32.0;
const HEADING_FONT_SIZE: f64 = 13.0;
const PLAYBAR_HEIGHT: f64 = 44.0;

pub struct Layout {
    pub root: Retained<NSSplitViewController>,
    pub inspector: Retained<NSSplitViewItem>,
    pub timeline: Retained<NSSplitViewItem>,
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

pub fn surface(mtm: MainThreadMarker) -> Retained<NSBox> {
    let panel = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
    panel.setBoxType(NSBoxType::Custom);
    panel.setTitlePosition(NSTitlePosition::NoTitle);
    panel.setBorderWidth(0.0);
    panel.setContentViewMargins(NSSize::ZERO);
    panel.setFillColor(&NSColor::controlBackgroundColor());
    panel
}

pub fn placeholder(title: &str, icon: &str, mtm: MainThreadMarker) -> Retained<NSBox> {
    let panel = surface(mtm);
    let content = panel.contentView().expect("panel must have a content view");
    let image = NSImageView::imageViewWithImage(&symbol(icon, title), mtm);
    image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    image.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));
    image.setTranslatesAutoresizingMaskIntoConstraints(false);
    content.addSubview(&image);
    image
        .widthAnchor()
        .constraintEqualToConstant(SYMBOL_SIZE)
        .setActive(true);
    image
        .heightAnchor()
        .constraintEqualToConstant(SYMBOL_SIZE)
        .setActive(true);
    image
        .centerXAnchor()
        .constraintEqualToAnchor(&content.centerXAnchor())
        .setActive(true);
    image
        .centerYAnchor()
        .constraintEqualToAnchor_constant(&content.centerYAnchor(), -PADDING)
        .setActive(true);
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(HEADING_FONT_SIZE)));
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    label.setTranslatesAutoresizingMaskIntoConstraints(false);
    content.addSubview(&label);
    label
        .centerXAnchor()
        .constraintEqualToAnchor(&content.centerXAnchor())
        .setActive(true);
    label
        .topAnchor()
        .constraintEqualToAnchor_constant(&image.bottomAnchor(), GAP)
        .setActive(true);
    panel
}

pub fn split_item(view: &NSView, mtm: MainThreadMarker) -> Retained<NSSplitViewItem> {
    let controller = NSViewController::new(mtm);
    controller.setView(view);
    NSSplitViewItem::splitViewItemWithViewController(&controller)
}

pub fn build(editor: &Editor) -> Layout {
    let mtm = editor.mtm();
    let inspector_content = placeholder("Inspector", "slider.horizontal.3", mtm);
    let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setContentView(inspector_content.contentView().as_deref());
    let inspector = split_item(&glass, mtm);
    inspector.setMinimumThickness(INSPECTOR_MIN_WIDTH);
    inspector.setPreferredThicknessFraction(INSPECTOR_FRACTION);
    inspector.setCanCollapse(true);

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
    preview_tools.addArrangedSubview(&button("checkmark", "Loading status", mtm));
    for text in ["—", "x1"] {
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        label
            .heightAnchor()
            .constraintEqualToConstant(BUTTON_SIZE)
            .setActive(true);
        preview_tools.addArrangedSubview(&label);
    }
    preview_tools.addArrangedSubview(&button("ruler", "Guides", mtm));
    preview_tools.addArrangedSubview(&NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO));
    let viewer = stack(false, mtm);
    viewer.addArrangedSubview(&preview_tools);
    viewer.addArrangedSubview(&placeholder("Preview", "play.rectangle", mtm));

    let playbar = stack(false, mtm);
    playbar.setAlignment(NSLayoutAttribute::CenterY);
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
    for (icon, label) in [
        ("backward.fill", "Step backward"),
        ("play.fill", "Play"),
        ("forward.fill", "Step forward"),
    ] {
        playbar.addArrangedSubview(&button(icon, label, mtm));
    }
    let progress = unsafe { NSSlider::sliderWithTarget_action(None, None, mtm) };
    progress.setEnabled(false);
    playbar.addArrangedSubview(&progress);
    let time = NSTextField::labelWithString(&NSString::from_str("—:—:— / —:—:—"), mtm);
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
    playbar.addArrangedSubview(&fullscreen);
    let preview = stack(true, mtm);
    preview.addArrangedSubview(&viewer);
    preview.addArrangedSubview(&playbar);
    let preview = split_item(&preview, mtm);
    preview.setMinimumThickness(PREVIEW_MIN_WIDTH);

    let top = NSSplitViewController::new(mtm);
    top.splitView().setVertical(true);
    top.splitView()
        .setDividerStyle(NSSplitViewDividerStyle::Thin);
    top.addSplitViewItem(&inspector);
    top.addSplitViewItem(&preview);
    let top = NSSplitViewItem::splitViewItemWithViewController(&top);
    top.setMinimumThickness(TOP_MIN_HEIGHT);
    let timeline = NSSplitViewItem::splitViewItemWithViewController(&timeline::build(mtm));
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
    Layout {
        root,
        inspector,
        timeline,
    }
}
