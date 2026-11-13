mod picker;

use super::control::Context;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{NSTextField, NSView};
use objc2_foundation::{MainThreadMarker, NSString};
use shrimply_components_appkit::{ActionButton, column_append_intrinsic, column_stack, row_stack};
use shrimply_inspector_core::{InspectorControl, font_selector::FamilyEdit};
use shrimply_project_document::project::FontFamily;

pub(super) fn view(
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let root = column_stack(6.0, mtm);
    let families: Vec<shrimply_project_document::project::FontFamily> =
        serde_json::from_str(&control.value).expect("font presentation is valid");
    if families.is_empty() {
        column_append_intrinsic(
            &root,
            &NSTextField::labelWithString(&NSString::from_str("System default"), mtm),
        );
    }
    for (index, family) in families.iter().enumerate() {
        let row = row_stack(4.0, mtm);
        let title = match family {
            FontFamily::GoogleFonts { .. } => format!("{} · Google", family.name()),
            FontFamily::Local { .. } => family.name().to_string(),
        };
        let button = picker_button(
            &title,
            Some((index, family.clone())),
            &root,
            control,
            context,
            mtm,
        );
        row.addArrangedSubview(button.view());
        row.addArrangedSubview(&NSView::new(mtm));
        for (symbol, label, enabled, edit) in [
            (
                "chevron.up",
                "Move font up",
                index > 0,
                FamilyEdit::Move { index, offset: -1 },
            ),
            (
                "chevron.down",
                "Move font down",
                index + 1 < families.len(),
                FamilyEdit::Move { index, offset: 1 },
            ),
            ("trash", "Remove font", true, FamilyEdit::Remove(index)),
        ] {
            let context = context.clone();
            let control = control.clone();
            let button = ActionButton::symbol(
                symbol,
                label,
                move || {
                    context.end_editing();
                    context.refresh(context.controller.edit_control_font_family(
                        &context.target,
                        &control,
                        edit.clone(),
                    ))
                },
                mtm,
            );
            button.view().setEnabled(enabled);
            row.addArrangedSubview(button.view());
        }
        column_append_intrinsic(&root, &row);
    }
    let add = picker_button("Add font…", None, &root, control, context, mtm);
    column_append_intrinsic(&root, add.view());
    root.into_super()
}

fn picker_button(
    title: &str,
    selected: Option<(usize, FontFamily)>,
    root: &NSView,
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) -> ActionButton {
    let root = Weak::new(root);
    let context = context.clone();
    let control = control.clone();
    ActionButton::new(
        title,
        move || {
            context.end_editing();
            if let Some(window) = root.load().and_then(|root| root.window()) {
                picker::show(&window, selected.clone(), &control, &context, mtm);
            }
        },
        mtm,
    )
}
