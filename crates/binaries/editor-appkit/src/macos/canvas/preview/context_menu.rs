use super::*;
use objc2::sel;
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem};
use objc2_foundation::{NSNumber, NSString, ns_string};
use shrimply_preview_render_core::playback::toolbar::{self, ContextRequest};

impl CanvasView {
    pub(in crate::macos::canvas) fn open_preview_context_menu(&self, event: &NSEvent) {
        let enabled = matches!(&*self.ivars().content.borrow(), Content::Preview(state) if state.renderer.presented_frame().is_some());
        self.ivars().menu_choice.set(None);
        let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm()), ns_string!("Preview"));
        menu.setAutoenablesItems(false);
        // The existing menu callback only records a tag; image work starts after tracking ends.
        let preferences = &self.ivars().session.preferences;
        let actions = toolbar::context_menu(preferences, true);
        for (index, entry) in actions.iter().enumerate() {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm()),
                    &NSString::from_str(&entry.label),
                    Some(sel!(chooseCanvasContext:)),
                    ns_string!(""),
                )
            };
            item.setTag(index.try_into().expect("preview menu index fits NSInteger"));
            item.setEnabled(entry.checked.is_some() || enabled);
            if let Some(checked) = entry.checked {
                unsafe {
                    item.setRepresentedObject(Some(&NSNumber::new_bool(checked)));
                }
                item.setState(if checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
            unsafe {
                item.setTarget(Some(self));
            }
            menu.addItem(&item);
        }
        crate::macos::menu_sheet::present(self, &menu, self.point(event), false);
        let request = self
            .ivars()
            .menu_choice
            .take()
            .and_then(|index| actions[index].action.activate(preferences));
        let result = match request {
            Some(ContextRequest::CopyImage) => self.capture_preview_image(false),
            Some(ContextRequest::SaveImage) => self.capture_preview_image(true),
            None => Ok(()),
        };
        if let Err(error) = result {
            self.show_error(&error);
        }
    }
}
