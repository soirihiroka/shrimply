mod fonts;

use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSColor, NSFont, NSGlassEffectView, NSGlassEffectViewStyle, NSScrollView,
    NSStackView, NSTextField, NSTextView, NSTitlebarSeparatorStyle, NSToolbar, NSView, NSWindow,
    NSWindowStyleMask, NSWindowTitleVisibility, NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSURL,
};
use shrimply_components_appkit::{
    ColorPicker, ExpressionEditor, FrameGraph, InspectorCard, InspectorGraphProperty,
    MultilineTextInput, Number2Picker, NumberPicker, ProgressButton, ProgressButtonState,
    ReadOnlyField, SingleLineTextInput, StringChoice, StringSelector, Tabs, column_append,
    column_stack, control_row, live_performance, modifier_menu, playback_shortcuts, row_stack,
    split_button, switch_row,
};
use shrimply_components_core::layered::{
    LayeredEdit, LayeredPropertyController, component_changes,
};
use std::cell::{Cell, OnceCell};
use std::path::PathBuf;
use std::rc::Rc;

const WINDOW_SIZE: NSSize = NSSize::new(920.0, 900.0);
const PAGE_INSET: f64 = 16.0;
const PAGE_GAP: f64 = 10.0;

struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn launched(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::ZERO, WINDOW_SIZE),
                    NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable
                        | NSWindowStyleMask::FullSizeContentView,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe {
                window.setReleasedWhenClosed(false);
            }
            window.setTitle(&NSString::from_str("Shrimply AppKit Components"));
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            window.setTitlebarAppearsTransparent(true);
            window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
            window.setToolbar(Some(&NSToolbar::new(mtm)));
            window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
            window.setMovableByWindowBackground(true);
            window.setContentMinSize(NSSize::ZERO);
            window.setContentMaxSize(NSSize::new(f64::MAX, f64::MAX));
            window.setResizeIncrements(NSSize::new(1.0, 1.0));
            let showcase = build_showcase(mtm);
            window.setContentView(Some(&window_shell(showcase.view(), mtm)));
            window.center();
            window.makeKeyAndOrderFront(None);
            self.ivars()
                .window
                .set(window)
                .expect("showcase window already installed");
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.activate();
        }
    }
);

pub fn run() {
    let mtm = MainThreadMarker::new().expect("AppKit demo must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::alloc(mtm).set_ivars(DelegateIvars {
        window: OnceCell::new(),
    });
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

fn window_shell(content: &NSView, mtm: MainThreadMarker) -> Retained<NSGlassEffectView> {
    let glass = NSGlassEffectView::initWithFrame(
        NSGlassEffectView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, WINDOW_SIZE),
    );
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setTintColor(Some(&NSColor::windowBackgroundColor()));
    glass.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, WINDOW_SIZE));
    root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let title =
        NSTextField::labelWithString(&NSString::from_str("Shrimply AppKit Components"), mtm);
    title.setFont(Some(
        &NSFont::boldSystemFontOfSize(NSFont::systemFontSize()),
    ));
    title.setTranslatesAutoresizingMaskIntoConstraints(false);
    content.setTranslatesAutoresizingMaskIntoConstraints(false);
    root.addSubview(&title);
    root.addSubview(content);
    for constraint in [
        title
            .centerXAnchor()
            .constraintEqualToAnchor(&root.centerXAnchor()),
        title
            .topAnchor()
            .constraintEqualToAnchor_constant(&root.topAnchor(), 14.0),
        content
            .leadingAnchor()
            .constraintEqualToAnchor(&root.leadingAnchor()),
        content
            .trailingAnchor()
            .constraintEqualToAnchor(&root.trailingAnchor()),
        content
            .topAnchor()
            .constraintEqualToAnchor_constant(&root.topAnchor(), 44.0),
        content
            .bottomAnchor()
            .constraintEqualToAnchor(&root.bottomAnchor()),
    ] {
        constraint.setActive(true);
    }
    glass.setContentView(Some(&root));
    glass
}

fn build_showcase(mtm: MainThreadMarker) -> Tabs {
    let events = NSTextView::initWithFrame(NSTextView::alloc(mtm), NSRect::ZERO);
    events.setEditable(false);
    events.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        NSFont::systemFontSize(),
        unsafe { objc2_app_kit::NSFontWeightRegular },
    )));
    let log: Rc<dyn Fn(String)> = Rc::new({
        let events = events.clone();
        move |message| {
            events.setString(&NSString::from_str(&format!(
                "{message}\n{}",
                events.string()
            )));
        }
    });
    Tabs::new(
        vec![
            ("General", general_page(log.clone(), mtm)),
            ("Fonts", fonts::page(log.clone(), mtm)),
            ("Info", info_page(log, mtm)),
            ("Log", text_scroll(events, mtm).into_super()),
        ],
        mtm,
    )
}

fn general_page(log: Rc<dyn Fn(String)>, mtm: MainThreadMarker) -> Retained<NSView> {
    let general = page_stack(mtm);
    let number = NumberPicker::builder(12.5)
        .accepted_range(-100.0, 100.0)
        .drag_step(0.25)
        .digits(2)
        .unit_name("px")
        .on_change({
            let log = log.clone();
            move |value| log(format!("number changed {value}"))
        })
        .on_commit({
            let log = log.clone();
            move |value| log(format!("number committed {value}"))
        })
        .build(mtm);
    column_append(&general, &control_row("Number", &number, mtm));

    let pair = Number2Picker::builder(1920.0, 1080.0)
        .minimum(1.0)
        .maximum(16_384.0)
        .digits(0)
        .first_prefix("W")
        .second_prefix("H")
        .unit_name("px")
        .enable_lock()
        .on_change({
            let log = log.clone();
            move |values, component| log(format!("pair {component} {}", values[component]))
        })
        .build_with_handles(mtm);
    column_append(&general, &control_row("Pair", &pair.widget, mtm));

    let vector = shrimply_components_appkit::Number3Picker::builder([1.0, 2.0, 3.0])
        .prefixes(["X", "Y", "Z"])
        .enable_lock()
        .on_change({
            let log = log.clone();
            move |values, component| log(format!("vector {component}: {values:?}"))
        })
        .build_with_handles(mtm);
    column_append(&general, &control_row("Vector", &vector.widget, mtm));

    let single = SingleLineTextInput::new(
        "Editable text",
        Some("Type here"),
        Some(40),
        |_| {},
        {
            let log = log.clone();
            move |value| log(format!("text committed {value}"))
        },
        mtm,
    );
    column_append(&general, &control_row("Single line", single.view(), mtm));
    let multiline = MultilineTextInput::new(
        "Try a typo such as teh.",
        110.0,
        Some(240),
        |_| true,
        {
            let log = log.clone();
            move || log("multiline committed".to_string())
        },
        mtm,
    );
    column_append(&general, &control_row("Multiline", multiline.view(), mtm));
    let selector = StringSelector::new(
        "two",
        [
            "one", "two", "three", "four", "five", "six", "seven", "eight",
        ]
        .into_iter()
        .map(|value| StringChoice {
            value: value.to_string(),
            label: value.to_uppercase(),
        })
        .collect(),
        {
            let log = log.clone();
            move |value| log(format!("selected {value}"))
        },
        mtm,
    );
    column_append(
        &general,
        &control_row("Searchable dropdown", selector.view(), mtm),
    );

    let position = pair_property(
        PairPropertyConfig {
            label: "Position",
            initial: [960.0, 540.0],
            prefixes: ["X", "Y"],
            unit: Some("px"),
            digits: 0,
            modes: (true, true),
            minimum: None,
            locked: false,
        },
        log.clone(),
        mtm,
    );
    let anchor = pair_property(
        PairPropertyConfig {
            label: "Anchor",
            initial: [960.0, 540.0],
            prefixes: ["X", "Y"],
            unit: Some("px"),
            digits: 0,
            modes: (false, false),
            minimum: None,
            locked: false,
        },
        log.clone(),
        mtm,
    );
    let scale = pair_property(
        PairPropertyConfig {
            label: "Scale",
            initial: [1.0, 1.0],
            prefixes: ["X", "Y"],
            unit: Some("x"),
            digits: 2,
            modes: (false, false),
            minimum: Some(0.0),
            locked: true,
        },
        log.clone(),
        mtm,
    );
    let shear = pair_property(
        PairPropertyConfig {
            label: "Shear",
            initial: [0.0, 0.0],
            prefixes: ["X", "Y"],
            unit: None,
            digits: 2,
            modes: (false, false),
            minimum: None,
            locked: false,
        },
        log.clone(),
        mtm,
    );
    let rotation = scalar_property("Rotation", 0.0, log.clone(), mtm);
    let card = InspectorCard::new(
        "Transform",
        true,
        {
            let log = log.clone();
            let resets = [
                position.reset.clone(),
                anchor.reset.clone(),
                scale.reset.clone(),
                shear.reset.clone(),
                rotation.reset.clone(),
            ];
            move || {
                for reset in &resets {
                    reset();
                }
                log("transform reset".to_string());
            }
        },
        mtm,
    );
    for view in [
        position.view,
        anchor.view,
        scale.view,
        shear.view,
        rotation.view,
    ] {
        card.append(&view);
    }
    let modifiers = modifier_menu(
        shrimply_components_demo_core::modifier_names()
            .into_iter()
            .map(|name| StringChoice {
                value: name.to_string(),
                label: name.to_string(),
            })
            .collect::<Vec<_>>()
            .into(),
        {
            let log = log.clone();
            move |value| log(format!("add modifier {value}"))
        },
        mtm,
    );
    let transform_group = column_stack(0.0, mtm);
    column_append(&transform_group, card.view());
    column_append(&transform_group, modifiers.view());
    column_append(&general, &transform_group);
    column_append(&general, &live_performance(mtm));
    column_append(
        &general,
        &switch_row(
            "Enabled",
            Some("Toggle this option"),
            true,
            {
                let log = log.clone();
                move |value| log(format!("switch {value}"))
            },
            mtm,
        ),
    );
    let color = ColorPicker::new(
        shrimply_math_color::Color::new(0x35, 0x84, 0xe4, 0xcc),
        {
            let log = log.clone();
            move |value| {
                log(format!(
                    "color #{:02X}{:02X}{:02X}{:02X}",
                    value.r, value.g, value.b, value.a
                ))
            }
        },
        {
            let log = log.clone();
            move || log("color committed".to_string())
        },
        mtm,
    );
    column_append(&general, &control_row("Color", color.view(), mtm));
    let split = split_button(
        "Primary",
        "Secondary",
        {
            let log = log.clone();
            move || log("primary".to_string())
        },
        {
            let log = log.clone();
            move || log("secondary".to_string())
        },
        mtm,
    );
    column_append(&general, &control_row("Split", &split, mtm));
    let progress = row_stack(8.0, mtm);
    let idle = ProgressButton::new("Idle", mtm);
    let working = ProgressButton::new("Working", mtm);
    working.set_state(ProgressButtonState::Indeterminate);
    let half = ProgressButton::new("Half", mtm);
    half.set_state(ProgressButtonState::Progress(0.5));
    let progress_spacer = NSView::new(mtm);
    progress_spacer.setContentHuggingPriority_forOrientation(
        objc2_app_kit::NSLayoutPriorityDefaultLow,
        objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
    );
    progress.addArrangedSubview(&progress_spacer);
    progress.addArrangedSubview(idle.view());
    progress.addArrangedSubview(working.view());
    progress.addArrangedSubview(half.view());
    column_append(&general, &control_row("Progress", &progress, mtm));
    let playback = playback_shortcuts(
        {
            let log = log.clone();
            move || log("toggle playback".to_string())
        },
        {
            let log = log.clone();
            move || log("step playback speed".to_string())
        },
        mtm,
    );
    column_append(&general, &control_row("Playback keys", &playback, mtm));
    let page = scrolling_page(general, mtm);
    card.connect_expansion({
        let scroll = Weak::new(&*page.scroll);
        let preserved_origin = Rc::new(Cell::new(None::<NSPoint>));
        move |expanding, complete| {
            if let Some(scroll) = scroll.load() {
                let clip = scroll.contentView();
                if !complete && !expanding {
                    preserved_origin.set(Some(clip.bounds().origin));
                }
                scroll.layoutSubtreeIfNeeded();
                if let Some(origin) = preserved_origin.get() {
                    clip.scrollToPoint(origin);
                    scroll.reflectScrolledClipView(&clip);
                }
                if complete && expanding {
                    preserved_origin.set(None);
                }
            }
        }
    });
    page.scroll.into_super()
}

struct DemoProperty {
    view: Retained<NSView>,
    reset: Rc<dyn Fn()>,
}

struct PairPropertyConfig<'a> {
    label: &'a str,
    initial: [f64; 2],
    prefixes: [&'a str; 2],
    unit: Option<&'a str>,
    digits: usize,
    modes: (bool, bool),
    minimum: Option<f64>,
    locked: bool,
}

fn pair_property(
    config: PairPropertyConfig<'_>,
    log: Rc<dyn Fn(String)>,
    mtm: MainThreadMarker,
) -> DemoProperty {
    let PairPropertyConfig {
        label,
        initial,
        prefixes,
        unit,
        digits,
        modes,
        minimum,
        locked,
    } = config;
    let controller = LayeredPropertyController::default();
    let graph = FrameGraph::with_components(
        shrimply_components_demo_core::property_graph_components(&initial, 0),
        {
            let label = label.to_string();
            move |_| log(format!("{label} keyframe action"))
        },
        mtm,
    );
    let values = Rc::new(Cell::new(initial));
    let mut builder = Number2Picker::builder(initial[0], initial[1])
        .digits(digits)
        .first_prefix(prefixes[0])
        .second_prefix(prefixes[1]);
    if let Some(unit) = unit {
        builder = builder.unit_name(unit);
    }
    if let Some(minimum) = minimum {
        builder = builder.minimum(minimum);
    }
    if locked {
        builder = builder.enable_lock();
    }
    let picker = builder
        .on_change({
            let controller = controller.clone();
            let graph = graph.clone();
            let values = values.clone();
            move |next, component| {
                let previous = values.replace(next);
                match controller.edit_component(next, component) {
                    LayeredEdit::Base(_) => graph.activate_component(component),
                    LayeredEdit::Keyframe(_) => {
                        graph.edit_component_values(component, &component_changes(previous, next))
                    }
                }
            }
        })
        .build_with_handles(mtm);
    let handles = [picker.first.clone(), picker.second.clone()];
    graph.connect_status({
        let graph = graph.clone();
        let controller = controller.clone();
        let handles = handles.clone();
        let values = values.clone();
        move |status| {
            if !controller.keyframes() {
                return;
            }
            let component = graph.active_component().min(1);
            let mut next = values.get();
            next[component] = status.value;
            values.set(next);
            handles[component].set_f64(status.value);
        }
    });
    let expression = ExpressionEditor::new(
        shrimply_components_demo_core::EXPRESSION_SOURCE,
        &shrimply_components_demo_core::expression_output(
            shrimply_components_demo_core::EXPRESSION_SOURCE,
        ),
        |source| shrimply_components_demo_core::expression_output(&source),
        mtm,
    );
    let property = InspectorGraphProperty::new(
        label,
        &picker.widget,
        graph.clone(),
        expression,
        controller.clone(),
        mtm,
    );
    property.set_keyframes_active(modes.0);
    property.set_expression_active(modes.1);
    let reset = Rc::new(move || {
        controller.select_component::<2>(0);
        values.set(initial);
        graph.replace_components(shrimply_components_demo_core::property_graph_components(
            &initial, 0,
        ));
        handles[0].set_f64(initial[0]);
        handles[1].set_f64(initial[1]);
    });
    DemoProperty {
        view: property.into_view().into_super(),
        reset,
    }
}

fn scalar_property(
    label: &str,
    initial: f64,
    log: Rc<dyn Fn(String)>,
    mtm: MainThreadMarker,
) -> DemoProperty {
    let controller = LayeredPropertyController::default();
    let graph = FrameGraph::with_actions(
        shrimply_components_demo_core::property_graph_state(initial),
        {
            let label = label.to_string();
            move |_| log(format!("{label} keyframe action"))
        },
        mtm,
    );
    let picker = NumberPicker::builder(initial)
        .digits(1)
        .drag_step(0.1)
        .rotating_prefix_symbol("arrow.up")
        .unit_name("°")
        .on_change({
            let controller = controller.clone();
            let graph = graph.clone();
            move |value| {
                if let LayeredEdit::Keyframe(value) = controller.edit(value) {
                    graph.edit_value(value);
                }
            }
        })
        .build_with_handle(mtm);
    let handle = picker.handle.clone();
    graph.connect_status({
        let controller = controller.clone();
        move |status| {
            if controller.keyframes() {
                handle.set_f64(status.value);
            }
        }
    });
    let expression = ExpressionEditor::new(
        shrimply_components_demo_core::EXPRESSION_SOURCE,
        &shrimply_components_demo_core::expression_output(
            shrimply_components_demo_core::EXPRESSION_SOURCE,
        ),
        |source| shrimply_components_demo_core::expression_output(&source),
        mtm,
    );
    let property = InspectorGraphProperty::new(
        label,
        &picker.widget,
        graph.clone(),
        expression,
        controller,
        mtm,
    );
    let reset_handle = picker.handle;
    let reset = Rc::new(move || {
        graph.replace_state(shrimply_components_demo_core::property_graph_state(initial));
        reset_handle.set_f64(initial);
    });
    DemoProperty {
        view: property.into_view().into_super(),
        reset,
    }
}

fn info_page(log: Rc<dyn Fn(String)>, mtm: MainThreadMarker) -> Retained<NSView> {
    let info = page_stack(mtm);
    for (label, value) in [
        ("Selected item", "Example clip · 00:00:02:00"),
        ("Component package", "shrimply-components-appkit"),
        ("Frame graph", "Shared Rust renderer"),
    ] {
        let field = ReadOnlyField::new(value, true, mtm);
        column_append(&info, &control_row(label, field.view(), mtm));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let home_text = home.display().to_string();
    let field = ReadOnlyField::with_action(
        &home_text,
        "Show in Folder",
        move || {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&home.display().to_string()));
            NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(
                &objc2_foundation::NSArray::from_retained_slice(&[url]),
            );
            log("showed home folder".to_string());
        },
        mtm,
    );
    column_append(&info, &control_row("Home folder", field.view(), mtm));
    scrolling_page(info, mtm).scroll.into_super()
}

fn page_stack(mtm: MainThreadMarker) -> Retained<NSStackView> {
    let page = column_stack(PAGE_GAP, mtm);
    page.setEdgeInsets(objc2_foundation::NSEdgeInsets {
        top: PAGE_INSET,
        left: PAGE_INSET,
        bottom: PAGE_INSET,
        right: PAGE_INSET,
    });
    page
}

struct PageDocumentIvars;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PageDocumentIvars]
    struct PageDocument;

    unsafe impl NSObjectProtocol for PageDocument {}

    impl PageDocument {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }

    }
);

struct ScrollingPage {
    scroll: Retained<NSScrollView>,
}

fn scrolling_page(content: Retained<NSStackView>, mtm: MainThreadMarker) -> ScrollingPage {
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(false);
    scroll.setAutohidesScrollers(true);
    let document = PageDocument::alloc(mtm).set_ivars(PageDocumentIvars);
    let document: Retained<PageDocument> =
        unsafe { msg_send![super(document), initWithFrame: NSRect::ZERO] };
    content.setTranslatesAutoresizingMaskIntoConstraints(false);
    content.setContentHuggingPriority_forOrientation(
        objc2_app_kit::NSLayoutPriorityRequired,
        objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
    );
    content.setContentCompressionResistancePriority_forOrientation(
        objc2_app_kit::NSLayoutPriorityRequired,
        objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
    );
    document.setTranslatesAutoresizingMaskIntoConstraints(false);
    document.addSubview(&content);
    scroll.setDocumentView(Some(&document));
    for constraint in [
        document
            .widthAnchor()
            .constraintEqualToAnchor(&scroll.contentView().widthAnchor()),
        document
            .heightAnchor()
            .constraintGreaterThanOrEqualToAnchor(&scroll.contentView().heightAnchor()),
        content
            .leadingAnchor()
            .constraintEqualToAnchor(&document.leadingAnchor()),
        content
            .trailingAnchor()
            .constraintEqualToAnchor(&document.trailingAnchor()),
        content
            .topAnchor()
            .constraintEqualToAnchor(&document.topAnchor()),
        content
            .bottomAnchor()
            .constraintLessThanOrEqualToAnchor(&document.bottomAnchor()),
    ] {
        constraint.setActive(true);
    }
    ScrollingPage { scroll }
}

fn text_scroll(view: Retained<NSTextView>, mtm: MainThreadMarker) -> Retained<NSScrollView> {
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    scroll.setDocumentView(Some(&view));
    scroll
}
