use crate::{
    EffectRole, EffectView, FrameGraph, MultilineTextInput, action, column_append, column_stack,
    control_row_with_suffix, inset, row_stack,
};
use block2::RcBlock;
use objc2::MainThreadOnly;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{
    NSAnimationContext, NSButton, NSButtonType, NSColor, NSControlStateValueOn, NSImage,
    NSLayoutConstraint, NSStackView, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSRect, NSString};
use shrimply_components_core::layered::LayeredPropertyController;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const CARD_GAP: f64 = 8.0;
const CARD_ANIMATION_SECONDS: f64 = 0.18;
type ExpansionHandlers = Rc<RefCell<Vec<Box<dyn Fn(bool, bool)>>>>;

#[derive(Clone)]
struct CardHeight {
    natural: Retained<NSLayoutConstraint>,
    animated: Retained<NSLayoutConstraint>,
}

pub struct InspectorCard {
    root: Retained<NSView>,
    controls: Retained<NSStackView>,
    header_before_reset: Retained<NSStackView>,
    header_after_reset: Retained<NSStackView>,
    expansion_handlers: ExpansionHandlers,
}

impl InspectorCard {
    pub fn new(
        title: &str,
        expanded: bool,
        on_reset: impl Fn() + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        Self::build(title, expanded, Some(Box::new(on_reset)), mtm)
    }

    pub fn without_reset(title: &str, expanded: bool, mtm: MainThreadMarker) -> Self {
        Self::build(title, expanded, None, mtm)
    }

    fn build(
        title: &str,
        expanded: bool,
        on_reset: Option<Box<dyn Fn()>>,
        mtm: MainThreadMarker,
    ) -> Self {
        let effect = EffectView::new(NSRect::ZERO, EffectRole::Card, mtm);
        effect.set_corner_radius(8.0);
        let root = Retained::from(effect.view());
        let vertical = column_stack(0.0, mtm);
        let header = row_stack(4.0, mtm);
        let disclosure = unsafe {
            NSButton::buttonWithImage_target_action(
                &symbol(
                    if expanded {
                        "chevron.down"
                    } else {
                        "chevron.right"
                    },
                    "Expand",
                ),
                None,
                None,
                mtm,
            )
        };
        disclosure.setBordered(false);
        let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        label.setUsesSingleLineMode(true);
        label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
        label.setToolTip(Some(&NSString::from_str(title)));
        label.setContentCompressionResistancePriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityDefaultLow,
            objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
        );
        label.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(
            objc2_app_kit::NSFont::systemFontSize(),
        )));
        let spacer = NSView::new(mtm);
        let header_before_reset = row_stack(4.0, mtm);
        let header_after_reset = row_stack(4.0, mtm);
        header.addArrangedSubview(&disclosure);
        header.addArrangedSubview(&label);
        header.addArrangedSubview(&spacer);
        header.addArrangedSubview(&header_before_reset);
        if let Some(on_reset) = on_reset {
            let reset = unsafe {
                NSButton::buttonWithImage_target_action(
                    &symbol("arrow.counterclockwise", "Reset"),
                    None,
                    None,
                    mtm,
                )
            };
            reset.setBordered(false);
            reset.setToolTip(Some(&NSString::from_str("Reset")));
            action::attach(&reset, move |_| on_reset(), mtm);
            header.addArrangedSubview(&reset);
        }
        header.addArrangedSubview(&header_after_reset);
        let controls = column_stack(CARD_GAP, mtm);
        let controls_content = inset(
            &controls,
            NSEdgeInsets {
                top: 4.0,
                left: 12.0,
                bottom: 12.0,
                right: 12.0,
            },
            mtm,
        );
        // A clipping viewport can shrink without compressing the controls inside it.
        let controls_container = NSView::new(mtm);
        controls_container.setWantsLayer(true);
        controls_container
            .layer()
            .expect("card clipping layer")
            .setMasksToBounds(true);
        controls_content.setTranslatesAutoresizingMaskIntoConstraints(false);
        controls_container.addSubview(&controls_content);
        for constraint in [
            controls_content
                .leadingAnchor()
                .constraintEqualToAnchor(&controls_container.leadingAnchor()),
            controls_content
                .trailingAnchor()
                .constraintEqualToAnchor(&controls_container.trailingAnchor()),
            controls_content
                .topAnchor()
                .constraintEqualToAnchor(&controls_container.topAnchor()),
        ] {
            constraint.setActive(true);
        }
        let natural_height = controls_container
            .heightAnchor()
            .constraintEqualToAnchor(&controls_content.heightAnchor());
        natural_height.setActive(true);
        controls.setHuggingPriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityRequired,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        controls.setClippingResistancePriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityRequired,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        controls_container.setHidden(!expanded);
        let animation_height = controls_container
            .heightAnchor()
            .constraintEqualToConstant(0.0);
        let callback_container = controls_container.clone();
        let callback_root = Weak::new(&*root);
        let callback_height = CardHeight {
            natural: natural_height,
            animated: animation_height,
        };
        let expanded_state = Rc::new(Cell::new(expanded));
        let animation_generation = Rc::new(Cell::new(0_u64));
        let expansion_handlers = Rc::new(RefCell::new(Vec::<Box<dyn Fn(bool, bool)>>::new()));
        let callback_handlers = expansion_handlers.clone();
        action::attach(
            &disclosure,
            move |control| {
                let Some(root) = callback_root.load() else {
                    return;
                };
                let expanding = !expanded_state.get();
                expanded_state.set(expanding);
                let generation = animation_generation.get().wrapping_add(1);
                animation_generation.set(generation);
                for handler in callback_handlers.borrow().iter() {
                    handler(expanding, false);
                }
                animate_card_content(
                    root,
                    callback_container.clone(),
                    callback_height.clone(),
                    expanding,
                    generation,
                    animation_generation.clone(),
                    callback_handlers.clone(),
                );
                control
                    .downcast_ref::<NSButton>()
                    .expect("disclosure sender")
                    .setImage(Some(&symbol(
                        if expanding {
                            "chevron.down"
                        } else {
                            "chevron.right"
                        },
                        "Expand",
                    )));
            },
            mtm,
        );
        column_append(
            &vertical,
            &inset(
                &header,
                NSEdgeInsets {
                    top: 6.0,
                    left: 8.0,
                    bottom: 6.0,
                    right: 8.0,
                },
                mtm,
            ),
        );
        column_append(&vertical, &controls_container);
        vertical.setTranslatesAutoresizingMaskIntoConstraints(false);
        vertical.setHuggingPriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityRequired,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        vertical.setClippingResistancePriority_forOrientation(
            objc2_app_kit::NSLayoutPriorityRequired,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        effect.set_content_view(Some(&vertical));
        for constraint in [
            vertical
                .leadingAnchor()
                .constraintEqualToAnchor(&root.leadingAnchor()),
            vertical
                .trailingAnchor()
                .constraintEqualToAnchor(&root.trailingAnchor()),
            vertical
                .topAnchor()
                .constraintEqualToAnchor(&root.topAnchor()),
            vertical
                .bottomAnchor()
                .constraintEqualToAnchor(&root.bottomAnchor()),
        ] {
            constraint.setActive(true);
        }
        Self {
            root,
            controls,
            header_before_reset,
            header_after_reset,
            expansion_handlers,
        }
    }

    pub fn append(&self, child: &NSView) {
        column_append(&self.controls, child);
    }

    pub fn append_before_reset(&self, child: &NSView) {
        self.header_before_reset.addArrangedSubview(child);
    }

    pub fn append_after_reset(&self, child: &NSView) {
        self.header_after_reset.addArrangedSubview(child);
    }
    pub fn view(&self) -> &NSView {
        &self.root
    }

    pub fn connect_expansion(&self, handler: impl Fn(bool, bool) + 'static) {
        self.expansion_handlers.borrow_mut().push(Box::new(handler));
    }
}

fn animate_card_content(
    root: Retained<NSView>,
    container: Retained<NSView>,
    heights: CardHeight,
    expanding: bool,
    generation: u64,
    animation_generation: Rc<Cell<u64>>,
    handlers: ExpansionHandlers,
) {
    let layout = highest_ancestor(&root);
    let current_height = if container.isHidden() {
        0.0
    } else {
        container
            .layer()
            .and_then(|layer| unsafe { layer.presentationLayer() })
            .map_or(container.frame().size.height, |layer| {
                layer.frame().size.height
            })
    };
    layout.layoutSubtreeIfNeeded();
    let target = container.subviews().objectAtIndex(0).fittingSize().height;
    heights.natural.setActive(false);
    let height = heights.animated;
    if expanding {
        container.setHidden(false);
    }
    height.setConstant(current_height);
    height.setActive(true);
    invalidate_ancestor_layout(&container);
    layout.layoutSubtreeIfNeeded();
    let target = if expanding { target } else { 0.0 };
    let changes_layout = layout.clone();
    let changes_height = height.clone();
    let changes = RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
        unsafe { context.as_ref() }.setDuration(CARD_ANIMATION_SECONDS);
        unsafe { context.as_ref() }.setAllowsImplicitAnimation(true);
        changes_height.setConstant(target);
        changes_layout.layoutSubtreeIfNeeded();
    });
    let completion_layout = layout;
    let completion_container = container;
    let completion_height = height;
    let completion = RcBlock::new(move || {
        if animation_generation.get() != generation {
            return;
        }
        completion_container.setHidden(!expanding);
        completion_height.setActive(false);
        heights.natural.setActive(true);
        invalidate_ancestor_layout(&completion_container);
        completion_layout.layoutSubtreeIfNeeded();
        for handler in handlers.borrow().iter() {
            handler(expanding, true);
        }
    });
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&completion));
}

fn highest_ancestor(view: &NSView) -> Retained<NSView> {
    let mut highest = Retained::from(view);
    let mut ancestor = unsafe { view.superview() };
    while let Some(view) = ancestor {
        ancestor = unsafe { view.superview() };
        highest = view;
    }
    highest
}

pub struct ExpressionEditor {
    root: Retained<NSStackView>,
    output: Retained<NSTextField>,
    _editor: MultilineTextInput,
}

impl ExpressionEditor {
    pub fn new(
        source: &str,
        output: &str,
        on_edit: impl Fn(String) -> String + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        let root = column_stack(4.0, mtm);
        let output_field = NSTextField::labelWithString(&NSString::from_str(output), mtm);
        output_field.setAlignment(NSTextAlignment::Right);
        output_field.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        let callback_output = output_field.clone();
        let editor = MultilineTextInput::code(
            source,
            180.0,
            move |source| {
                callback_output.setStringValue(&NSString::from_str(&on_edit(source)));
                true
            },
            || {},
            mtm,
        );
        column_append(&root, editor.view());
        column_append(&root, &output_field);
        Self {
            root,
            output: output_field,
            _editor: editor,
        }
    }

    pub fn view(&self) -> &NSStackView {
        &self.root
    }
    pub fn set_output(&self, value: &str) {
        self.output.setStringValue(&NSString::from_str(value));
    }
}

pub struct InspectorGraphProperty {
    root: Retained<NSStackView>,
    keyframes: Retained<NSButton>,
    expression: Retained<NSButton>,
    graph: FrameGraph,
    controller: LayeredPropertyController,
    _expression_editor: ExpressionEditor,
}

impl InspectorGraphProperty {
    pub fn new(
        label: &str,
        editor: &NSView,
        graph: FrameGraph,
        expression_editor: ExpressionEditor,
        controller: LayeredPropertyController,
        mtm: MainThreadMarker,
    ) -> Self {
        let root = column_stack(6.0, mtm);
        let keyframes = unsafe {
            NSButton::buttonWithImage_target_action(
                &symbol("stopwatch", "Toggle keyframes"),
                None,
                None,
                mtm,
            )
        };
        keyframes.setButtonType(NSButtonType::PushOnPushOff);
        keyframes.setBordered(false);
        keyframes.setToolTip(Some(&NSString::from_str("Toggle keyframes")));
        set_toggle_tint(&keyframes, false);
        let expression = unsafe {
            NSButton::buttonWithImage_target_action(
                &symbol(
                    "chevron.left.forwardslash.chevron.right",
                    "Toggle expression",
                ),
                None,
                None,
                mtm,
            )
        };
        expression.setButtonType(NSButtonType::PushOnPushOff);
        expression.setBordered(false);
        expression.setToolTip(Some(&NSString::from_str("Toggle expression")));
        set_toggle_tint(&expression, false);
        let suffix = row_stack(4.0, mtm);
        suffix.addArrangedSubview(&keyframes);
        suffix.addArrangedSubview(&expression);
        column_append(
            &root,
            &control_row_with_suffix(label, editor, Some(&suffix), mtm),
        );
        graph.view().setHidden(true);
        expression_editor.view().setHidden(true);
        column_append(&root, graph.view());
        column_append(&root, expression_editor.view());
        graph
            .view()
            .widthAnchor()
            .constraintEqualToAnchor(&root.widthAnchor())
            .setActive(true);
        expression_editor
            .view()
            .widthAnchor()
            .constraintEqualToAnchor(&root.widthAnchor())
            .setActive(true);
        let graph_view = graph.retained_view();
        let graph_controller = controller.clone();
        action::attach(
            &keyframes,
            move |control| {
                let active = control
                    .downcast_ref::<NSButton>()
                    .expect("keyframe toggle sender")
                    .state()
                    == NSControlStateValueOn;
                graph_controller.set_keyframes(active);
                set_toggle_tint(
                    control
                        .downcast_ref::<NSButton>()
                        .expect("keyframe toggle sender"),
                    active,
                );
                graph_view.setHidden(!active);
                invalidate_ancestor_layout(&graph_view);
            },
            mtm,
        );
        let expression_view = expression_editor.root.clone();
        let expression_controller = controller.clone();
        action::attach(
            &expression,
            move |control| {
                let active = control
                    .downcast_ref::<NSButton>()
                    .expect("expression toggle sender")
                    .state()
                    == NSControlStateValueOn;
                expression_controller.set_expression(active);
                set_toggle_tint(
                    control
                        .downcast_ref::<NSButton>()
                        .expect("expression toggle sender"),
                    active,
                );
                expression_view.setHidden(!active);
                invalidate_ancestor_layout(&expression_view);
            },
            mtm,
        );
        Self {
            root,
            keyframes,
            expression,
            graph,
            controller,
            _expression_editor: expression_editor,
        }
    }

    pub fn view(&self) -> &NSStackView {
        &self.root
    }

    pub fn into_view(self) -> Retained<NSStackView> {
        self.root
    }

    pub fn graph(&self) -> &FrameGraph {
        &self.graph
    }
    pub fn set_keyframes_active(&self, active: bool) {
        self.controller.set_keyframes(active);
        self.keyframes
            .setState(if active { NSControlStateValueOn } else { 0 });
        set_toggle_tint(&self.keyframes, active);
        self.graph.view().setHidden(!active);
        invalidate_ancestor_layout(self.graph.view());
    }
    pub fn set_expression_active(&self, active: bool) {
        self.controller.set_expression(active);
        self.expression
            .setState(if active { NSControlStateValueOn } else { 0 });
        set_toggle_tint(&self.expression, active);
        self._expression_editor.view().setHidden(!active);
        invalidate_ancestor_layout(self._expression_editor.view());
    }
}

fn set_toggle_tint(button: &NSButton, active: bool) {
    let color = if active {
        NSColor::controlAccentColor()
    } else {
        NSColor::secondaryLabelColor()
    };
    button.setContentTintColor(Some(&color));
}

fn invalidate_ancestor_layout(view: &NSView) {
    let mut ancestor = unsafe { view.superview() };
    while let Some(view) = ancestor {
        view.invalidateIntrinsicContentSize();
        view.setNeedsLayout(true);
        ancestor = unsafe { view.superview() };
    }
}

fn symbol(name: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(label)),
    )
    .unwrap_or_else(|| panic!("macOS must provide the {name} system symbol"))
}
