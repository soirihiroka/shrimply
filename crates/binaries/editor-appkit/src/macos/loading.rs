use super::*;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSEvent, NSEventModifierFlags, NSEventType, NSFont,
    NSGraphicsContext, NSImage, NSImageInterpolation, NSImageScaling, NSImageView,
    NSLayoutConstraint, NSStackView, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSData, NSSize};
use shrimply_project_document::project::{PreparedProject, ProjectLoadError};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const SHRIMP_SIZE: NSSize = NSSize::new(160.0, 180.0);
const CONTENT_SPACING: f64 = 16.0;
const SUBTITLE_WIDTH: f64 = 480.0;

// AppKit handles GIF playback; disable interpolation to preserve the pixel art.
define_class!(
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    struct LoadingImage;

    unsafe impl NSObjectProtocol for LoadingImage {}

    impl LoadingImage {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, rect: NSRect) {
            let context = NSGraphicsContext::currentContext().expect("loading drawing context");
            context.setImageInterpolation(NSImageInterpolation::None);
            unsafe { let _: () = msg_send![super(self), drawRect: rect]; }
        }

    }
);

pub(super) struct View {
    pub root: Retained<NSView>,
    image: Retained<LoadingImage>,
    subtitle: Retained<NSTextField>,
}

impl View {
    pub fn new(path: &Path, mtm: MainThreadMarker) -> Self {
        let root = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, layout::WINDOW_SIZE),
        );
        root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        root.setWantsLayer(true);
        root.layer()
            .expect("loading background layer")
            .setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
        let image: Retained<LoadingImage> = unsafe {
            msg_send![LoadingImage::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, SHRIMP_SIZE)]
        };
        let gif = NSImage::initWithData(
            NSImage::alloc(),
            &NSData::with_bytes(include_bytes!("../../../../../assets/loading-shrimp.gif")),
        )
        .expect("bundled loading animation must decode");
        image.setImage(Some(&gif));
        image.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        image.setAnimates(true);
        image.setTranslatesAutoresizingMaskIntoConstraints(false);
        image
            .widthAnchor()
            .constraintEqualToConstant(SHRIMP_SIZE.width)
            .setActive(true);
        image
            .heightAnchor()
            .constraintEqualToConstant(SHRIMP_SIZE.height)
            .setActive(true);
        let heading = NSTextField::labelWithString(ns_string!("Loading project…"), mtm);
        heading.setFont(Some(
            &NSFont::boldSystemFontOfSize(NSFont::systemFontSize()),
        ));
        let filename = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let subtitle = NSTextField::labelWithString(&NSString::from_str(&filename), mtm);
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        subtitle.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
        subtitle
            .widthAnchor()
            .constraintLessThanOrEqualToConstant(SUBTITLE_WIDTH)
            .setActive(true);
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setSpacing(CONTENT_SPACING);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterX);
        stack.setTranslatesAutoresizingMaskIntoConstraints(false);
        stack.addArrangedSubview(&image);
        stack.addArrangedSubview(&heading);
        stack.addArrangedSubview(&subtitle);
        root.addSubview(&stack);
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
            stack
                .centerXAnchor()
                .constraintEqualToAnchor(&root.centerXAnchor()),
            stack
                .centerYAnchor()
                .constraintEqualToAnchor(&root.centerYAnchor()),
        ]));
        Self {
            root,
            image,
            subtitle,
        }
    }

    pub fn set_status(&self, status: &str) {
        self.subtitle.setStringValue(&NSString::from_str(status));
    }
}

pub(super) type Preparation = Receiver<Result<PreparedProject, ProjectLoadError>>;

impl Editor {
    pub(super) fn begin_project_load(&self) {
        let path = self.ivars().project_path.clone();
        let (sender, receiver) = mpsc::channel();
        self.ivars().preparation.replace(Some(receiver));
        std::thread::Builder::new()
            .name("project-load".into())
            .spawn(move || {
                let _ = sender.send(shrimply_project_document::project::prepare_project(&path));
            })
            .expect("start project loading worker");
    }

    pub(super) fn poll_project_load(&self) {
        let result = {
            let mut pending = self.ivars().preparation.borrow_mut();
            let Some(receiver) = pending.as_ref() else {
                return;
            };
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(ProjectLoadError::Other(
                    "Project loading worker stopped unexpectedly".into(),
                )),
            };
            pending.take();
            result
        };
        match result {
            Ok(prepared) => {
                let session = match EditorSession::new(
                    shrimply_project_document::project::activate_project(prepared),
                ) {
                    Ok(session) => Rc::new(session),
                    Err(error) => {
                        self.fail_startup(&error);
                        return;
                    }
                };
                let window = self.ivars().window.get().expect("loading window installed");
                window.setTitle(&NSString::from_str(&session.title().text));
                self.ivars()
                    .session
                    .set(session)
                    .unwrap_or_else(|_| panic!("session already installed"));
                let layout = layout::build(self);
                // Keep editor controls out of the window until its first frame is ready.
                self.ivars()
                    .layout
                    .set(layout)
                    .unwrap_or_else(|_| panic!("layout already installed"));
            }
            Err(ProjectLoadError::LockedByOtherInstance { pid }) => {
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(ns_string!("Project is in use"));
                alert.setInformativeText(&NSString::from_str(&format!(
                    "The project lock is held by another editor process (PID {pid})."
                )));
                alert.addButtonWithTitle(ns_string!("Retry"));
                alert
                    .addButtonWithTitle(ns_string!("Stop Other Editor"))
                    .setHasDestructiveAction(true);
                alert
                    .addButtonWithTitle(ns_string!("Close"))
                    .setKeyEquivalent(&NSString::from_str("\u{1b}"));
                match alert.runModal() {
                    response if response == NSAlertFirstButtonReturn => self.begin_project_load(),
                    response if response == NSAlertSecondButtonReturn => {
                        if shrimply_project_document::project::terminate_project_process(pid) {
                            self.begin_project_load();
                        } else {
                            self.fail_startup("Could not stop other editor: Shrimply could not signal the other process.");
                        }
                    }
                    response if response == NSAlertThirdButtonReturn => {
                        self.stop_loading(Ok(false))
                    }
                    _ => panic!("unexpected project-lock alert response"),
                }
            }
            Err(ProjectLoadError::Other(error)) => {
                self.fail_startup(&format!("Could not open project: {error}"))
            }
        }
    }

    pub(super) fn poll_preview_startup(&self) {
        use shrimply_preview_render_metal::StartupStatus;
        let mut status = StartupStatus::Ready;
        for canvas in &self
            .ivars()
            .layout
            .get()
            .expect("layout installed")
            .canvases
        {
            match canvas.poll_startup() {
                Err(error) => {
                    self.fail_startup(&error);
                    return;
                }
                Ok(StartupStatus::CompilingShaders) => {
                    status = StartupStatus::CompilingShaders;
                    break;
                }
                Ok(StartupStatus::PreparingPreview) => status = StartupStatus::PreparingPreview,
                Ok(StartupStatus::Ready) => {}
            }
        }
        if status != StartupStatus::Ready {
            self.ivars()
                .loading
                .borrow()
                .as_ref()
                .expect("loading view installed")
                .set_status(match status {
                    StartupStatus::CompilingShaders => "Compiling Metal shaders…",
                    StartupStatus::PreparingPreview => "Preparing preview…",
                    StartupStatus::Ready => unreachable!(),
                });
            return;
        }
        let loading = self
            .ivars()
            .loading
            .borrow_mut()
            .take()
            .expect("loading view installed");
        loading.image.setAnimates(false);
        let window = self.ivars().window.get().expect("window installed");
        let layout = self.ivars().layout.get().expect("layout installed");
        layout.root.view().setFrame(loading.root.bounds());
        window.setContentViewController(Some(&layout.root));
        window.makeFirstResponder(None);
        let toolbar = window.toolbar().expect("toolbar installed");
        for (index, identifier) in menus::toolbar_identifiers().iter().enumerate() {
            toolbar.insertItemWithItemIdentifier_atIndex(&identifier, index as isize);
        }
        menus::install(self);
        self.sync_panels();
        self.install_fullscreen_events();
        self.ivars().outcome.set(Ok(true));
    }

    pub(super) fn fail_startup(&self, error: &str) {
        if let Some(display_link) = self.ivars().display_link.get() {
            display_link.invalidate();
        }
        self.show_error(error);
        self.stop_loading(Err(()));
    }

    pub(super) fn stop_loading(&self, outcome: Result<bool, ()>) {
        self.ivars().outcome.set(outcome);
        if let Some(display_link) = self.ivars().display_link.get() {
            display_link.invalidate();
        }
        let app = NSApplication::sharedApplication(self.mtm());
        app.stop(None);
        // A display callback can stop the run loop while it is waiting for its next event.
        let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined, NSPoint::ZERO, NSEventModifierFlags::empty(),
            0.0, 0, None, 0, 0, 0,
        ).expect("application wake event");
        app.postEvent_atStart(&event, true);
    }
}
