use super::Editor;
use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSSplitView, NSSplitViewItem};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};
use shrimply_editor_state::preferences::{self, SharedPreferences};
use std::cell::Cell;

#[derive(Clone, Copy)]
pub(super) enum Dimension {
    Inspector,
    Timeline,
}

pub(super) struct SplitState {
    preferences: SharedPreferences,
    editor: Weak<Editor>,
    dimension: Dimension,
    tracking: Cell<bool>,
    restoring: Cell<bool>,
    animating: Cell<bool>,
    animation_generation: Cell<u64>,
}

define_class!(
    #[unsafe(super(NSSplitView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitState]
    pub(super) struct LayoutSplit;

    unsafe impl NSObjectProtocol for LayoutSplit {}

    impl LayoutSplit {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let before = self.subviews().iter().map(|view| (view.frame(), view.isHidden(), self.isSubviewCollapsed(&view))).collect::<Vec<_>>();
            self.ivars().tracking.set(true);
            unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            self.ivars().tracking.set(false);
            if before != self.subviews().iter().map(|view| (view.frame(), view.isHidden(), self.isSubviewCollapsed(&view))).collect::<Vec<_>>() {
                self.save();
            }
            self.restore();
        }

        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resize_subviews(&self, size: NSSize) {
            unsafe { let _: () = msg_send![super(self), resizeSubviewsWithOldSize: size]; }
            self.restore();
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe { let _: () = msg_send![super(self), layout]; }
            self.restore();
        }
    }
);

impl LayoutSplit {
    pub(super) fn new(editor: &Editor, dimension: Dimension, size: NSSize) -> Retained<Self> {
        let split = Self::alloc(editor.mtm()).set_ivars(SplitState {
            preferences: editor
                .ivars()
                .session
                .get()
                .expect("project loaded")
                .preferences
                .clone(),
            editor: Weak::new(editor),
            dimension,
            tracking: Cell::new(false),
            restoring: Cell::new(false),
            animating: Cell::new(false),
            animation_generation: Cell::new(0),
        });
        unsafe { msg_send![super(split), initWithFrame: NSRect::new(NSPoint::ZERO, size)] }
    }

    pub(super) fn is_animating(&self) -> bool {
        self.ivars().animating.get()
    }

    pub(super) fn begin_animation(&self) -> u64 {
        let generation = self.ivars().animation_generation.get().wrapping_add(1);
        self.ivars().animation_generation.set(generation);
        self.ivars().animating.set(true);
        generation
    }

    pub(super) fn finish_animation(&self, generation: u64) -> bool {
        if self.ivars().animation_generation.get() != generation {
            return false;
        }
        self.ivars().animating.set(false);
        true
    }

    pub(super) fn cancel_animation(&self) {
        self.ivars()
            .animation_generation
            .set(self.ivars().animation_generation.get().wrapping_add(1));
        self.ivars().animating.set(false);
    }

    pub(super) fn sync_preferred_thickness(&self, item: &NSSplitViewItem) {
        let height = self.bounds().size.height;
        if height <= 0.0 {
            return;
        }
        let fraction = preferences::snapshot(&self.ivars().preferences)
            .editor_layout
            .timeline_fraction;
        let extent = shrimply_components_core::math::split_pane_extent(
            height as f32,
            self.dividerThickness() as f32,
            fraction,
        );
        item.setPreferredThicknessFraction(f64::from(extent) / height);
    }

    pub(super) fn tracking(&self) -> bool {
        self.ivars().tracking.get()
    }

    fn panes_visible(&self) -> bool {
        let panes = self.arrangedSubviews();
        panes.len() == 2
            && panes
                .iter()
                .all(|view| !view.isHidden() && !self.isSubviewCollapsed(&view))
    }

    /// Allocation may clamp the actual panes; only native divider tracking saves a new desire.
    pub(super) fn restore(&self) {
        if self.tracking()
            || self.ivars().restoring.get()
            || self.ivars().animating.get()
            || !self.panes_visible()
        {
            return;
        }
        let layout = preferences::snapshot(&self.ivars().preferences).editor_layout;
        let desired = match self.ivars().dimension {
            Dimension::Inspector => f64::from(layout.inspector_width),
            Dimension::Timeline => f64::from(shrimply_components_core::math::split_pane_extent(
                self.bounds().size.height as f32,
                self.dividerThickness() as f32,
                1.0 - layout.timeline_fraction,
            )),
        };
        let minimum = self.minPossiblePositionOfDividerAtIndex(0);
        let maximum = self.maxPossiblePositionOfDividerAtIndex(0);
        if maximum < minimum {
            return;
        }
        let target = desired.clamp(minimum, maximum);
        let frame = self.arrangedSubviews().objectAtIndex(0).frame();
        let actual = if self.isVertical() {
            frame.origin.x + frame.size.width
        } else {
            frame.origin.y + frame.size.height
        };
        let scale = self
            .window()
            .map_or(1.0, |window| window.backingScaleFactor());
        if (actual - target).abs() < 1.0 / scale {
            return;
        }
        self.ivars().restoring.set(true);
        self.setPosition_ofDividerAtIndex(target, 0);
        self.ivars().restoring.set(false);
    }

    fn save(&self) {
        let panes = self.arrangedSubviews();
        if panes.len() != 2 {
            return;
        }
        let inspector = panes.objectAtIndex(0);
        let timeline = panes.objectAtIndex(1);
        let visible = match self.ivars().dimension {
            Dimension::Inspector => !inspector.isHidden() && !self.isSubviewCollapsed(&inspector),
            Dimension::Timeline => !timeline.isHidden() && !self.isSubviewCollapsed(&timeline),
        };
        let result = preferences::update_editor_layout(&self.ivars().preferences, |layout| {
            match self.ivars().dimension {
                Dimension::Inspector => {
                    layout.inspector_visible = visible;
                    if visible {
                        layout.inspector_width = inspector.frame().size.width as f32;
                    }
                }
                Dimension::Timeline => {
                    layout.timeline_visible = visible;
                    if visible
                        && let Some(fraction) = shrimply_components_core::math::split_pane_fraction(
                            inspector.frame().size.height as f32,
                            timeline.frame().size.height as f32,
                        )
                    {
                        layout.timeline_fraction = fraction;
                    }
                }
            }
        });
        if let Err(error) = result
            && let Some(editor) = self.ivars().editor.load()
        {
            editor.show_error(&error);
        }
    }
}
