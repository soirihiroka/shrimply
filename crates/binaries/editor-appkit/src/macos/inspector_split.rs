use super::{
    Editor,
    layout_split::{Dimension, LayoutSplit, PANEL_ANIMATION_SECONDS},
};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBox, NSBoxType, NSColor, NSShadow,
    NSSplitView, NSSplitViewDelegate, NSSplitViewDividerStyle, NSTitlePosition, NSView,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize};
use shrimply_editor_state::preferences::{self, SharedPreferences};
use std::cell::Cell;

const INSPECTOR_MIN_WIDTH: f64 = shrimply_components_core::layout::INSPECTOR_MIN_WIDTH as f64;
const OVERLAY_SHADOW_BLUR_RADIUS: f64 = 12.0;
const PREVIEW_MIN_WIDTH: f64 = shrimply_components_core::layout::MIN_PREVIEW_WIDTH as f64;

struct ResponsiveState {
    split: Retained<LayoutSplit>,
    inspector: Retained<NSView>,
    preview: Retained<NSView>,
    overlay: Cell<bool>,
    preferences: SharedPreferences,
    shadow: Retained<NSShadow>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ResponsiveState]
    struct ResponsiveInspector;

    unsafe impl NSObjectProtocol for ResponsiveInspector {}

    impl ResponsiveInspector {
        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe { let _: () = msg_send![super(self), layout]; }
            self.update();
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            unsafe { let _: () = msg_send![super(self), setFrameSize: size]; }
            self.setNeedsLayout(true);
        }
    }
);

impl ResponsiveInspector {
    fn update(&self) {
        let state = self.ivars();
        if state.split.is_animating() {
            return;
        }
        let bounds = self.bounds();
        let width = if state.split.tracking() {
            state.inspector.frame().size.width
        } else {
            f64::from(
                preferences::snapshot(&state.preferences)
                    .editor_layout
                    .inspector_width,
            )
        };
        let overlay = shrimply_components_core::layout::inspector_overlay(bounds.size.width as f32);
        if overlay != state.overlay.replace(overlay) {
            state
                .inspector
                .setShadow(if overlay { Some(&state.shadow) } else { None });
            state.inspector.removeFromSuperview();
            if overlay {
                self.addSubview(&state.inspector);
            } else {
                state
                    .split
                    .insertArrangedSubview_atIndex(&state.inspector, 0);
                state.inspector.setFrame(NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(width, bounds.size.height),
                ));
            }
        }
        state.split.setFrame(bounds);
        if overlay {
            state.preview.setFrame(state.split.bounds());
            state.inspector.setFrame(NSRect::new(
                bounds.origin,
                NSSize::new(width.min(bounds.size.width), bounds.size.height),
            ));
        } else {
            state.split.adjustSubviews();
            state.split.restore();
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct DividerDelegate;

    unsafe impl NSObjectProtocol for DividerDelegate {}

    unsafe impl NSSplitViewDelegate for DividerDelegate {
        #[unsafe(method(splitView:constrainMinCoordinate:ofSubviewAt:))]
        fn minimum(&self, _split: &NSSplitView, _proposed: f64, _index: isize) -> f64 {
            INSPECTOR_MIN_WIDTH
        }

        #[unsafe(method(splitView:constrainMaxCoordinate:ofSubviewAt:))]
        fn maximum(&self, split: &NSSplitView, _proposed: f64, _index: isize) -> f64 {
            split.bounds().size.width - split.dividerThickness() - PREVIEW_MIN_WIDTH
        }
    }
);

/// AppKit owns divider sizing; only the inspector moves into the narrow overlay.
/// The preview remains in its pane while the shared layout policy changes.
pub struct InspectorSplit {
    host: Retained<ResponsiveInspector>,
    inspector: Retained<NSView>,
    _delegate: Retained<DividerDelegate>,
}

impl InspectorSplit {
    pub fn new(
        editor: &Editor,
        inspector: &NSView,
        preview: &NSView,
        size: NSSize,
        mtm: MainThreadMarker,
    ) -> Self {
        let preferences = editor
            .ivars()
            .session
            .get()
            .expect("project loaded")
            .preferences
            .clone();
        let width = f64::from(
            preferences::snapshot(&preferences)
                .editor_layout
                .inspector_width,
        );
        let split = LayoutSplit::new(editor, Dimension::Inspector, size);
        split.setVertical(true);
        split.setDividerStyle(NSSplitViewDividerStyle::Thin);
        let delegate: Retained<DividerDelegate> =
            unsafe { msg_send![DividerDelegate::alloc(mtm), init] };
        split.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let inspector_content = pane(inspector, mtm);
        inspector_content.setClipsToBounds(true);
        let surface = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
        surface.setBoxType(NSBoxType::Custom);
        surface.setTitlePosition(NSTitlePosition::NoTitle);
        surface.setBorderWidth(0.0);
        surface.setContentViewMargins(NSSize::ZERO);
        surface.setFillColor(&NSColor::windowBackgroundColor().colorWithAlphaComponent(1.0));
        surface.setContentView(Some(&inspector_content));
        surface.setWantsLayer(true);
        surface.setClipsToBounds(false);
        let inspector: Retained<NSView> = surface.into_super();
        let shadow = NSShadow::new();
        shadow.setShadowBlurRadius(OVERLAY_SHADOW_BLUR_RADIUS);
        shadow.setShadowOffset(NSSize::ZERO);
        let preview = pane(preview, mtm);
        inspector.setFrame(NSRect::new(
            NSPoint::ZERO,
            NSSize::new(width.min(size.width), size.height),
        ));
        let preview_x = width + split.dividerThickness();
        preview.setFrame(NSRect::new(
            NSPoint::new(preview_x, 0.0),
            NSSize::new((size.width - preview_x).max(0.0), size.height),
        ));
        split.addSubview(&inspector);
        split.addSubview(&preview);
        let host = ResponsiveInspector::alloc(mtm).set_ivars(ResponsiveState {
            split: split.clone(),
            inspector: inspector.clone(),
            preview,
            overlay: Cell::new(false),
            preferences,
            shadow,
        });
        let host: Retained<ResponsiveInspector> =
            unsafe { msg_send![super(host), initWithFrame: NSRect::new(NSPoint::ZERO, size)] };
        host.setClipsToBounds(true);
        host.addSubview(&split);
        host.update();
        Self {
            host,
            inspector,
            _delegate: delegate,
        }
    }

    pub fn view(&self) -> &NSView {
        self.host.as_super()
    }

    pub fn set_collapsed(&self, collapsed: bool, animated: bool) {
        let state = self.host.ivars();
        if !animated {
            state.split.cancel_animation();
            if self.inspector.isHidden() != collapsed {
                self.inspector.setHidden(collapsed);
            }
            self.host.update();
            return;
        }
        let bounds = self.host.bounds();
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            self.set_collapsed(collapsed, false);
            return;
        }
        let saved = f64::from(
            preferences::snapshot(&state.preferences)
                .editor_layout
                .inspector_width,
        );
        let overlay = state.overlay.get();
        let divider = state.split.dividerThickness();
        let max_width = f64::from(shrimply_components_core::layout::inspector_max_width(
            bounds.size.width as f32,
        ));
        let width = saved.clamp(INSPECTOR_MIN_WIDTH.min(max_width), max_width);
        let height = bounds.size.height;
        let generation = state.split.begin_animation();

        if !collapsed && self.inspector.isHidden() {
            if overlay {
                self.inspector.setFrame(NSRect::new(
                    NSPoint::new(-width, bounds.origin.y),
                    NSSize::new(width, height),
                ));
            } else {
                self.inspector
                    .setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(0.0, height)));
                state.preview.setFrame(NSRect::new(
                    NSPoint::ZERO,
                    NSSize::new(bounds.size.width, height),
                ));
            }
            self.inspector.setHidden(false);
        }

        let target_inspector = if overlay {
            NSRect::new(
                NSPoint::new(if collapsed { -width } else { 0.0 }, bounds.origin.y),
                NSSize::new(width, height),
            )
        } else {
            NSRect::new(
                NSPoint::ZERO,
                NSSize::new(if collapsed { 0.0 } else { width }, height),
            )
        };
        let target_preview = if collapsed {
            NSRect::new(NSPoint::ZERO, NSSize::new(bounds.size.width, height))
        } else {
            let x = width + divider;
            NSRect::new(
                NSPoint::new(x, 0.0),
                NSSize::new((bounds.size.width - x).max(0.0), height),
            )
        };

        let anim_inspector = self.inspector.clone();
        let anim_preview = state.preview.clone();
        let changes = RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
            unsafe { context.as_ref() }.setDuration(PANEL_ANIMATION_SECONDS);
            unsafe { context.as_ref() }.setAllowsImplicitAnimation(true);
            anim_inspector.animator().setFrame(target_inspector);
            if !overlay {
                anim_preview.animator().setFrame(target_preview);
            }
        });
        let done_host = self.host.clone();
        let done_inspector = self.inspector.clone();
        let completion = RcBlock::new(move || {
            if !done_host.ivars().split.finish_animation(generation) {
                return;
            }
            if collapsed {
                done_inspector.setHidden(true);
            }
            done_host.update();
        });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&completion));
    }
}

fn pane(content: &NSView, mtm: MainThreadMarker) -> Retained<NSView> {
    let pane = NSView::new(mtm);
    content.setTranslatesAutoresizingMaskIntoConstraints(false);
    pane.addSubview(content);
    for constraint in [
        content
            .leadingAnchor()
            .constraintEqualToAnchor(&pane.leadingAnchor()),
        content
            .trailingAnchor()
            .constraintEqualToAnchor(&pane.trailingAnchor()),
        content
            .topAnchor()
            .constraintEqualToAnchor(&pane.topAnchor()),
        content
            .bottomAnchor()
            .constraintEqualToAnchor(&pane.bottomAnchor()),
    ] {
        constraint.setActive(true);
    }
    pane
}
