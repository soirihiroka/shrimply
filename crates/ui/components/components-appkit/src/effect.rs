use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSGlassEffectView, NSGlassEffectViewStyle, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectView,
};
use objc2_foundation::{MainThreadMarker, NSRect};
use std::cell::RefCell;

pub enum EffectRole {
    Window,
    Sidebar,
    Card,
    RecentProject,
    Controls,
}

enum Effect {
    Glass(Retained<NSGlassEffectView>),
    Visual {
        view: Retained<NSVisualEffectView>,
        content: RefCell<Option<Retained<NSView>>>,
    },
}

pub struct EffectView {
    effect: Effect,
}

impl EffectView {
    pub fn new(frame: NSRect, role: EffectRole, mtm: MainThreadMarker) -> Self {
        // Typed class lookup (including downcasting) would panic on macOS 15.
        let effect = if AnyClass::get(c"NSGlassEffectView").is_some() {
            let view = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
            view.setStyle(match role {
                EffectRole::RecentProject => NSGlassEffectViewStyle::Clear,
                _ => NSGlassEffectViewStyle::Regular,
            });
            if matches!(role, EffectRole::Window) {
                view.setTintColor(Some(&NSColor::windowBackgroundColor()));
            }
            Effect::Glass(view)
        } else {
            let view = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
            let (material, blending) = match role {
                EffectRole::Window => (
                    NSVisualEffectMaterial::WindowBackground,
                    NSVisualEffectBlendingMode::BehindWindow,
                ),
                EffectRole::Sidebar => (
                    NSVisualEffectMaterial::Sidebar,
                    NSVisualEffectBlendingMode::BehindWindow,
                ),
                EffectRole::Card | EffectRole::RecentProject => (
                    NSVisualEffectMaterial::Popover,
                    NSVisualEffectBlendingMode::WithinWindow,
                ),
                EffectRole::Controls => (
                    NSVisualEffectMaterial::HUDWindow,
                    NSVisualEffectBlendingMode::WithinWindow,
                ),
            };
            view.setMaterial(material);
            view.setBlendingMode(blending);
            Effect::Visual {
                view,
                content: RefCell::new(None),
            }
        };
        Self { effect }
    }

    pub fn view(&self) -> &NSView {
        match &self.effect {
            Effect::Glass(view) => view.as_super(),
            Effect::Visual { view, .. } => view.as_super(),
        }
    }

    pub fn set_corner_radius(&self, radius: f64) {
        match &self.effect {
            Effect::Glass(view) => view.setCornerRadius(radius),
            Effect::Visual { view, .. } => {
                view.setWantsLayer(true);
                let layer = view.layer().expect("visual effect backing layer");
                layer.setCornerRadius(radius);
                layer.setMasksToBounds(true);
            }
        }
    }

    pub fn set_content_view(&self, content: Option<&NSView>) {
        match &self.effect {
            Effect::Glass(view) => view.setContentView(content),
            Effect::Visual {
                view,
                content: previous,
            } => {
                if let Some(previous) = previous.replace(content.map(Retained::from)) {
                    previous.removeFromSuperview();
                }
                if let Some(content) = content {
                    // Auto Layout callers own their edge constraints. Frame-based
                    // callers need the same fill behavior as glass content views.
                    if content.translatesAutoresizingMaskIntoConstraints() {
                        content.setFrame(view.bounds());
                        content.setAutoresizingMask(
                            NSAutoresizingMaskOptions::ViewWidthSizable
                                | NSAutoresizingMaskOptions::ViewHeightSizable,
                        );
                    }
                    view.addSubview(content);
                }
            }
        }
    }
}
