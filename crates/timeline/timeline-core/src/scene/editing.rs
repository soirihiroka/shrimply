use super::*;
use crate::drawing::item_rect;
use crate::timeline_operation::SequenceTimeline;

impl Scene {
    pub(super) fn begin_item_edit(&mut self, point: Vec2, toggle: bool, extend: bool) {
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        if x < timeline_x() {
            return;
        }
        if y < RULER_HEIGHT {
            self.seeking = true;
            return;
        }
        let project = self.project.borrow();
        let preferences = preferences::snapshot(&self.preferences);
        let tools = ToolState::from_preferences(&preferences);
        if let Some(gesture) = crate::transitions::Gesture::begin(
            &project,
            &self.selection,
            self.view,
            point.as_dvec2(),
        ) {
            self.transition = Some(gesture);
            return;
        }
        let nested = folded_sequence::hit_projected_item(&project, self.view, x, y);
        let root = items::hit_item_at(&project, self.view, x, y);
        let address = nested
            .as_ref()
            .map(|hit| hit.key.clone())
            .or_else(|| root.and_then(|key| selection_state::item_address(&project, key)));
        if let Some(address) = address {
            let scope = SequenceTimeline::for_item(&project, &address).expect("hit item scope");
            if (tools.cursor != CursorTool::Cut || nested.is_some())
                && !crate::selection::select_item_in_context(
                    &scope,
                    &project,
                    &self.selection,
                    address.clone(),
                    toggle,
                    extend,
                )
            {
                return;
            }
            let selected = selection_state::selected_item_addresses(&self.selection, &project);
            if let Some(hit) = root {
                self.dragged_group = items::dragged_group_for_hit(
                    &project,
                    &selection_state::selected_items(&self.selection),
                    hit,
                    self.view,
                    x,
                    tools.drag_collision,
                );
            }
            if tools.cursor == CursorTool::Cut {
                self.dragged_group = None;
                self.cutting = true;
                self.cut_preview = items::cut_time_for_address(
                    &project,
                    self.view,
                    &address,
                    x,
                    &self.snap_repository,
                )
                .map(|time| crate::cutting::timeline_cut(&project, &selected, address, time));
            } else if let Some(hit) = nested {
                let (left, width) = item_rect(hit.start, hit.end, timeline_x(), self.view);
                let kind = if x <= left + ITEM_RESIZE_HANDLE_WIDTH {
                    folded_sequence::FoldedDragKind::ResizeStart
                } else if x >= left + width - ITEM_RESIZE_HANDLE_WIDTH {
                    folded_sequence::FoldedDragKind::ResizeEnd
                } else {
                    folded_sequence::FoldedDragKind::Move
                };
                self.dragged_group = None;
                self.folded_drag = folded_sequence::begin_drag(
                    &project,
                    hit,
                    kind,
                    crate::math::time_at_x(self.view, x).as_secs_f64(),
                    &selected,
                );
            } else if let Some((hit, edge)) =
                items::hit_resize_handle_at(&project, self.view, x, y, ITEM_RESIZE_HANDLE_WIDTH)
            {
                self.dragged_group = None;
                self.resize_drag = items::resize_drag_for_hit(
                    &project,
                    &selection_state::selected_items(&self.selection),
                    hit,
                    edge,
                    tools.drag_collision,
                );
            }
        } else if tools.cursor == CursorTool::Cut {
            if !toggle && !extend {
                selection_state::set_selected_gap(
                    &self.selection,
                    items::hit_gap_at(&project, self.view, x, y),
                );
            }
        } else {
            let time = crate::math::time_at_x(self.view, x);
            let time = self.snap_repository.snap(time).unwrap_or(time);
            let y = y.max(RULER_HEIGHT) + self.view.scroll_y;
            self.view.selection = Some(TimelineSelection {
                start: time,
                end: time,
                start_y: y,
                end_y: y,
                add_to_selection: toggle,
                ignore_grouping: extend,
            });
        }
    }

    pub(super) fn update_item_edit(&mut self, point: Vec2) -> bool {
        let project = self.project.borrow();
        let x = f64::from(point.x);
        if let Some(gesture) = &mut self.transition {
            if self.drag_moved {
                let mut view = self.view;
                view.drag_moved = true;
                gesture.update(&project, &self.selection, view, x, &self.snap_repository);
            }
            return true;
        }
        if self.cutting {
            return true;
        }
        if let Some(drag) = &mut self.resize_drag {
            if self.drag_moved {
                items::update_resize_drag(drag, &project, self.view, x, &self.snap_repository);
            }
            return true;
        }
        if let Some(drag) = &mut self.folded_drag {
            if self.drag_moved {
                crate::dragging::update_nested(
                    drag,
                    &project,
                    crate::dragging::DragRequest {
                        view: self.view,
                        position: point.as_dvec2(),
                        collision_mode: ToolState::from_preferences(&preferences::snapshot(
                            &self.preferences,
                        ))
                        .drag_collision,
                        snap_repository: &self.snap_repository,
                    },
                );
            }
            return true;
        }
        let time = crate::math::time_at_x(self.view, x);
        let time = self.snap_repository.snap(time).unwrap_or(time);
        let y = f64::from(point.y).max(RULER_HEIGHT) + self.view.scroll_y;
        if let Some(selection) = &mut self.view.selection {
            selection.end = time;
            selection.end_y = y;
            return true;
        }
        false
    }

    pub(super) fn finish_item_edit(&mut self, point: Vec2) -> Result<bool, String> {
        self.update_item_edit(point);
        if let Some(gesture) = self.transition.take() {
            let mut edited = self.project.borrow().clone();
            if let Some(applied) = gesture.finish(&mut edited, self.drag_moved) {
                self.commit_context_edit(edited, applied.message, None)?;
                if let Some(side) = applied.focus {
                    selection_state::set_focused_transition(&self.selection, side);
                } else {
                    selection_state::clear_focused_transition(&self.selection);
                }
            }
            return Ok(true);
        }
        if let Some(selection) = self.view.selection.take() {
            let project = self.project.borrow();
            if self.drag_moved {
                let row = ((selection.start_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
                let scope = items::track_rows(&project)
                    .get(row)
                    .filter(|row| row.root_key.is_none())
                    .and_then(|row| SequenceTimeline::for_track(&project, &row.address))
                    .unwrap_or_else(SequenceTimeline::root);
                crate::selection::commit_rectangle_selection(
                    &scope,
                    &project,
                    &self.selection,
                    selection,
                );
            } else if !selection.add_to_selection {
                if !selection.ignore_grouping
                    && let Some(gap) =
                        items::hit_gap_at(&project, self.view, point.x.into(), point.y.into())
                {
                    selection_state::set_selected_gap(&self.selection, Some(gap));
                } else {
                    selection_state::set_selected_items(&self.selection, Vec::new(), None);
                }
            }
            return Ok(true);
        }
        if !self.cutting && self.resize_drag.is_none() && self.folded_drag.is_none() {
            return Ok(false);
        }
        let mut edited = self.project.borrow().clone();
        let mut commit_name = "edit-timeline-items";
        let grabbed = self
            .folded_drag
            .as_ref()
            .map(|drag| drag.key.item_id())
            .or_else(|| self.cut_preview.as_ref().map(|cut| cut.key.item_id()));
        let selection = if std::mem::take(&mut self.cutting) {
            self.cut_preview
                .take()
                .filter(|_| !self.drag_moved)
                .and_then(|cut| {
                    let scope = SequenceTimeline::for_item(&edited, &cut.key)?;
                    let (selected, _) =
                        items::split_item_addresses(&scope, &mut edited, &cut.keys, cut.time);
                    (!selected.is_empty()).then_some(selected)
                })
        } else if let Some(drag) = self.resize_drag.take() {
            if self.drag_moved {
                items::apply_resize_drag(&mut edited, drag).map(|selected| {
                    selected
                        .into_iter()
                        .filter_map(|key| selection_state::item_address(&edited, key))
                        .collect()
                })
            } else {
                None
            }
        } else if let Some(mut drag) = self.folded_drag.take().filter(|_| self.drag_moved) {
            crate::dragging::finish_nested(
                &mut drag,
                &mut edited,
                crate::dragging::DragRequest {
                    view: self.view,
                    position: point.as_dvec2(),
                    collision_mode: ToolState::from_preferences(&preferences::snapshot(
                        &self.preferences,
                    ))
                    .drag_collision,
                    snap_repository: &self.snap_repository,
                },
            )
            .map(|outcome| {
                commit_name = outcome.commit_name;
                outcome.selected
            })
        } else {
            None
        };
        if let Some(selection) = selection {
            let focused =
                grabbed.and_then(|id| selection.iter().find(|key| key.item_id() == id).cloned());
            self.commit_context_edit(edited, commit_name, Some(selection.clone()))?;
            if let Some(focused) = focused {
                selection_state::set_selected_item_addresses(
                    &self.selection,
                    &self.project.borrow(),
                    selection,
                    Some(focused),
                );
            }
        }
        Ok(true)
    }
}

impl Scene {
    pub(super) fn update_cut_preview(&mut self, point: Vec2) {
        if ToolState::from_preferences(&preferences::snapshot(&self.preferences)).cursor
            != CursorTool::Cut
            || self.scrollbar_at(point).is_some()
            || self.scrollbar_drag.is_some()
            || self.seeking
            || self.dragged_group.is_some()
            || self.resize_drag.is_some()
            || self.folded_drag.is_some()
            || self.transition.is_some()
            || self.view.selection.is_some()
            || self.view.drag_mode == DragMode::MiddlePan
        {
            self.cut_preview = None;
            return;
        }
        if !self.cutting {
            self.snap_repository = self.build_snap_repository();
        }
        let project = self.project.borrow();
        self.cut_preview = crate::cutting::preview(
            &project,
            &selection_state::selected_item_addresses(&self.selection, &project),
            self.cut_preview.as_ref(),
            crate::cutting::PreviewRequest {
                view: self.view,
                position: point.as_dvec2(),
                active: self.cutting,
                snaps: &self.snap_repository,
            },
        );
    }
}
