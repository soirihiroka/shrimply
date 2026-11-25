use objc2::rc::{Retained, Weak};
use objc2_app_kit::{NSTextField, NSView};
use objc2_foundation::{MainThreadMarker, NSString};
use shrimply_components_appkit::{
    ActionButton, FontPicker, FontPickerItem, column_append_intrinsic,
};
use skia_safe::{FontMgr, FontStyle};
use std::{cell::RefCell, rc::Rc, sync::mpsc};

pub(super) fn page(log: Rc<dyn Fn(String)>, mtm: MainThreadMarker) -> Retained<NSView> {
    let root = super::page_stack(mtm);
    let label = NSTextField::labelWithString(
        &NSString::from_str("Choose an installed font to preview the native specimen grid."),
        mtm,
    );
    column_append_intrinsic(&root, &label);
    let weak = Weak::new(&*root);
    let button = ActionButton::new(
        "Choose font…",
        move || {
            let Some(window) = weak.load().and_then(|root| root.window()) else {
                return;
            };
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let mut names = FontMgr::new().family_names().collect::<Vec<_>>();
                names.sort_by_key(|name| name.to_lowercase());
                let _ = sender.send(
                    names
                        .into_iter()
                        .map(|name| FontPickerItem {
                            id: format!("Local:0:{}", name.to_lowercase()),
                            name,
                            google: false,
                        })
                        .collect::<Vec<_>>(),
                );
            });
            let items = Rc::new(RefCell::new(Vec::<FontPickerItem>::new()));
            let query = Rc::new(RefCell::new(String::new()));
            FontPicker::builder(|item| {
                thread_local! { static MANAGER: FontMgr = FontMgr::new(); }
                MANAGER
                    .with(|manager| manager.match_family_style(&item.name, FontStyle::default()))
                    .ok_or_else(|| format!("Font {} is unavailable", item.name))
            })
            .on_search({
                let items = items.clone();
                let query = query.clone();
                move |picker, value, _| {
                    query.replace(value.to_lowercase());
                    picker.set_items(
                        items
                            .borrow()
                            .iter()
                            .filter(|item| {
                                item.name.to_lowercase().contains(query.borrow().as_str())
                            })
                            .cloned()
                            .collect(),
                    );
                }
            })
            .on_choose({
                let log = log.clone();
                let label = label.clone();
                move |picker, item| {
                    log(format!("font selected {}", item.name));
                    label.setStringValue(&NSString::from_str(&item.name));
                    picker.close();
                }
            })
            .on_poll(move |picker| {
                if let Ok(loaded) = receiver.try_recv() {
                    items.replace(loaded);
                    picker.set_items(
                        items
                            .borrow()
                            .iter()
                            .filter(|item| {
                                item.name.to_lowercase().contains(query.borrow().as_str())
                            })
                            .cloned()
                            .collect(),
                    );
                    picker.set_status("");
                }
            })
            .show(&window, mtm)
            .set_status("Loading fonts…");
        },
        mtm,
    );
    column_append_intrinsic(&root, button.view());
    root.into_super()
}
