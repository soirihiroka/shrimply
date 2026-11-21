use objc2::{AnyThread, ClassType, rc::Retained};
use objc2_app_kit::{
    NSFont, NSImage, NSImageScaling, NSImageView, NSLayoutConstraintOrientation,
    NSLayoutPriorityDefaultLow, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSDate, NSDateFormatter, NSDateFormatterStyle, NSString};
use shrimply_components_appkit::{ReadOnlyField, Spinner, control_row, row_stack};
use shrimply_inspector_core::{ControlKind, InspectorControl};

pub(super) fn format_date(seconds: i64) -> Option<String> {
    objc2::rc::autoreleasepool(|_| {
        Some(
            NSDateFormatter::localizedStringFromDate_dateStyle_timeStyle(
                &NSDate::dateWithTimeIntervalSince1970(seconds as f64),
                NSDateFormatterStyle::MediumStyle,
                NSDateFormatterStyle::ShortStyle,
            )
            .to_string(),
        )
    })
}

pub(super) fn view(control: &InspectorControl, mtm: MainThreadMarker) -> Retained<NSView> {
    match control.kind {
        ControlKind::InfoHeading => {
            let label = NSTextField::labelWithString(&NSString::from_str(&control.label), mtm);
            label.setFont(Some(
                &NSFont::boldSystemFontOfSize(NSFont::systemFontSize()),
            ));
            label.setUsesSingleLineMode(true);
            label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
            label.setToolTip(Some(&NSString::from_str(&control.label)));
            label.setContentCompressionResistancePriority_forOrientation(
                NSLayoutPriorityDefaultLow,
                NSLayoutConstraintOrientation::Horizontal,
            );
            label.as_super().as_super().into()
        }
        ControlKind::InfoLoading => {
            let row = row_stack(6.0, mtm);
            let spinner = Spinner::new(mtm);
            spinner.set_active(true);
            row.addArrangedSubview(spinner.view());
            row.addArrangedSubview(&NSTextField::labelWithString(
                &NSString::from_str(&control.value),
                mtm,
            ));
            row.addArrangedSubview(&NSView::new(mtm));
            control_row(&control.label, &row, mtm)
        }
        ControlKind::InfoArtwork => {
            let artwork = control
                .artwork
                .as_ref()
                .expect("artwork requires a thumbnail");
            let path = NSString::from_str(&artwork.path().to_string_lossy());
            let Some(image) = NSImage::initWithContentsOfFile(NSImage::alloc(), &path)
                .filter(|image| image.size().width > 0.0 && image.size().height > 0.0)
            else {
                return ReadOnlyField::new("Could not display the artwork thumbnail", false, mtm)
                    .view()
                    .as_super()
                    .into();
            };
            let view = NSImageView::new(mtm);
            view.setImage(Some(&image));
            view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
            view.setToolTip(Some(&NSString::from_str(&control.label)));
            view.setContentCompressionResistancePriority_forOrientation(
                NSLayoutPriorityDefaultLow,
                NSLayoutConstraintOrientation::Horizontal,
            );
            view.heightAnchor()
                .constraintLessThanOrEqualToConstant(image.size().height)
                .setActive(true);
            view.setContentCompressionResistancePriority_forOrientation(
                NSLayoutPriorityDefaultLow,
                NSLayoutConstraintOrientation::Vertical,
            );
            let aspect = view.heightAnchor().constraintEqualToAnchor_multiplier(
                &view.widthAnchor(),
                image.size().height / image.size().width,
            );
            aspect.setPriority(objc2_app_kit::NSLayoutPriorityDefaultHigh);
            aspect.setActive(true);
            view.as_super().as_super().into()
        }
        _ => unreachable!("media-info renderer requires a media-info control"),
    }
}
