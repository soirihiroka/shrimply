use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSColor, NSFont, NSGlassEffectView,
    NSGlassEffectViewStyle, NSImage, NSMenu, NSMenuItem, NSPopUpArrowPosition, NSPopUpButton,
    NSPopUpButtonCell, NSScrollElasticity, NSScrollView, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, ns_string};
use shrimply_cross_ui_core::launcher;
use shrimply_support::recent_projects::RecentProject;

const ROW_HEIGHT: f64 = 64.0;
const ROW_SPACING: f64 = 8.0;

pub struct List {
    scroll: objc2::rc::Retained<NSScrollView>,
    document: objc2::rc::Retained<NSView>,
}

impl List {
    pub fn new(frame: NSRect, mtm: MainThreadMarker) -> Self {
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(false);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        let document = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), scroll.contentSize()),
        );
        document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        scroll.setDocumentView(Some(&document));
        Self { scroll, document }
    }

    pub fn view(&self) -> &NSScrollView {
        &self.scroll
    }

    pub fn refresh(
        &self,
        target: &AnyObject,
        query: &str,
        mtm: MainThreadMarker,
    ) -> Result<Vec<RecentProject>, String> {
        for view in self.document.subviews().iter() {
            view.removeFromSuperview();
        }

        let projects = launcher::load_recent_projects(query)?;
        let viewport = self.scroll.contentSize();
        let needed_height = projects.len() as f64 * (ROW_HEIGHT + ROW_SPACING);
        let scrollable = needed_height > viewport.height;
        let height = viewport.height.max(needed_height);
        self.scroll.setHasVerticalScroller(scrollable);
        self.scroll.setVerticalScrollElasticity(if scrollable {
            NSScrollElasticity::Automatic
        } else {
            NSScrollElasticity::None
        });
        self.document
            .setFrameSize(NSSize::new(viewport.width, height));
        self.scroll
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, height - viewport.height));

        if projects.is_empty() {
            let title = if query.trim().is_empty() {
                "No Recent Projects"
            } else {
                "No Matching Projects"
            };
            let empty = NSTextField::labelWithString(&NSString::from_str(title), mtm);
            empty.setAlignment(NSTextAlignment::Center);
            empty.setFont(Some(&NSFont::systemFontOfSize(17.0)));
            empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
            empty.setFrame(NSRect::new(
                NSPoint::new(0.0, (height - 24.0) / 2.0),
                NSSize::new(viewport.width, 24.0),
            ));
            empty.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewMaxYMargin,
            );
            self.document.addSubview(&empty);
            return Ok(projects);
        }

        for (index, project) in projects.iter().enumerate() {
            let row_y = height - ROW_HEIGHT - index as f64 * (ROW_HEIGHT + ROW_SPACING);
            let card = NSGlassEffectView::initWithFrame(
                NSGlassEffectView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, row_y),
                    NSSize::new(viewport.width, ROW_HEIGHT),
                ),
            );
            card.setStyle(NSGlassEffectViewStyle::Clear);
            card.setCornerRadius(12.0);
            card.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewMinYMargin,
            );
            let row = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(viewport.width, ROW_HEIGHT),
                ),
            );
            row.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

            let name = NSString::from_str(&project.name);
            let open = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &name,
                    Some(target),
                    Some(sel!(openRecent:)),
                    mtm,
                )
            };
            open.setTag(index as isize);
            open.setAlignment(NSTextAlignment::Left);
            open.setFont(Some(&NSFont::systemFontOfSize(15.0)));
            open.setBordered(false);
            open.setFrame(NSRect::new(
                NSPoint::new(8.0, 30.0),
                NSSize::new(viewport.width - 52.0, 26.0),
            ));
            open.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            open.setToolTip(Some(&NSString::from_str(&project.path.to_string_lossy())));
            row.addSubview(&open);

            let edited = launcher::last_edited(&project.path)
                .map(|date| format!("Last edited {date}"))
                .unwrap_or_else(|| "Last edited time unavailable".to_string());
            let subtitle = NSTextField::labelWithString(&NSString::from_str(&edited), mtm);
            subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
            subtitle.setFrame(NSRect::new(
                NSPoint::new(16.0, 10.0),
                NSSize::new(viewport.width - 60.0, 20.0),
            ));
            subtitle.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            row.addSubview(&subtitle);

            let options = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(mtm),
                NSRect::new(
                    NSPoint::new(viewport.width - 38.0, 18.0),
                    NSSize::new(30.0, 28.0),
                ),
                true,
            );
            options.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            options.setBordered(false);
            options.setToolTip(Some(ns_string!("Project options")));
            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
            let display = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!(""),
                    None,
                    ns_string!(""),
                )
            };
            let ellipsis = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("ellipsis"),
                Some(ns_string!("Project options")),
            )
            .expect("macOS must provide the project-options system symbol");
            display.setImage(Some(&ellipsis));
            menu.addItem(&display);
            for (title, action) in [
                ("Info", sel!(recentInfo:)),
                ("Show in Finder", sel!(showRecentInFinder:)),
                ("Delete", sel!(removeRecent:)),
            ] {
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(title),
                        Some(action),
                        ns_string!(""),
                    )
                };
                item.setTag(index as isize);
                unsafe { item.setTarget(Some(target)) };
                menu.addItem(&item);
            }
            options.setMenu(Some(&menu));
            options.setImage(Some(&ellipsis));
            options.setBezelStyle(NSBezelStyle::AccessoryBarAction);
            options
                .cell()
                .and_then(|cell| cell.downcast::<NSPopUpButtonCell>().ok())
                .expect("project-options control must use an NSPopUpButtonCell")
                .setArrowPosition(NSPopUpArrowPosition::NoArrow);
            row.addSubview(&options);
            card.setContentView(Some(&row));
            self.document.addSubview(&card);
        }
        Ok(projects)
    }
}
