//! GTK widgets render the shared paint toolbar; tool policy lives in paint-edit-skia.
use super::*;
use shrimply_paint_edit_skia::toolbar::{self, Action, Presentation};

struct Widgets {
    button: gtk::ToggleButton,
    color: Option<Rc<Cell<[f32; 4]>>>,
}

struct Rail {
    tools: glib::WeakRef<gtk::Box>,
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    surface: PreviewController,
    model: RefCell<Option<Presentation>>,
    widgets: RefCell<Vec<Widgets>>,
}

pub(super) fn new(
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    surface: PreviewController,
) -> gtk::Box {
    let tools = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let rail = Rc::new(Rail {
        tools: tools.downgrade(),
        project,
        player,
        selection,
        surface,
        model: RefCell::new(None),
        widgets: RefCell::new(Vec::new()),
    });
    refresh(&rail);
    let update = rail.clone();
    selection_state::connect_named(&rail.selection, "preview paint tools", move || {
        refresh(&update)
    });
    let update = rail.clone();
    player_state::connect_named(&rail.player, "preview paint toolbar", move |_| {
        refresh(&update)
    });
    tools.add_tick_callback(move |_, _| {
        refresh(&rail);
        glib::ControlFlow::Continue
    });
    tools
}

fn presentation(rail: &Rail) -> Option<Presentation> {
    let project = rail.project.borrow();
    let address = selection_state::focused_video_address(&rail.selection, &project)?;
    let item = project.video_item(&address)?;
    let project::VideoItemContent::Paint(paint) = &item.content else {
        return None;
    };
    let sequence_time = project.timeline_time_to_sequence(
        &address.track(),
        player_state::snapshot(&rail.player).position,
    )?;
    let time = project::generated_item_time(item, sequence_time).unwrap_or(Time::ZERO);
    Some(
        rail.surface
            .preview_state(PAINT_PREVIEW_STATE, |state: &PaintPreviewState| {
                Presentation::new(state, paint, time)
            }),
    )
}

fn activate(rail: &Rail, action: Action, value: Option<f32>) {
    let project = rail.project.borrow();
    let Some(address) = selection_state::focused_video_address(&rail.selection, &project) else {
        return;
    };
    let Some(item) = project.video_item(&address) else {
        return;
    };
    let project::VideoItemContent::Paint(paint) = &item.content else {
        return;
    };
    rail.surface
        .update_preview_state(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
            toolbar::apply(state, paint, action, value)
                .expect("paint toolbar action became invalid");
        });
}

fn refresh(rail: &Rc<Rail>) {
    let Some(tools) = rail.tools.upgrade() else {
        return;
    };
    let model = presentation(rail);
    tools.set_visible(model.is_some());
    if *rail.model.borrow() == model {
        return;
    }
    let same_structure = rail
        .model
        .borrow()
        .as_ref()
        .zip(model.as_ref())
        .is_some_and(|(before, after)| {
            before
                .buttons
                .iter()
                .map(|button| button.action)
                .eq(after.buttons.iter().map(|button| button.action))
        });
    let Some(model) = model else {
        rail.model.borrow_mut().take();
        return;
    };
    if !same_structure {
        rail.widgets.borrow_mut().clear();
        while let Some(child) = tools.first_child() {
            tools.remove(&child);
        }
        for control in &model.buttons {
            if control.separator {
                tools.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            let button = gtk::ToggleButton::new();
            button.set_size_request(PREVIEW_TOOLBAR_ICON_SIZE, PREVIEW_TOOLBAR_ICON_SIZE);
            button.set_margin_start(4);
            button.set_margin_end(4);
            button.set_margin_top(2);
            button.set_margin_bottom(2);
            button.add_css_class("preview-control-chip");
            button.add_css_class("flat");
            let color = control.color.map(|color| {
                let color = Rc::new(Cell::new(color));
                let swatch = gtk::DrawingArea::new();
                swatch.set_content_width(PREVIEW_TOOLBAR_ICON_SIZE - 6);
                swatch.set_content_height(PREVIEW_TOOLBAR_ICON_SIZE - 6);
                let value = color.clone();
                swatch.set_draw_func(move |_, context, width, height| {
                    let [red, green, blue, alpha] = value.get();
                    context.set_source_rgba(red.into(), green.into(), blue.into(), alpha.into());
                    context.arc(
                        f64::from(width) * 0.5,
                        f64::from(height) * 0.5,
                        f64::from(width.min(height)) * 0.5,
                        0.0,
                        std::f64::consts::TAU,
                    );
                    context.fill().expect("draw paint palette swatch");
                });
                button.set_child(Some(&swatch));
                color
            });
            if let Some(icon) = control.icon {
                button.set_child(Some(&gtk::Image::from_icon_name(icon)));
            }
            if control.action == Action::Size {
                size_popover(rail, &button);
            } else {
                let update = Rc::downgrade(rail);
                let action = control.action;
                button.connect_clicked(move |_| {
                    if let Some(rail) = update.upgrade() {
                        activate(&rail, action, None);
                        refresh(&rail);
                    }
                });
            }
            tools.append(&button);
            rail.widgets.borrow_mut().push(Widgets { button, color });
        }
    }
    for (widget, control) in rail.widgets.borrow().iter().zip(&model.buttons) {
        widget.button.set_tooltip_text(Some(&control.label));
        widget.button.set_active(control.active);
        if let Some(text) = &control.text {
            widget.button.set_label(text);
        }
        if let Some((color, value)) = widget.color.as_ref().zip(control.color) {
            color.set(value);
            widget.button.queue_draw();
        }
    }
    *rail.model.borrow_mut() = Some(model);
}

fn size_popover(rail: &Rc<Rail>, button: &gtk::ToggleButton) {
    let update = Rc::downgrade(rail);
    shrimply_components_gtk::ui::adaptive_popover(button, move || {
        let rail = update.upgrade().expect("paint toolbar is alive");
        let size = presentation(&rail).expect("paint toolbar is visible").size;
        let slider = gtk::Scale::with_range(
            gtk::Orientation::Vertical,
            size.minimum.into(),
            size.maximum.into(),
            size.step.into(),
        );
        slider.set_size_request(56, 280);
        slider.set_draw_value(false);
        slider.set_digits(size.digits as i32);
        slider.adjustment().configure(
            size.value.into(),
            size.minimum.into(),
            size.maximum.into(),
            size.step.into(),
            size.page.into(),
            0.0,
        );
        let update = Rc::downgrade(&rail);
        slider.connect_value_changed(move |slider| {
            if let Some(rail) = update.upgrade() {
                activate(&rail, Action::Size, Some(slider.value() as f32));
                refresh(&rail);
            }
        });
        slider.upcast()
    });
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    let update = Rc::downgrade(rail);
    gesture.connect_pressed(move |_, _, _, _| {
        let Some(rail) = update.upgrade() else { return };
        let state = rail
            .surface
            .preview_state(PAINT_PREVIEW_STATE, |state: &PaintPreviewState| {
                state.clone()
            });
        let value = toolbar::stepped_size(&state, false);
        activate(&rail, Action::Size, Some(value));
        refresh(&rail);
    });
    button.add_controller(gesture);
}
