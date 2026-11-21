use super::*;
use shrimply_components_gtk::ui::I18nWidgetExt;

const TRACK_ADD_MENU_ICON_SIZE: i32 = 16;
const TRACK_ADD_MENU_ITEM_GAP: i32 = 12;

#[derive(Clone)]
struct TrackAddContext {
    area: gtk::glib::WeakRef<gtk::GLArea>,
    runtime: std::rc::Weak<RefCell<TimelineRuntime>>,
    position: crate::project::Time,
}

pub(super) fn timeline_sidebar(
    area: &gtk::GLArea,
    preferences: &preferences_store::SharedPreferences,
) -> gtk::Box {
    let toolbar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    toolbar.set_width_request(SIDEBAR_WIDTH);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);

    use shrimply_timeline_skia::TimelineTool;
    let button = |tool: TimelineTool| {
        let description = tool.description();
        let button = timeline_tool_button(description.icon);
        button.set_tooltip_i18n(description.label);
        button
    };
    let magnet = button(TimelineTool::Magnet);

    let beat_grid = button(TimelineTool::BeatGrid);

    let pointer = button(TimelineTool::Pointer);

    let cut = button(TimelineTool::Cut);

    let overwrite = button(TimelineTool::Overwrite);

    let block = button(TimelineTool::Block);

    let new_track = button(TimelineTool::NewTrack);

    let tools = TimelineTools::new(preferences.clone());
    let state = tools.state();
    magnet.set_active(state.magnet);
    beat_grid.set_active(state.beat_grid);
    pointer.set_active(state.cursor == CursorTool::Pointer);
    cut.set_active(state.cursor == CursorTool::Cut);
    overwrite.set_active(state.drag_collision == DragCollisionMode::Overwrite);
    block.set_active(state.drag_collision == DragCollisionMode::Block);
    new_track.set_active(state.drag_collision == DragCollisionMode::NewTrack);

    let area_for_magnet = area.clone();
    let tools_for_magnet = tools.clone();
    magnet.connect_toggled(move |button| {
        tools_for_magnet.set_magnet(button.is_active());
        area_for_magnet.queue_render();
    });

    let area_for_beat_grid = area.clone();
    let tools_for_beat_grid = tools.clone();
    beat_grid.connect_toggled(move |button| {
        tools_for_beat_grid.set_beat_grid(button.is_active());
        area_for_beat_grid.queue_render();
    });

    let area_for_pointer = area.clone();
    let cut_for_pointer = cut.clone();
    let tools_for_pointer = tools.clone();
    pointer.connect_toggled(move |button| {
        if button.is_active() {
            cut_for_pointer.set_active(false);
            tools_for_pointer.set_cursor(CursorTool::Pointer);
            area_for_pointer.queue_render();
        } else if !cut_for_pointer.is_active() {
            button.set_active(true);
        }
    });

    let area_for_cut = area.clone();
    let pointer_for_cut = pointer.clone();
    let tools_for_cut = tools.clone();
    cut.connect_toggled(move |button| {
        if button.is_active() {
            pointer_for_cut.set_active(false);
            tools_for_cut.set_cursor(CursorTool::Cut);
            tracing::debug!("timeline cut tool enabled");
            area_for_cut.queue_render();
        } else if !pointer_for_cut.is_active() {
            tracing::debug!("timeline cut tool disabled");
            button.set_active(true);
        }
    });

    let area_for_overwrite = area.clone();
    let block_for_overwrite = block.clone();
    let new_track_for_overwrite = new_track.clone();
    let tools_for_overwrite = tools.clone();
    overwrite.connect_toggled(move |button| {
        if button.is_active() {
            block_for_overwrite.set_active(false);
            new_track_for_overwrite.set_active(false);
            tools_for_overwrite.set_drag_collision(DragCollisionMode::Overwrite);
            area_for_overwrite.queue_render();
        } else if !block_for_overwrite.is_active() && !new_track_for_overwrite.is_active() {
            button.set_active(true);
        }
    });

    let area_for_block = area.clone();
    let overwrite_for_block = overwrite.clone();
    let new_track_for_block = new_track.clone();
    let tools_for_block = tools.clone();
    block.connect_toggled(move |button| {
        if button.is_active() {
            overwrite_for_block.set_active(false);
            new_track_for_block.set_active(false);
            tools_for_block.set_drag_collision(DragCollisionMode::Block);
            area_for_block.queue_render();
        } else if !overwrite_for_block.is_active() && !new_track_for_block.is_active() {
            button.set_active(true);
        }
    });

    let area_for_new_track = area.clone();
    let overwrite_for_new_track = overwrite.clone();
    let block_for_new_track = block.clone();
    let tools_for_new_track = tools;
    new_track.connect_toggled(move |button| {
        if button.is_active() {
            overwrite_for_new_track.set_active(false);
            block_for_new_track.set_active(false);
            tools_for_new_track.set_drag_collision(DragCollisionMode::NewTrack);
            area_for_new_track.queue_render();
        } else if !overwrite_for_new_track.is_active() && !block_for_new_track.is_active() {
            button.set_active(true);
        }
    });

    toolbar.append(&magnet);
    toolbar.append(&beat_grid);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    toolbar.append(&pointer);
    toolbar.append(&cut);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    toolbar.append(&overwrite);
    toolbar.append(&block);
    toolbar.append(&new_track);

    toolbar
}

pub(super) fn show_track_add_menu(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    request: TrackAddMenuRequest,
) {
    let TrackAddMenuRequest {
        key,
        import_targets,
        position,
        touch,
        point,
    } = request;
    if let Some(existing) = runtime.borrow_mut().active_context_menu.take() {
        existing.close();
    }
    let context = TrackAddContext {
        area: area.downgrade(),
        runtime: Rc::downgrade(runtime),
        position,
    };
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let presented = if shrimply_components_gtk::layout::menu_uses_bottom_sheet(if touch {
        "touch"
    } else {
        "pointer"
    }) {
        let dialog = crate::context_menu::bottom_sheet(area, &menu);
        crate::context_menu::PresentedMenu::Sheet(dialog)
    } else {
        let parent = area.parent().expect("timeline GLArea must have a parent");
        let point = area
            .compute_point(&parent, &gtk::graphene::Point::new(point.x, point.y))
            .expect("timeline coordinates must translate to its parent");
        let popover = gtk::Popover::builder()
            .has_arrow(true)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Center)
            .child(&menu)
            .build();
        popover.set_parent(&parent);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            point.x() as i32,
            point.y() as i32,
            1,
            1,
        )));
        shrimply_components_gtk::ui::unparent_on_close(&popover);
        popover.popup();
        crate::context_menu::PresentedMenu::Popover(popover)
    };
    for entry in track_add_menu(key.kind) {
        let TrackAddMenuEntry::Action(action) = entry else {
            menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            continue;
        };
        let context = context.clone();
        let import_targets = import_targets.clone();
        append_add_menu_item_i18n(
            &menu,
            &presented,
            action.label(key.kind),
            action.icon(),
            move || activate_track_add_action(&context, key, &import_targets, *action),
        );
    }
    runtime.borrow_mut().active_context_menu = Some(presented);
}

fn activate_track_add_action(
    context: &TrackAddContext,
    key: TrackKey,
    import_targets: &[TrackKey],
    action: TrackAddAction,
) {
    let (Some(area), Some(runtime)) = (context.area.upgrade(), context.runtime.upgrade()) else {
        return;
    };
    if action == TrackAddAction::Import {
        interaction::open_track_import_dialog(
            &area,
            &runtime,
            import_targets.to_vec(),
            context.position,
        );
        return;
    }
    let result = runtime
        .borrow_mut()
        .scene
        .activate_track_add_at(key, action, context.position);
    if let Err(error) = result {
        interaction::show_error_dialog(&area, "Could not add timeline item", &error);
    }
    area.queue_render();
}

fn append_add_menu_item_i18n(
    menu: &gtk::Box,
    presented: &crate::context_menu::PresentedMenu,
    label: &str,
    icon: &str,
    activate: impl Fn() + 'static,
) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, TRACK_ADD_MENU_ITEM_GAP);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(TRACK_ADD_MENU_ICON_SIZE);
    let label = gtk::Label::new(Some(shrimply_components_gtk::i18n::text(label).as_ref()));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    content.append(&image);
    content.append(&label);

    let button = gtk::Button::builder()
        .has_frame(false)
        .child(&content)
        .build();
    presented.close_on_click(&button);
    button.connect_clicked(move |_| activate());
    menu.append(&button);
}
