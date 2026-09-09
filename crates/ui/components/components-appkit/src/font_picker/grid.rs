use super::{
    FontPicker, State,
    preview::{Key, SPECIMEN_EDGE},
};
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::ProtocolObject,
    sel,
};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSCollectionView, NSCollectionViewDataSource,
    NSCollectionViewElement, NSCollectionViewFlowLayout, NSCollectionViewItem, NSColor,
    NSControlTextEditingDelegate, NSEvent, NSFont, NSImage, NSImageScaling, NSImageView,
    NSIndexPathNSCollectionViewAdditions, NSScrollView, NSSearchField, NSSearchFieldDelegate,
    NSTextAlignment, NSTextField, NSTextFieldDelegate, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSData, NSEdgeInsets, NSIndexPath, NSInteger,
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSet, NSSize, NSString,
};
use std::rc::Weak;

const ITEM_SIZE: NSSize = NSSize::new(200.0, 240.0);
const CARD_Y: f64 = 54.0;
const STATUS_TAG: NSInteger = 1;

pub(super) struct Ivars {
    state: Weak<State>,
}

define_class!(
    #[unsafe(super(NSCollectionViewItem))]
    #[thread_kind = MainThreadOnly]
    struct SpecimenItem;
    unsafe impl NSObjectProtocol for SpecimenItem {}
    unsafe impl NSUserInterfaceItemIdentification for SpecimenItem {}
    unsafe impl NSCollectionViewElement for SpecimenItem {
        #[unsafe(method(prepareForReuse))]
        fn prepare_for_reuse(&self) {
            unsafe { let _: () = msg_send![super(self), prepareForReuse]; }
            self.imageView().expect("font image").setImage(None);
            self.textField().expect("font label").setStringValue(&NSString::from_str(""));
            self.view().setToolTip(None);
        }
    }
    impl SpecimenItem {
        #[unsafe(method(setSelected:))]
        fn set_selected(&self, selected: bool) {
            unsafe { let _: () = msg_send![super(self), setSelected: selected]; }
            if self.isViewLoaded() {
                let card = self.view().subviews().objectAtIndex(0).downcast::<NSBox>().expect("font card box");
                let color = if selected { NSColor::controlAccentColor() } else { NSColor::separatorColor() };
                card.setBorderColor(&color);
                card.setBorderWidth(if selected { 3.0 } else { 1.0 });
            }
        }
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(());
            unsafe { msg_send![super(this), initWithNibName: None::<&NSString>, bundle: None::<&NSBundle>] }
        }
        #[unsafe(method(loadView))]
        fn load_view(&self) {
            build_card(self, self.mtm());
        }
    }
);

define_class!(
    #[unsafe(super(NSCollectionView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub(super) struct Collection;
    unsafe impl NSObjectProtocol for Collection {}
    impl Collection {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            let hit = self.indexPathForItemAtPoint(point);
            unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            if let Some(path) = hit && let Some(state) = self.ivars().state.upgrade() {
                FontPicker(state).choose(path.item() as usize);
            }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let Some(state) = self.ivars().state.upgrade() else { return; };
            let picker = FontPicker(state);
            match event.charactersIgnoringModifiers().map(|s| s.to_string()).as_deref() {
                Some("\r" | "\u{3}") => {
                    if let Some(path) = self.selectionIndexPaths().anyObject() { picker.choose(path.item() as usize); }
                }
                Some("\u{1b}") => picker.close(),
                _ => unsafe { let _: () = msg_send![super(self), keyDown: event]; },
            }
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub(super) struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSTextFieldDelegate for Delegate {}
    unsafe impl NSSearchFieldDelegate for Delegate {}
    unsafe impl NSControlTextEditingDelegate for Delegate {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, _notification: &NSNotification) { self.search(false); }
    }
    unsafe impl NSCollectionViewDataSource for Delegate {
        #[unsafe(method(collectionView:numberOfItemsInSection:))]
        fn count(&self, _collection: &NSCollectionView, _section: NSInteger) -> NSInteger {
            self.ivars().state.upgrade().map_or(0, |s| s.items.borrow().len() as NSInteger)
        }
        #[unsafe(method_id(collectionView:itemForRepresentedObjectAtIndexPath:))]
        fn item(&self, collection: &NSCollectionView, path: &NSIndexPath) -> Retained<NSCollectionViewItem> {
            let item = collection.makeItemWithIdentifier_forIndexPath(&NSString::from_str("FontSpecimen"), path);
            // Loading our registered item builds its outlets before AppKit lays it out.
            let view = item.view();
            let state = self.ivars().state.upgrade().expect("font picker is alive while providing items");
            let entries = state.items.borrow();
            let entry = &entries[path.item() as usize];
            item.textField().expect("font label").setStringValue(&NSString::from_str(&entry.name));
            item.imageView().expect("font image").setImage(None);
            view.setToolTip(Some(&NSString::from_str(&entry.name)));
            let status = item.view().viewWithTag(STATUS_TAG).expect("font status").downcast::<NSTextField>().expect("font status label");
            let (scale, dark) = state.appearance.get().unwrap_or_else(|| display_style(&state));
            let key = Key { id: entry.id.clone(), scale, dark };
            match state.previews.borrow().cached(&key) {
                Some(Ok(bytes)) => {
                    let data = NSData::with_bytes(&bytes);
                    let preview = NSImage::initWithData(NSImage::alloc(), &data).expect("valid encoded font preview");
                    preview.setSize(NSSize::new(SPECIMEN_EDGE, SPECIMEN_EDGE));
                    item.imageView().expect("font image").setImage(Some(&preview));
                    status.setStringValue(&NSString::from_str(entry.source.as_deref().unwrap_or("")));
                }
                Some(Err(error)) => {
                    status.setStringValue(&NSString::from_str("Preview unavailable"));
                    view.setToolTip(Some(&NSString::from_str(&error)));
                }
                None => status.setStringValue(&NSString::from_str("Loading preview…")),
            }
            item
        }
    }
    impl Delegate {
        #[unsafe(method(search:))]
        fn submitted(&self, _sender: &NSSearchField) { self.search(true); }
    }
);

impl Delegate {
    fn search(&self, immediate: bool) {
        if let Some(state) = self.ivars().state.upgrade() {
            let picker = FontPicker(state);
            if let Some(search) = &picker.0.on_search {
                search(
                    &picker,
                    picker.0.search.stringValue().to_string(),
                    immediate,
                );
            }
        }
    }
}

pub(super) fn new(
    state: Weak<State>,
    scroll: &NSScrollView,
    search: &NSSearchField,
    mtm: MainThreadMarker,
) -> (Retained<Collection>, Retained<Delegate>) {
    let grid = Collection::alloc(mtm).set_ivars(Ivars {
        state: state.clone(),
    });
    let grid: Retained<Collection> =
        unsafe { msg_send![super(grid), initWithFrame: scroll.bounds()] };
    grid.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    grid.setSelectable(true);
    grid.setAllowsEmptySelection(true);
    grid.setAllowsMultipleSelection(false);
    grid.setBackgroundColors(Some(&NSArray::from_slice(&[
        &*NSColor::controlBackgroundColor(),
    ])));
    let layout = NSCollectionViewFlowLayout::new(mtm);
    layout.setItemSize(ITEM_SIZE);
    layout.setMinimumInteritemSpacing(20.0);
    layout.setMinimumLineSpacing(24.0);
    layout.setSectionInset(NSEdgeInsets {
        top: 24.0,
        left: 24.0,
        bottom: 24.0,
        right: 24.0,
    });
    grid.setCollectionViewLayout(Some(&layout));
    unsafe {
        grid.registerClass_forItemWithIdentifier(
            Some(SpecimenItem::class()),
            &NSString::from_str("FontSpecimen"),
        );
    }
    let delegate = Delegate::alloc(mtm).set_ivars(Ivars { state });
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    grid.setDataSource(Some(ProtocolObject::from_ref(&*delegate)));
    unsafe {
        search.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        search.setTarget(Some(&delegate));
        search.setAction(Some(sel!(search:)));
    }
    scroll.setDocumentView(Some(&grid));
    (grid, delegate)
}

fn build_card(item: &NSCollectionViewItem, mtm: MainThreadMarker) {
    let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, ITEM_SIZE));
    item.setView(&root);
    let card_frame = NSRect::new(
        NSPoint::new(10.0, CARD_Y),
        NSSize::new(SPECIMEN_EDGE, SPECIMEN_EDGE),
    );
    let card = NSBox::initWithFrame(NSBox::alloc(mtm), card_frame);
    card.setBoxType(NSBoxType::Custom);
    card.setCornerRadius(20.0);
    card.setBorderWidth(1.0);
    card.setFillColor(&NSColor::controlBackgroundColor());
    card.setBorderColor(&NSColor::separatorColor());
    root.addSubview(&card);
    let image = NSImageView::initWithFrame(NSImageView::alloc(mtm), card_frame);
    image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    root.addSubview(&image);
    item.setImageView(Some(&image));
    let name = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
    name.setAlignment(NSTextAlignment::Center);
    name.setFont(Some(&NSFont::systemFontOfSize(15.0)));
    name.setFrame(NSRect::new(
        NSPoint::ZERO,
        NSSize::new(ITEM_SIZE.width, 46.0),
    ));
    root.addSubview(&name);
    item.setTextField(Some(&name));
    let status = NSTextField::labelWithString(&NSString::from_str("Loading preview…"), mtm);
    status.setAlignment(NSTextAlignment::Center);
    status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    status.setFrame(NSRect::new(
        NSPoint::new(14.0, CARD_Y + 8.0),
        NSSize::new(172.0, 18.0),
    ));
    status.setTag(STATUS_TAG);
    root.addSubview(&status);
}

fn display_style(state: &State) -> (u32, bool) {
    let scale = state.window.backingScaleFactor().ceil().max(1.0) as u32;
    let appearance = state.grid.effectiveAppearance();
    let dark = unsafe {
        appearance
            .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[
                NSAppearanceNameAqua,
                NSAppearanceNameDarkAqua,
            ]))
            .is_some_and(|name| *name == *NSAppearanceNameDarkAqua)
    };
    (scale, dark)
}

pub(super) fn refresh(picker: &FontPicker) {
    let state = &picker.0;
    let (scale, dark) = display_style(state);
    let appearance_changed = state.appearance.replace(Some((scale, dark))) != Some((scale, dark));
    let entries = state.items.borrow();
    let visible = state
        .grid
        .indexPathsForVisibleItems()
        .iter()
        .filter_map(|path| {
            let entry = entries.get(path.item() as usize)?.clone();
            let key = Key {
                id: entry.id.clone(),
                scale,
                dark,
            };
            Some((path, entry, key))
        })
        .collect::<Vec<_>>();
    drop(entries);
    let mut previews = state.previews.borrow_mut();
    let changed = previews.visible(visible.iter().map(|(_, _, key)| key.clone()).collect());
    let mut reload = Vec::new();
    for (path, entry, key) in visible {
        previews.request(&key, &entry);
        if appearance_changed || changed.contains(&key) {
            reload.push(path);
        }
    }
    drop(previews);
    if !reload.is_empty() {
        state
            .grid
            .reloadItemsAtIndexPaths(&NSSet::from_retained_slice(&reload));
    }
}
