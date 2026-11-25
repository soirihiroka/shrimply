mod grid;
mod preview;

use crate::ActionButton;
use block2::RcBlock;
use objc2::{MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSFont,
    NSIndexPathNSCollectionViewAdditions, NSScrollView, NSSearchField, NSTextField, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSIndexPath, NSPoint, NSRect, NSSet, NSSize, NSString, NSTimer,
};
use skia_safe::Typeface;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const WINDOW_SIZE: NSSize = NSSize::new(1000.0, 720.0);
const MINIMUM_SIZE: NSSize = NSSize::new(660.0, 480.0);
const HEADER_HEIGHT: f64 = 100.0;
const FOOTER_HEIGHT: f64 = 52.0;
const INSET: f64 = 24.0;
const POLL_INTERVAL: Duration = Duration::from_millis(33);

/// A stable ID must include the font source and revision, not just its display name.
#[derive(Clone, Debug)]
pub struct FontPickerItem {
    pub id: String,
    pub name: String,
    pub google: bool,
}

type SearchCallback = Box<dyn Fn(&FontPicker, String, bool)>;
type ChooseCallback = Box<dyn Fn(&FontPicker, &FontPickerItem)>;
type PollCallback = Box<dyn Fn(&FontPicker)>;

pub struct FontPickerBuilder {
    load: preview::Loader,
    items: Vec<FontPickerItem>,
    selected: Option<String>,
    search: Option<SearchCallback>,
    choose: Option<ChooseCallback>,
    poll: Option<PollCallback>,
}

#[derive(Clone)]
pub struct FontPicker(Rc<State>);

struct State {
    window: Retained<NSWindow>,
    grid: Retained<grid::Collection>,
    _delegate: Retained<grid::Delegate>,
    search: Retained<NSSearchField>,
    count: Retained<NSTextField>,
    status: Retained<NSTextField>,
    items: RefCell<Vec<FontPickerItem>>,
    selected: RefCell<Option<String>>,
    previews: RefCell<preview::Previews>,
    appearance: Cell<Option<(u32, bool)>>,
    active: Cell<bool>,
    busy: Cell<bool>,
    on_search: Option<SearchCallback>,
    on_choose: ChooseCallback,
    on_poll: Option<PollCallback>,
}

impl FontPicker {
    pub fn builder(
        load: impl Fn(&FontPickerItem) -> Result<Typeface, String> + Send + Sync + 'static,
    ) -> FontPickerBuilder {
        FontPickerBuilder {
            load: Arc::new(load),
            items: Vec::new(),
            selected: None,
            search: None,
            choose: None,
            poll: None,
        }
    }

    pub fn set_items(&self, items: Vec<FontPickerItem>) {
        self.0.count.setStringValue(&NSString::from_str(&format!(
            "{} font families",
            items.len()
        )));
        self.0.items.replace(items);
        self.0.grid.reloadData();
        let selected = self.0.selected.borrow().clone();
        self.set_selected(selected);
        self.0.grid.scrollPoint(NSPoint::ZERO);
        grid::refresh(self);
    }

    pub fn set_status(&self, status: &str) {
        self.0.status.setStringValue(&NSString::from_str(status));
    }

    pub fn set_selected(&self, id: Option<String>) {
        let paths = self
            .0
            .items
            .borrow()
            .iter()
            .position(|item| Some(&item.id) == id.as_ref())
            .map(|index| NSIndexPath::indexPathForItem_inSection(index as isize, 0))
            .into_iter()
            .collect::<Vec<_>>();
        self.0
            .grid
            .setSelectionIndexPaths(&NSSet::from_retained_slice(&paths));
        self.0.selected.replace(id);
    }

    /// Prevent duplicate choices while the caller activates a remote family.
    pub fn set_busy(&self, busy: bool) {
        self.0.busy.set(busy);
        self.0.search.setEnabled(!busy);
    }

    pub fn close(&self) {
        if !self.0.active.replace(false) {
            return;
        }
        self.0.previews.borrow_mut().close();
        if let Some(parent) = self.0.window.sheetParent() {
            parent.endSheet(&self.0.window);
        }
        self.0.window.orderOut(None);
    }

    fn choose(&self, index: usize) {
        if !self.0.active.get() || self.0.busy.get() {
            return;
        }
        let item = self.0.items.borrow().get(index).cloned();
        if let Some(item) = item {
            (self.0.on_choose)(self, &item);
        }
    }
}

impl FontPickerBuilder {
    pub fn items(mut self, items: Vec<FontPickerItem>) -> Self {
        self.items = items;
        self
    }
    pub fn selected(mut self, selected: Option<String>) -> Self {
        self.selected = selected;
        self
    }
    /// The boolean is true for an explicit Return, false for an ordinary edit.
    pub fn on_search(mut self, callback: impl Fn(&FontPicker, String, bool) + 'static) -> Self {
        self.search = Some(Box::new(callback));
        self
    }
    pub fn on_choose(mut self, callback: impl Fn(&FontPicker, &FontPickerItem) + 'static) -> Self {
        self.choose = Some(Box::new(callback));
        self
    }
    /// Called only while the sheet is open, to drain asynchronous model updates.
    pub fn on_poll(mut self, callback: impl Fn(&FontPicker) + 'static) -> Self {
        self.poll = Some(Box::new(callback));
        self
    }

    pub fn show(self, parent: &NSWindow, mtm: MainThreadMarker) -> FontPicker {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::ZERO, WINDOW_SIZE),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setTitle(&NSString::from_str("Fonts"));
        window.setContentMinSize(MINIMUM_SIZE);
        let root =
            NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, WINDOW_SIZE));
        let title = NSTextField::labelWithString(&NSString::from_str("All Fonts"), mtm);
        title.setFont(Some(&NSFont::boldSystemFontOfSize(20.0)));
        title.setFrame(NSRect::new(
            NSPoint::new(INSET, WINDOW_SIZE.height - 38.0),
            NSSize::new(220.0, 26.0),
        ));
        let count = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        count.setTextColor(Some(&NSColor::secondaryLabelColor()));
        count.setFrame(NSRect::new(
            NSPoint::new(INSET, WINDOW_SIZE.height - 62.0),
            NSSize::new(260.0, 22.0),
        ));
        let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        status.setFrame(NSRect::new(
            NSPoint::new(INSET, WINDOW_SIZE.height - 90.0),
            NSSize::new(WINDOW_SIZE.width - INSET * 2.0, 22.0),
        ));
        for label in [&title, &count, &status] {
            label.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewWidthSizable,
            );
            root.addSubview(label);
        }
        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(
                NSPoint::new(350.0, WINDOW_SIZE.height - 58.0),
                NSSize::new(WINDOW_SIZE.width - 350.0 - INSET, 32.0),
            ),
        );
        search.setPlaceholderString(Some(&NSString::from_str(
            "Search fonts or paste a Google Fonts URL",
        )));
        search.setSendsWholeSearchString(true);
        search.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinYMargin | NSAutoresizingMaskOptions::ViewWidthSizable,
        );
        root.addSubview(&search);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, FOOTER_HEIGHT),
                NSSize::new(
                    WINDOW_SIZE.width,
                    WINDOW_SIZE.height - HEADER_HEIGHT - FOOTER_HEIGHT,
                ),
            ),
        );
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        root.addSubview(&scroll);
        let picker = FontPicker(Rc::new_cyclic(|weak| {
            let (grid, delegate) = grid::new(weak.clone(), &scroll, &search, mtm);
            State {
                window: window.clone(),
                grid,
                _delegate: delegate,
                search: search.clone(),
                count,
                status,
                items: RefCell::new(Vec::new()),
                selected: RefCell::new(self.selected),
                previews: RefCell::new(preview::Previews::new(self.load)),
                appearance: Cell::new(None),
                active: Cell::new(true),
                busy: Cell::new(false),
                on_search: self.search,
                on_choose: self
                    .choose
                    .expect("font picker requires a selection callback"),
                on_poll: self.poll,
            }
        }));
        let weak = Rc::downgrade(&picker.0);
        let cancel = ActionButton::new(
            "Cancel",
            move || {
                if let Some(state) = weak.upgrade() {
                    FontPicker(state).close();
                }
            },
            mtm,
        );
        cancel
            .view()
            .setKeyEquivalent(&NSString::from_str("\u{1b}"));
        cancel.view().setFrame(NSRect::new(
            NSPoint::new(WINDOW_SIZE.width - INSET - 90.0, 10.0),
            NSSize::new(90.0, 32.0),
        ));
        cancel
            .view()
            .setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        root.addSubview(cancel.view());
        window.setContentView(Some(&root));
        parent.beginSheet_completionHandler(&window, None);
        picker.set_items(self.items);
        window.makeFirstResponder(Some(&search));
        let timer_picker = picker.clone();
        let timer = RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            if !timer_picker.0.active.get() || timer_picker.0.window.sheetParent().is_none() {
                timer_picker.close();
                unsafe { timer.as_ref() }.invalidate();
                return;
            }
            if let Some(poll) = &timer_picker.0.on_poll {
                poll(&timer_picker);
            }
            if timer_picker.0.active.get() {
                grid::refresh(&timer_picker);
            }
        });
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                POLL_INTERVAL.as_secs_f64(),
                true,
                &timer,
            );
        }
        picker
    }
}
