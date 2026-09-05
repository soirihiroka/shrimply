use objc2::AnyThread;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName,
    NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits, NSApplication,
    NSLinkAttributeName,
};
use objc2_foundation::{
    MainThreadMarker, NSDictionary, NSMutableAttributedString, NSString, NSURL,
};
use shrimply_component_core::about;

pub fn show(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let text = NSString::from_str(&format!(
        "{}\n\n{}\n\n{}\n{}\n\nWebsite\nReport an Issue\nGNU General Public License v3.0",
        about::DESCRIPTION,
        about::DEVELOPER,
        about::CREDIT_HEADING,
        about::CREDITS.join(" · "),
    ));
    let credits =
        NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
    for (label, url) in [
        ("Website", about::WEBSITE),
        ("Report an Issue", about::ISSUE_URL),
        (
            "GNU General Public License v3.0",
            "https://www.gnu.org/licenses/gpl-3.0.html",
        ),
    ] {
        let url = NSURL::URLWithString(&NSString::from_str(url)).expect("About link must be valid");
        unsafe {
            credits.addAttribute_value_range(
                NSLinkAttributeName,
                &url,
                text.rangeOfString(&NSString::from_str(label)),
            );
        }
    }
    let name = NSString::from_str(about::NAME);
    let version = NSString::from_str(env!("CARGO_PKG_VERSION"));
    let icon = app
        .applicationIconImage()
        .expect("application icon must be installed");
    let options = unsafe {
        NSDictionary::from_slices(
            &[
                NSAboutPanelOptionApplicationName,
                NSAboutPanelOptionApplicationVersion,
                NSAboutPanelOptionApplicationIcon,
                NSAboutPanelOptionCredits,
            ],
            &[
                &*name as &AnyObject,
                &*version as &AnyObject,
                &*icon as &AnyObject,
                &*credits as &AnyObject,
            ],
        )
    };
    unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
}
