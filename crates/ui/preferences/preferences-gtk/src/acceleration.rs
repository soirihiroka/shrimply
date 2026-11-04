use crate::store;
use adw::prelude::*;
use shrimply_components_gtk::tr;

pub fn row(preferences: store::SharedPreferences) -> adw::ComboRow {
    let saved = store::snapshot(&preferences).acceleration_device;
    let mut choices = vec![(String::new(), tr!("Automatic").to_string())];
    let enumeration = shrimply_visual_cuda::gpu::acceleration_devices();
    let error = enumeration.as_ref().err().cloned();
    if let Ok(devices) = enumeration {
        choices.extend(devices.into_iter().map(|device| (device.id, device.name)));
    }
    if !saved.is_empty() && !choices.iter().any(|(id, _)| id == &saved) {
        choices.push((saved.clone(), format!("{} ({saved})", tr!("Unavailable"))));
    }
    let labels = choices
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>();
    let row = adw::ComboRow::builder()
        .title(tr!("Acceleration device").as_ref())
        .subtitle(tr!("Rendering, playback and export. Restart required.").as_ref())
        .model(&gtk::StringList::new(&labels))
        .use_markup(false)
        .selected(
            choices
                .iter()
                .position(|(id, _)| id == &saved)
                .expect("saved GPU choice") as u32,
        )
        .build();
    if let Some(error) = error {
        row.set_tooltip_text(Some(&error));
    }
    row.connect_selected_notify(move |row| {
        let Some((id, _)) = choices.get(row.selected() as usize) else {
            return;
        };
        if let Err(error) = store::set_acceleration_device(&preferences, id) {
            let saved = store::snapshot(&preferences).acceleration_device;
            row.set_selected(
                choices
                    .iter()
                    .position(|(id, _)| id == &saved)
                    .expect("previous GPU choice") as u32,
            );
            let dialog = adw::AlertDialog::new(
                Some(tr!("Could not save acceleration device").as_ref()),
                Some(&error),
            );
            dialog.add_response("close", tr!("Close").as_ref());
            dialog.present(Some(row));
        }
    });
    row
}
