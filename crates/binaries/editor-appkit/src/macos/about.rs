use objc2::MainThreadOnly;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{
    NSAlert, NSApplication, NSBackingStoreType, NSColor, NSFont, NSFontWeightRegular,
    NSGridCellPlacement, NSGridView, NSImageScaling, NSImageView, NSLayoutAttribute,
    NSTextAlignment, NSTextField, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectView, NSWindow, NSWindowStyleMask, NSWindowTitleVisibility, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSBundle, NSPoint, NSRect, NSSize, NSString, NSURL};
use shrimply_components_appkit::{ActionButton, column_stack, row_stack};
use shrimply_components_core::about;
use shrimply_i18n::text;
use std::cell::OnceCell;

const WIDTH: f64 = 348.0;
const TOP_INSET: f64 = 40.0;
const BOTTOM_INSET: f64 = 28.0;
const ICON_SIZE: f64 = 112.0;

pub fn show(mtm: MainThreadMarker) {
    thread_local! {
        static WINDOW: OnceCell<Retained<NSWindow>> = const { OnceCell::new() };
    }
    WINDOW.with(|window| {
        window.get_or_init(|| build(mtm)).makeKeyAndOrderFront(None);
    });
}

fn build(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, 360.0));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(&text("About Shrimply")));
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setMovableByWindowBackground(true);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));

    let background = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
    background.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    background.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    window.setContentView(Some(&background));

    let body = column_stack(6.0, mtm);
    body.setAlignment(NSLayoutAttribute::CenterX);
    body.setTranslatesAutoresizingMaskIntoConstraints(false);
    background.addSubview(&body);
    body.centerXAnchor()
        .constraintEqualToAnchor(&background.centerXAnchor())
        .setActive(true);
    body.topAnchor()
        .constraintEqualToAnchor_constant(&background.topAnchor(), TOP_INSET)
        .setActive(true);

    let icon = NSImageView::new(mtm);
    icon.setImage(
        NSApplication::sharedApplication(mtm)
            .applicationIconImage()
            .as_deref(),
    );
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    body.addArrangedSubview(&icon);
    icon.widthAnchor()
        .constraintEqualToConstant(ICON_SIZE)
        .setActive(true);
    icon.heightAnchor()
        .constraintEqualToConstant(ICON_SIZE)
        .setActive(true);
    body.setCustomSpacing_afterView(12.0, &icon);

    let name = NSTextField::labelWithString(&NSString::from_str(about::NAME), mtm);
    name.setFont(Some(&NSFont::boldSystemFontOfSize(22.0)));
    body.addArrangedSubview(&name);

    let description =
        NSTextField::wrappingLabelWithString(&NSString::from_str(&text(about::DESCRIPTION)), mtm);
    description.setTextColor(Some(&NSColor::secondaryLabelColor()));
    description.setAlignment(NSTextAlignment::Center);
    description.setPreferredMaxLayoutWidth(WIDTH - 64.0);
    body.addArrangedSubview(&description);
    description
        .widthAnchor()
        .constraintEqualToConstant(WIDTH - 64.0)
        .setActive(true);
    body.setCustomSpacing_afterView(16.0, &description);

    let metadata = version_details(mtm);
    let metadata_width = metadata.fittingSize().width;
    body.addArrangedSubview(&metadata);
    metadata
        .widthAnchor()
        .constraintEqualToConstant(metadata_width)
        .setActive(true);
    body.setCustomSpacing_afterView(20.0, &metadata);

    let links = row_stack(8.0, mtm);
    add_link(&links, "GitHub", about::WEBSITE.1, mtm);
    add_link(&links, about::DOCUMENTATION.0, about::DOCUMENTATION.1, mtm);
    let parent = Weak::new(&*window);
    let credits = ActionButton::new(
        &text("Credits…"),
        move || {
            if let Some(window) = parent.load() {
                show_credits(&window, mtm);
            }
        },
        mtm,
    );
    links.addArrangedSubview(credits.view());
    body.addArrangedSubview(&links);

    window.setContentSize(NSSize::new(
        WIDTH,
        body.fittingSize().height + TOP_INSET + BOTTOM_INSET,
    ));
    window.center();
    window
}

fn bundle_version() -> (String, Option<String>) {
    let bundle = NSBundle::mainBundle();
    let string = |key: &str| {
        bundle
            .objectForInfoDictionaryKey(&NSString::from_str(key))?
            .downcast::<NSString>()
            .ok()
            .map(|value| value.to_string())
    };
    let version =
        string("CFBundleShortVersionString").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
    (version, string("CFBundleVersion"))
}

fn version_details(mtm: MainThreadMarker) -> Retained<NSGridView> {
    let (version, build) = bundle_version();
    let grid =
        NSGridView::gridViewWithNumberOfColumns_rows(2, 1 + isize::from(build.is_some()), mtm);
    grid.setColumnSpacing(10.0);
    grid.setRowSpacing(4.0);
    grid.columnAtIndex(0)
        .setXPlacement(NSGridCellPlacement::Trailing);
    grid.columnAtIndex(1)
        .setXPlacement(NSGridCellPlacement::Leading);
    let font = NSFont::monospacedSystemFontOfSize_weight(12.0, unsafe { NSFontWeightRegular });
    for (row, (label, value)) in std::iter::once(("Version", version))
        .chain(build.map(|build| ("Build", build)))
        .enumerate()
    {
        let label = NSTextField::labelWithString(&NSString::from_str(&text(label)), mtm);
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let value = NSTextField::labelWithString(&NSString::from_str(&value), mtm);
        value.setFont(Some(&font));
        value.setSelectable(true);
        grid.cellAtColumnIndex_rowIndex(0, row as isize)
            .setContentView(Some(&label));
        grid.cellAtColumnIndex_rowIndex(1, row as isize)
            .setContentView(Some(&value));
    }
    grid
}

fn add_link(links: &objc2_app_kit::NSStackView, label: &str, url: &str, mtm: MainThreadMarker) {
    let url = NSURL::URLWithString(&NSString::from_str(url)).expect("About link must be valid");
    let button = ActionButton::new(
        &text(label),
        move || {
            NSWorkspace::sharedWorkspace().openURL(&url);
        },
        mtm,
    );
    links.addArrangedSubview(button.view());
}

fn show_credits(window: &NSWindow, mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&text("Credits")));
    let credits = about::CREDIT_SECTIONS
        .iter()
        .map(|(heading, entries)| format!("{}\n{}", text(heading), entries.join(" · ")))
        .collect::<Vec<_>>()
        .join("\n\n");
    alert.setInformativeText(&NSString::from_str(&credits));
    let links = row_stack(8.0, mtm);
    for &(label, url) in about::LINKS {
        if label != about::WEBSITE.0 && label != about::DOCUMENTATION.0 {
            add_link(&links, label, url, mtm);
        }
    }
    add_link(
        &links,
        "License",
        "https://www.gnu.org/licenses/gpl-3.0.html",
        mtm,
    );
    links.setFrameSize(links.fittingSize());
    alert.setAccessoryView(Some(&links));
    alert.addButtonWithTitle(&NSString::from_str(&text("Done")));
    alert.beginSheetModalForWindow_completionHandler(window, None);
}
