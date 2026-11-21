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
use shrimply_components_core::about;

pub fn show(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let credit_sections = about::CREDIT_SECTIONS
        .iter()
        .map(|(heading, entries)| {
            format!("{}\n{}", shrimply_i18n::text(heading), entries.join(" · "))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let links: Vec<_> = about::LINKS
        .iter()
        .copied()
        .chain(std::iter::once((
            "GNU General Public License v3.0",
            "https://www.gnu.org/licenses/gpl-3.0.html",
        )))
        .map(|(label, url)| (shrimply_i18n::text(label), url))
        .collect();
    let link_labels = links
        .iter()
        .map(|(label, _)| label.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    let text = NSString::from_str(&format!(
        "{}\n\n{credit_sections}\n\n{link_labels}",
        about::DESCRIPTION,
    ));
    let credits =
        NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
    for (label, url) in links {
        let url = NSURL::URLWithString(&NSString::from_str(url)).expect("About link must be valid");
        unsafe {
            credits.addAttribute_value_range(
                NSLinkAttributeName,
                &url,
                text.rangeOfString(&NSString::from_str(&label)),
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
