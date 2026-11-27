use crate::tr;
use adw::prelude::*;

pub fn show(window: &impl IsA<gtk::Widget>, version: &str) {
    use shrimply_components_core::about;
    let dialog = adw::AboutDialog::builder()
        .application_name(about::NAME)
        .application_icon(about::ICON_NAME)
        .version(version)
        .comments(about::DESCRIPTION)
        .developer_name(about::DEVELOPER)
        .license_type(gtk::License::Gpl30)
        .title(tr!("About Shrimply").as_ref())
        .build();
    for &(heading, entries) in about::CREDIT_SECTIONS {
        dialog.add_credit_section(Some(tr!(heading).as_ref()), entries);
    }
    for &link in about::LINKS {
        match link {
            about::WEBSITE => dialog.set_website(about::WEBSITE.1),
            about::REPORT_ISSUE => dialog.set_issue_url(about::REPORT_ISSUE.1),
            (label, url) => dialog.add_link(tr!(label).as_ref(), url),
        }
    }
    dialog.present(Some(window));
}
