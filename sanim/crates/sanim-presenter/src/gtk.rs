use crate::{Player, WINDOW_SIZE};
use gtk::{gdk, gio, glib, prelude::*};
use sanim_render::Frame;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Instant,
};

pub fn run(title: &str, player: Player) -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())?;
    let app = gtk::Application::new(
        Some("dev.shrimply.SanimPresenter"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    let stage = Rc::new(Stage {
        player: RefCell::new(player),
        ticking: Cell::new(false),
        failure: RefCell::new(None),
    });
    let title = title.to_owned();
    app.connect_activate({
        let stage = stage.clone();
        move |app| {
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.add_css_class("sanim-presentation");
            let css = gtk::CssProvider::new();
            css.load_from_string(".sanim-presentation { background-color: black; }");
            gtk::style_context_add_provider_for_display(
                &WidgetExt::display(&picture),
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let window = gtk::ApplicationWindow::builder()
                .application(app)
                .title(&title)
                .default_width(WINDOW_SIZE.0)
                .default_height(WINDOW_SIZE.1)
                .child(&picture)
                .build();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_released({
                let stage = stage.clone();
                let picture = picture.clone();
                move |controller, key, _, modifiers| {
                    if modifiers.intersects(
                        gdk::ModifierType::CONTROL_MASK
                            | gdk::ModifierType::ALT_MASK
                            | gdk::ModifierType::SUPER_MASK,
                    ) {
                        return;
                    }
                    let now = Instant::now();
                    match key {
                        gdk::Key::Right | gdk::Key::Page_Down | gdk::Key::space => {
                            stage.player.borrow_mut().next(now);
                        }
                        gdk::Key::Left | gdk::Key::Page_Up => stage.player.borrow_mut().previous(),
                        gdk::Key::F11 | gdk::Key::Escape => {
                            let window = controller
                                .widget()
                                .and_downcast::<gtk::ApplicationWindow>()
                                .expect("presenter keyboard controller is attached to its window");
                            if window.is_fullscreen() {
                                window.unfullscreen();
                            } else if key == gdk::Key::F11 {
                                window.fullscreen();
                            }
                            return;
                        }
                        _ => return,
                    }
                    stage.refresh(&picture, now);
                }
            });
            window.add_controller(keys);
            window.present();
            let now = Instant::now();
            stage.player.borrow_mut().next(now);
            stage.refresh(&picture, now);
        }
    });
    let exit = app.run_with_args(&["sanim-presenter"]);
    if let Some(error) = stage.failure.borrow_mut().take() {
        return Err(error);
    }
    if exit != glib::ExitCode::SUCCESS {
        return Err(format!("GTK exited with status {}", exit.get()));
    }
    Ok(())
}

struct PixelBytes(Arc<Frame>);

impl AsRef<[u8]> for PixelBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0.rgba
    }
}

struct Stage {
    player: RefCell<Player>,
    ticking: Cell<bool>,
    failure: RefCell<Option<String>>,
}

impl Stage {
    fn refresh(self: &Rc<Self>, picture: &gtk::Picture, now: Instant) {
        if self.draw(picture, now) && !self.ticking.replace(true) {
            let stage = self.clone();
            picture.add_tick_callback(move |picture, _| {
                if stage.draw(picture, Instant::now()) {
                    glib::ControlFlow::Continue
                } else {
                    stage.ticking.set(false);
                    glib::ControlFlow::Break
                }
            });
        }
    }

    fn draw(&self, picture: &gtk::Picture, now: Instant) -> bool {
        let mut player = self.player.borrow_mut();
        match player.frame(now) {
            Ok(Some(frame)) => {
                let (width, height) = (frame.width, frame.height);
                let texture = gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gdk::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(PixelBytes(frame)),
                    width as usize * 4,
                );
                picture.set_paintable(Some(&texture));
            }
            Ok(None) => {}
            Err(error) => {
                self.failure.replace(Some(error));
                if let Some(window) = picture.root().and_downcast::<gtk::ApplicationWindow>() {
                    window.close();
                }
                return false;
            }
        }
        player.playing()
    }
}
