use super::*;
use gtk::gdk;
use shrimply_gtk_components::ui::I18nWidgetExt;

const TRACK_ADD_MENU_ICON_SIZE: i32 = 16;
const TRACK_ADD_MENU_ITEM_GAP: i32 = 12;

#[derive(Clone)]
struct TrackAddContext {
    area: gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
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

    let magnet = timeline_tool_button("magnet-tilted-symbolic");
    magnet.set_tooltip_i18n("Magnet");

    let beat_grid = timeline_tool_button("metronome-symbolic");
    beat_grid.set_tooltip_i18n("Beat Grid");

    let pointer = timeline_tool_button("pointer-primary-click-symbolic");
    pointer.set_tooltip_i18n("Pointer");

    let cut = timeline_tool_button("cut-symbolic");
    cut.set_tooltip_i18n("Cut");

    let overwrite = timeline_tool_button("track-insert-symbolic");
    overwrite.set_tooltip_i18n("Overwrite/Insert");

    let block = timeline_tool_button("track-block-symbolic");
    block.set_tooltip_i18n("Block");

    let new_track = timeline_tool_button("track-move-above-symbolic");
    new_track.set_tooltip_i18n("New Track");

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

pub use shrimply_timeline_core::track_controls::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn show_track_add_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    request: TrackAddMenuRequest,
) {
    let TrackAddMenuRequest {
        key,
        import_targets,
    } = request;
    let row = items::row_for_track(&project.borrow(), key.kind, key.track_index)
        .expect("add menu track must exist");
    let view = runtime.borrow().scene.view();
    let context = TrackAddContext {
        area: area.clone(),
        project: project.clone(),
        player_state: player_state.clone(),
        selection_state: selection_state.clone(),
        runtime: runtime.clone(),
    };
    let popover = gtk::Popover::builder()
        .has_arrow(true)
        .position(gtk::PositionType::Bottom)
        .build();
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for entry in track_add_menu(key.kind) {
        let TrackAddMenuEntry::Action(action) = entry else {
            menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            continue;
        };
        let context = context.clone();
        let import_targets = import_targets.clone();
        append_add_menu_item_i18n(
            &menu,
            &popover,
            action.label(key.kind),
            action.icon(),
            move || activate_track_add_action(&context, key, &import_targets, *action),
        );
    }
    popover.set_child(Some(&menu));

    if let Some(existing) = runtime.borrow_mut().active_context_menu.take() {
        existing.popdown();
    }
    let button_y = track_label_button_y(row_screen_y(row, view));
    let parent = area.parent().expect("timeline GLArea must have a parent");
    let point = area
        .compute_point(
            &parent,
            &gtk::graphene::Point::new(TRACK_LABEL_ADD_X as f32, button_y as f32),
        )
        .expect("timeline coordinates must translate to its parent");
    popover.set_halign(gtk::Align::Center);
    popover.set_parent(&parent);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(
        point.x() as i32,
        point.y() as i32,
        TRACK_LABEL_BUTTON_SIZE as i32,
        TRACK_LABEL_BUTTON_SIZE as i32,
    )));
    popover.popup();
    runtime.borrow_mut().active_context_menu = Some(popover);
}

fn activate_track_add_action(
    context: &TrackAddContext,
    key: TrackKey,
    import_targets: &[TrackKey],
    action: TrackAddAction,
) {
    if action == TrackAddAction::Import {
        interaction::open_track_import_dialog(
            &context.area,
            &context.project,
            &context.player_state,
            &context.selection_state,
            &context.runtime,
            import_targets.to_vec(),
        );
        return;
    }
    let runtime = context.runtime.borrow();
    let default_text_font_family = runtime.scene.default_text_font_family.clone();
    let settings = shrimply_timeline_core::TrackAddSettings {
        default_visual_duration: runtime.scene.default_visual_duration,
        default_text_font_family: &default_text_font_family,
    };
    drop(runtime);
    if shrimply_timeline_core::activate_track_add(
        &context.project,
        &context.player_state,
        &context.selection_state,
        key,
        action,
        settings,
    ) == shrimply_timeline_core::TrackAddOutcome::Changed
    {
        context.area.queue_render();
    }
}

fn append_add_menu_item_i18n(
    menu: &gtk::Box,
    popover: &gtk::Popover,
    label: &str,
    icon: &str,
    activate: impl Fn() + 'static,
) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, TRACK_ADD_MENU_ITEM_GAP);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(TRACK_ADD_MENU_ICON_SIZE);
    let label = gtk::Label::new(Some(label));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    content.append(&image);
    content.append(&label);

    let button = gtk::Button::builder()
        .has_frame(false)
        .child(&content)
        .build();
    let popover = popover.clone();
    button.connect_clicked(move |_| {
        popover.popdown();
        activate();
    });
    menu.append(&button);
}
