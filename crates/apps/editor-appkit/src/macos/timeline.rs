use super::layout::{BUTTON_SIZE, TOOLBAR_WIDTH, button, placeholder, split_item, stack, surface};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBox, NSBoxType, NSColor, NSSplitViewController, NSSplitViewDividerStyle, NSView,
};
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSRect};

// Match the GTK timeline's tool rail, track labels, ruler, and audio-meter widths.
const TRACK_HEADER_WIDTH: f64 = 158.0;
const RULER_HEIGHT: f64 = 44.0;
const AUDIO_METER_WIDTH: f64 = shrimply_skia_adw_core::audio_meter::DEFAULT_WIDTH as f64;
const DIVIDER_WIDTH: f64 = 1.0;
const TOOL_GAP: f64 = 4.0;
const TIMELINE_MIN_WIDTH: f64 = 480.0;

pub fn build(mtm: MainThreadMarker) -> Retained<NSSplitViewController> {
    let tools = stack(true, mtm);
    tools.setSpacing(TOOL_GAP);
    tools.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterX);
    tools.setEdgeInsets(NSEdgeInsets {
        top: TOOL_GAP,
        left: TOOL_GAP,
        bottom: TOOL_GAP,
        right: TOOL_GAP,
    });
    tools
        .widthAnchor()
        .constraintEqualToConstant(TOOLBAR_WIDTH)
        .setActive(true);
    for group in [
        &[("paperclip", "Magnet"), ("metronome", "Beat Grid")][..],
        &[("cursorarrow", "Pointer"), ("scissors", "Cut")][..],
        &[
            ("rectangle.on.rectangle", "Overwrite/Insert"),
            ("pause.rectangle", "Block"),
            ("rectangle.badge.plus", "New Track"),
        ][..],
    ] {
        if tools.arrangedSubviews().count() != 0 {
            let divider = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
            divider.setBoxType(NSBoxType::Separator);
            divider
                .widthAnchor()
                .constraintEqualToConstant(BUTTON_SIZE)
                .setActive(true);
            divider
                .heightAnchor()
                .constraintEqualToConstant(DIVIDER_WIDTH)
                .setActive(true);
            tools.addArrangedSubview(&divider);
        }
        for (icon, label) in group {
            tools.addArrangedSubview(&button(icon, label, mtm));
        }
    }
    tools.addArrangedSubview(&NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO));

    let headers = surface(mtm);
    headers.setFillColor(&NSColor::windowBackgroundColor());
    headers
        .widthAnchor()
        .constraintEqualToConstant(TRACK_HEADER_WIDTH)
        .setActive(true);
    let ruler = surface(mtm);
    ruler.setFillColor(&NSColor::windowBackgroundColor());
    ruler
        .heightAnchor()
        .constraintEqualToConstant(RULER_HEIGHT)
        .setActive(true);
    let tracks = stack(true, mtm);
    tracks.addArrangedSubview(&ruler);
    tracks.addArrangedSubview(&placeholder("Timeline", "rectangle.stack", mtm));
    let timeline = stack(false, mtm);
    timeline.setSpacing(DIVIDER_WIDTH);
    timeline.addArrangedSubview(&tools);
    timeline.addArrangedSubview(&headers);
    timeline.addArrangedSubview(&tracks);

    let meter = super::audio_meter::new(mtm);
    let meter = split_item(&meter, mtm);
    meter.setMinimumThickness(AUDIO_METER_WIDTH);
    meter.setMaximumThickness(AUDIO_METER_WIDTH + BUTTON_SIZE);
    let timeline = split_item(&timeline, mtm);
    timeline.setMinimumThickness(TIMELINE_MIN_WIDTH);
    let split = NSSplitViewController::new(mtm);
    split.splitView().setVertical(true);
    split
        .splitView()
        .setDividerStyle(NSSplitViewDividerStyle::Thin);
    split.addSplitViewItem(&timeline);
    split.addSplitViewItem(&meter);
    split
}
