use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSView};
use objc2_foundation::MainThreadMarker;
use shrimply_editor_state::preview_focus::{self, SharedPreviewFocus};
use shrimply_inspector_core::{InspectorController, InspectorTarget, item::ControlPreviewFocus};
use std::{cell::RefCell, ptr::NonNull, rc::Rc};

struct Entry {
    view: Weak<NSView>,
    target: InspectorTarget,
    focus: ControlPreviewFocus,
}

pub(super) struct FocusMap {
    entries: RefCell<Vec<Entry>>,
    controller: InspectorController,
    state: SharedPreviewFocus,
}

impl FocusMap {
    pub fn new(controller: InspectorController, state: SharedPreviewFocus) -> Rc<Self> {
        Rc::new(Self {
            entries: RefCell::new(Vec::new()),
            controller,
            state,
        })
    }

    pub fn clear(&self, target: &InspectorTarget) {
        self.entries.borrow_mut().clear();
        if let Some(focus) = preview_focus::snapshot(&self.state) {
            let valid = matches!(target, InspectorTarget::Item(item)
                if self.controller.valid_preview_focus(&focus.item, focus.target, item));
            if !valid {
                preview_focus::clear(&self.state);
            }
        }
    }

    pub fn register(&self, view: &NSView, target: &InspectorTarget, focus: ControlPreviewFocus) {
        self.entries.borrow_mut().push(Entry {
            view: Weak::new(view),
            target: target.clone(),
            focus,
        });
    }

    pub fn prune(&self) {
        self.entries
            .borrow_mut()
            .retain(|entry| entry.view.load().is_some());
    }

    pub fn toggle(
        &self,
        target: &InspectorTarget,
        action: &shrimply_inspector_core::document::InspectorToggleAction,
        enabled: bool,
    ) {
        if let Some(focus) = self
            .controller
            .toggle_preview_focus(target, action, enabled)
        {
            preview_focus::set(&self.state, focus);
        }
    }

    fn clicked(&self, hit: Retained<NSView>) {
        let entries = self.entries.borrow();
        let mut ancestor = Some(hit);
        while let Some(view) = ancestor {
            if let Some(entry) = entries.iter().find(|entry| {
                entry
                    .view
                    .load()
                    .is_some_and(|candidate| std::ptr::eq(&*candidate, &*view))
            }) {
                let focus = self
                    .controller
                    .resolve_preview_focus(&entry.target, &entry.focus);
                drop(entries);
                if let Some(focus) = focus {
                    preview_focus::set(&self.state, focus);
                }
                return;
            }
            // AppKit owns the hierarchy; keep each ancestor retained while traversing it.
            ancestor = unsafe { view.superview() };
        }
    }
}

pub(super) struct FocusMonitor(Retained<AnyObject>);

impl FocusMonitor {
    pub fn new(root: &NSView, map: &Rc<FocusMap>, mtm: MainThreadMarker) -> Self {
        let root = Weak::new(root);
        let map = Rc::downgrade(map);
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
            // Observe only the initial mouse-down; return the original event unchanged.
            // NumberPicker continues to own its nested drag tracking loop.
            let value = unsafe { event.as_ref() };
            if let (Some(root), Some(map), Some(window)) =
                (root.load(), map.upgrade(), value.window(mtm))
                && root
                    .window()
                    .as_deref()
                    .is_some_and(|owner| std::ptr::eq(owner, &*window))
                && let Some(parent) = unsafe { root.superview() }
            {
                let point = parent.convertPoint_fromView(value.locationInWindow(), None);
                if let Some(hit) = root.hitTest(point) {
                    map.clicked(hit);
                }
            }
            event.as_ptr()
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown,
                &handler,
            )
        }
        .expect("install inspector preview focus monitor");
        Self(monitor)
    }
}

impl Drop for FocusMonitor {
    fn drop(&mut self) {
        // One owner removes the monitor when its inspector is released.
        unsafe {
            NSEvent::removeMonitor(&self.0);
        }
    }
}
