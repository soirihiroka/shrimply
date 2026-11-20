use crate::{Player, WINDOW_SIZE};
use block2::RcBlock;
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSF11FunctionKey as F11, NSImage,
    NSImageScaling, NSImageView, NSLeftArrowFunctionKey as LEFT,
    NSPageDownFunctionKey as PAGE_DOWN, NSPageUpFunctionKey as PAGE_UP,
    NSRightArrowFunctionKey as RIGHT, NSWindow, NSWindowCollectionBehavior, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_core_foundation::CFData;
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    kCGColorSpaceSRGB,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};
use sanim_render::Frame;
use std::{
    cell::RefCell,
    ptr::{self, NonNull},
    rc::Rc,
    time::{Duration, Instant},
};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}
    unsafe impl NSWindowDelegate for WindowDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_closed(&self, _notification: &NSNotification) {
            let app = NSApplication::sharedApplication(self.mtm());
            app.stop(None);
            // Closing from a render timer also needs to wake AppKit's event loop.
            if let Some(event) =
                NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    NSEventType::ApplicationDefined,
                    NSPoint::ZERO,
                    NSEventModifierFlags::empty(),
                    0.0,
                    0,
                    None,
                    0,
                    0,
                    0,
                )
            {
                app.postEvent_atStart(&event, true);
            }
        }
    }
);

pub fn run(title: &str, player: Player) -> Result<(), String> {
    let mtm = MainThreadMarker::new().expect("Sanim Presenter requires the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let bounds = NSRect::new(
        NSPoint::ZERO,
        NSSize::new(f64::from(WINDOW_SIZE.0), f64::from(WINDOW_SIZE.1)),
    );
    let window = unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            bounds,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        );
        // Retained owns the window across closing and callback cleanup.
        window.setReleasedWhenClosed(false);
        window
    };
    window.setTitle(&NSString::from_str(title));
    window.setBackgroundColor(Some(&NSColor::blackColor()));
    window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);

    let image_view = NSImageView::initWithFrame(NSImageView::alloc(mtm), bounds);
    image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    window.setContentView(Some(&image_view));

    let delegate = WindowDelegate::alloc(mtm).set_ivars(());
    let delegate: Retained<WindowDelegate> = unsafe { msg_send![super(delegate), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let stage = Rc::new(Stage {
        interval: player.frame_period()?,
        player: RefCell::new(player),
        window,
        image_view,
        timer: RefCell::new(None),
        failure: RefCell::new(None),
    });
    let keys = RcBlock::new({
        let stage = stage.clone();
        move |event: NonNull<NSEvent>| {
            if stage.key(unsafe { event.as_ref() }) {
                ptr::null_mut()
            } else {
                event.as_ptr()
            }
        }
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::KeyUp,
            &keys,
        )
    }
    .ok_or("could not install Sanim Presenter keyboard controls")?;

    stage.window.center();
    stage.window.makeKeyAndOrderFront(None);
    app.activate();
    let now = Instant::now();
    stage.player.borrow_mut().next(now);
    if stage.refresh(now) {
        app.run();
    }
    unsafe { NSEvent::removeMonitor(&monitor) };
    if let Some(timer) = stage.timer.borrow_mut().take() {
        timer.invalidate();
    }
    stage.failure.borrow_mut().take().map_or(Ok(()), Err)
}

struct Stage {
    player: RefCell<Player>,
    window: Retained<NSWindow>,
    image_view: Retained<NSImageView>,
    interval: Duration,
    timer: RefCell<Option<Retained<NSTimer>>>,
    failure: RefCell<Option<String>>,
}

impl Stage {
    fn key(self: &Rc<Self>, event: &NSEvent) -> bool {
        if !self.window.isKeyWindow()
            || event.modifierFlags().intersects(
                NSEventModifierFlags::Command
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Option,
            )
        {
            return false;
        }
        let Some(key) = event
            .charactersIgnoringModifiers()
            .and_then(|keys| keys.to_string().chars().next())
            .map(u32::from)
        else {
            return false;
        };
        const SPACE: u32 = ' ' as u32;
        const ESCAPE: u32 = '\u{1b}' as u32;
        let released = event.r#type() == NSEventType::KeyUp;
        let now = Instant::now();
        match key {
            RIGHT | PAGE_DOWN | SPACE => {
                if released {
                    self.player.borrow_mut().next(now);
                    self.refresh(now);
                }
            }
            LEFT | PAGE_UP => {
                if released {
                    self.player.borrow_mut().previous();
                    self.refresh(now);
                }
            }
            F11 | ESCAPE => {
                if released
                    && (key == F11
                        || self
                            .window
                            .styleMask()
                            .contains(NSWindowStyleMask::FullScreen))
                {
                    self.window.toggleFullScreen(None);
                }
            }
            _ => return false,
        }
        true
    }

    fn refresh(self: &Rc<Self>, now: Instant) -> bool {
        match self.draw(now) {
            Ok(true) if self.timer.borrow().is_none() => {
                let stage = Rc::downgrade(self);
                let tick = RcBlock::new(move |timer: NonNull<NSTimer>| {
                    let timer = unsafe { timer.as_ref() };
                    if let Some(stage) = stage.upgrade() {
                        match stage.draw(Instant::now()) {
                            Ok(true) => return,
                            Ok(false) => {}
                            Err(error) => stage.fail(error),
                        }
                        stage.timer.borrow_mut().take();
                    }
                    timer.invalidate();
                });
                let timer = unsafe {
                    NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                        self.interval.as_secs_f64(),
                        true,
                        &tick,
                    )
                };
                self.timer.replace(Some(timer));
            }
            Ok(_) => {}
            Err(error) => {
                self.fail(error);
                return false;
            }
        }
        true
    }

    fn draw(&self, now: Instant) -> Result<bool, String> {
        let mut player = self.player.borrow_mut();
        if let Some(frame) = player.frame(now)? {
            let image = native_image(&frame)?;
            self.image_view.setImage(Some(&image));
        }
        Ok(player.playing())
    }

    fn fail(&self, error: String) {
        self.failure.replace(Some(error));
        self.window.close();
    }
}

fn native_image(frame: &Frame) -> Result<Retained<NSImage>, String> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let data = CFData::from_bytes(&frame.rgba);
    let provider =
        CGDataProvider::with_cf_data(Some(&data)).ok_or("cannot read Sanim frame pixels")?;
    let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or("cannot create the sRGB color space")?;
    let rgba8 = CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0);
    let image = unsafe {
        CGImage::new(
            width,
            height,
            u8::BITS as usize,
            u32::BITS as usize,
            width * 4,
            Some(&color_space),
            rgba8,
            Some(&provider),
            ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or("cannot create the Sanim frame image")?;
    Ok(NSImage::initWithCGImage_size(
        NSImage::alloc(),
        &image,
        NSSize::new(f64::from(frame.width), f64::from(frame.height)),
    ))
}
