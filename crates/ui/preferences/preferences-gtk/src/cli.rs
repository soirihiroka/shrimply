use crate::store;
use adw::prelude::*;
use shrimply_components_gtk::tr;

pub fn row() -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(tr!("CLI").as_ref())
        .subtitle("shrimply")
        .use_markup(false)
        .build();
    let button = gtk::Button::builder()
        .label(tr!("Install").as_ref())
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));

    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(store::cli::status());
    });
    let status_row = row.clone();
    let status_button = button.clone();
    gtk::glib::spawn_future_local(async move {
        match receiver.recv().await.expect("CLI status worker stopped") {
            Ok(path) => {
                if let Some(path) = path {
                    status_button.set_label(tr!("Overwrite").as_ref());
                    status_row.set_subtitle(&path);
                }
                status_button.set_sensitive(true);
            }
            Err(error) => status_row.set_subtitle(&error),
        }
    });

    let install_row = row.downgrade();
    button.connect_clicked(move |button| {
        let Some(row) = install_row.upgrade() else {
            return;
        };
        button.set_sensitive(false);
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(store::cli::install());
        });
        let button = button.clone();
        gtk::glib::spawn_future_local(async move {
            match receiver
                .recv()
                .await
                .expect("CLI installation worker stopped")
            {
                Ok(message) => {
                    row.set_subtitle(&message);
                    button.set_label(tr!("Overwrite").as_ref());
                }
                Err(error) => {
                    let alert = adw::AlertDialog::new(
                        Some(tr!("Could not install CLI").as_ref()),
                        Some(&error),
                    );
                    alert.add_response("close", tr!("Close").as_ref());
                    alert.present(Some(&row));
                }
            }
            button.set_sensitive(true);
        });
    });
    row
}
