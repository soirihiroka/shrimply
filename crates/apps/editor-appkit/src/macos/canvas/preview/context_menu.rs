use super::*;
use objc2::sel;
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::{NSPoint, NSString, ns_string};

impl CanvasView {
    pub(in crate::macos::canvas) fn open_preview_context_menu(&self, event: &NSEvent) {
        let enabled = matches!(&*self.ivars().content.borrow(), Content::Preview(state) if state.renderer.image().is_some());
        self.ivars().menu_choice.set(None);
        let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm()), ns_string!("Preview"));
        menu.setAutoenablesItems(false);
        // The existing menu callback only records a tag; image work starts after tracking ends.
        let actions = [("Copy Preview Image", false), ("Save Preview Image…", true)];
        for (index, (label, _)) in actions.iter().enumerate() {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm()),
                    &NSString::from_str(label),
                    Some(sel!(chooseCanvasContext:)),
                    ns_string!(""),
                )
            };
            item.setTag(index.try_into().expect("preview menu index fits NSInteger"));
            item.setEnabled(enabled);
            unsafe {
                item.setTarget(Some(self));
            }
            menu.addItem(&item);
        }
        let point = self.point(event);
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            NSPoint::new(point.x.into(), point.y.into()),
            Some(self),
        );
        if let Some(index) = self.ivars().menu_choice.take()
            && let Err(error) = self.capture_preview_image(actions[index].1)
        {
            self.show_error(&error);
        }
    }
}
