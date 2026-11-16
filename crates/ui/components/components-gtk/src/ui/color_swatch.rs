use std::cell::{Cell, OnceCell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use shrimply_components_core::color::picker;
use shrimply_math_color::Color;

const WIDTH: i32 = 48;
const HEIGHT: i32 = 32;
const CHECKER_SIZE: f32 = picker::CHECKER_SIZE as f32;
const CHECKER_LIGHT: f32 = picker::CHECKER_LIGHT as f32;
const CHECKER_DARK: f32 = picker::CHECKER_DARK as f32;

#[derive(Clone, Copy)]
pub(super) enum SwatchShape {
    Rounded,
    PaletteTop,
    PaletteMiddle,
    PaletteBottom,
}

mod imp {
    use super::*;

    pub struct ColorSwatch {
        pub color: Cell<Color<u8>>,
        pub overlay: OnceCell<gtk::Image>,
        pub activate: RefCell<Option<Box<dyn Fn()>>>,
    }

    impl Default for ColorSwatch {
        fn default() -> Self {
            Self {
                color: Cell::new(Color::BLACK),
                overlay: OnceCell::new(),
                activate: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ColorSwatch {
        const NAME: &'static str = "ShrimplyColorSwatch";
        type Type = super::ColorSwatch;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("colorswatch");
            class.set_accessible_role(gtk::AccessibleRole::Radio);
        }
    }

    impl ObjectImpl for ColorSwatch {
        fn constructed(&self) {
            self.parent_constructed();
            let swatch = self.obj();
            swatch.set_focusable(true);
            swatch.set_overflow(gtk::Overflow::Hidden);
            swatch.add_css_class("activatable");

            let overlay = glib::Object::builder::<gtk::Image>()
                .property("accessible-role", gtk::AccessibleRole::None)
                .property("css-name", "overlay")
                .build();
            overlay.set_parent(&*swatch);
            self.overlay.set(overlay).expect("overlay initialized once");

            let click = gtk::GestureClick::new();
            click.set_button(0);
            click.connect_pressed(glib::clone!(
                #[weak]
                swatch,
                move |gesture, presses, _, _| {
                    if gesture.current_button() == gtk::gdk::BUTTON_PRIMARY && presses == 1 {
                        swatch.activate();
                    }
                }
            ));
            swatch.add_controller(click);

            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                swatch,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| {
                    if matches!(
                        key,
                        gtk::gdk::Key::space
                            | gtk::gdk::Key::Return
                            | gtk::gdk::Key::ISO_Enter
                            | gtk::gdk::Key::KP_Enter
                            | gtk::gdk::Key::KP_Space
                    ) {
                        swatch.activate();
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
            swatch.add_controller(keys);
        }

        fn dispose(&self) {
            if let Some(overlay) = self.overlay.get()
                && overlay.parent().is_some()
            {
                overlay.unparent();
            }
        }
    }

    impl WidgetImpl for ColorSwatch {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (minimum, natural, minimum_baseline, natural_baseline) = self
                .overlay
                .get()
                .expect("overlay initialized")
                .measure(orientation, for_size);
            let size = match orientation {
                gtk::Orientation::Horizontal => WIDTH,
                gtk::Orientation::Vertical => HEIGHT,
                _ => unreachable!("unknown GTK orientation"),
            };
            (
                minimum.max(size),
                natural.max(size),
                minimum_baseline,
                natural_baseline,
            )
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            self.overlay
                .get()
                .expect("overlay initialized")
                .size_allocate(&gtk::Allocation::new(0, 0, width, height), -1);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let swatch = self.obj();
            let width = swatch.width();
            let height = swatch.height();
            let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            let color = self.color.get();
            let rgba: gtk::gdk::RGBA = color.into();

            if color.a < u8::MAX {
                snapshot.push_repeat(&bounds, None);
                snapshot.append_color(
                    &gtk::gdk::RGBA::new(CHECKER_LIGHT, CHECKER_LIGHT, CHECKER_LIGHT, 1.0),
                    &gtk::graphene::Rect::new(0.0, 0.0, CHECKER_SIZE, CHECKER_SIZE),
                );
                snapshot.append_color(
                    &gtk::gdk::RGBA::new(CHECKER_DARK, CHECKER_DARK, CHECKER_DARK, 1.0),
                    &gtk::graphene::Rect::new(CHECKER_SIZE, 0.0, CHECKER_SIZE, CHECKER_SIZE),
                );
                snapshot.append_color(
                    &gtk::gdk::RGBA::new(CHECKER_DARK, CHECKER_DARK, CHECKER_DARK, 1.0),
                    &gtk::graphene::Rect::new(0.0, CHECKER_SIZE, CHECKER_SIZE, CHECKER_SIZE),
                );
                snapshot.append_color(
                    &gtk::gdk::RGBA::new(CHECKER_LIGHT, CHECKER_LIGHT, CHECKER_LIGHT, 1.0),
                    &gtk::graphene::Rect::new(
                        CHECKER_SIZE,
                        CHECKER_SIZE,
                        CHECKER_SIZE,
                        CHECKER_SIZE,
                    ),
                );
                snapshot.pop();
                snapshot.append_color(&rgba, &bounds);
            } else {
                let opaque = gtk::gdk::RGBA::new(rgba.red(), rgba.green(), rgba.blue(), 1.0);
                snapshot.append_color(&opaque, &bounds);
            }

            swatch.snapshot_child(self.overlay.get().expect("overlay initialized"), snapshot);
        }
    }
}

glib::wrapper! {
    pub struct ColorSwatch(ObjectSubclass<imp::ColorSwatch>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ColorSwatch {
    pub(super) fn new(
        color: Color<u8>,
        shape: SwatchShape,
        selected: bool,
        activate: impl Fn() + 'static,
    ) -> Self {
        let swatch: Self = glib::Object::new();
        swatch.imp().color.set(color);
        *swatch.imp().activate.borrow_mut() = Some(Box::new(activate));
        match shape {
            SwatchShape::Rounded => {
                swatch.add_css_class("left");
                swatch.add_css_class("right");
            }
            SwatchShape::PaletteTop => swatch.add_css_class("top"),
            SwatchShape::PaletteMiddle => {}
            SwatchShape::PaletteBottom => swatch.add_css_class("bottom"),
        }
        if color.r as f32 * 0.30 + color.g as f32 * 0.59 + color.b as f32 * 0.11 > 127.5 {
            swatch.add_css_class("light");
        } else {
            swatch.add_css_class("dark");
        }
        swatch.set_tooltip_text(Some(&format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            color.r, color.g, color.b, color.a
        )));
        swatch.set_cursor_from_name(Some("pointer"));
        swatch.set_selected(selected);
        swatch
    }

    pub(super) fn color(&self) -> Color<u8> {
        self.imp().color.get()
    }

    pub(super) fn set_selected(&self, selected: bool) {
        let overlay = self.imp().overlay.get().expect("overlay initialized");
        if selected {
            self.set_state_flags(gtk::StateFlags::SELECTED, false);
            overlay.set_icon_name(Some("object-select-symbolic"));
        } else {
            self.unset_state_flags(gtk::StateFlags::SELECTED);
            overlay.clear();
        }
    }

    fn activate(&self) {
        if let Some(activate) = self.imp().activate.borrow().as_ref() {
            activate();
        }
    }
}
