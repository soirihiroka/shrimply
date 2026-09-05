use crate::{
    project::{ItemAddress, Project},
    selection_state,
    timeline_operation::TimelineOperationContext,
};

pub fn commit_rectangle_selection(
    context: &impl crate::timeline_operation::TimelineOperationContext,
    project: &Project,
    selection_state: &selection_state::SharedSelectionState,
    selection: crate::TimelineSelection,
) {
    let mut selected = if selection.add_to_selection {
        selection_state::selected_item_addresses(selection_state, project)
            .into_iter()
            .filter(|item| context.contains_item(project, item))
            .collect()
    } else {
        Vec::new()
    };
    selected.extend(crate::items::selected_item_addresses(
        context, project, selection,
    ));
    if !selection.ignore_grouping {
        selected = crate::items::expand_grouped_item_addresses(context, project, &selected);
    }
    let context_items = context.items(project);
    selected.sort_by_key(|item| {
        context_items
            .iter()
            .position(|candidate| candidate == item)
            .unwrap_or(usize::MAX)
    });
    selected.dedup();
    selection_state::set_selected_item_addresses(selection_state, project, selected, None);
}

pub fn select_item_in_context(
    context: &dyn TimelineOperationContext,
    project: &Project,
    selection_state: &selection_state::SharedSelectionState,
    hit: ItemAddress,
    ctrl: bool,
    shift: bool,
) -> bool {
    assert!(
        context.contains_item(project, &hit),
        "selected item must belong to its operation context"
    );
    let mut selected = selection_state::selected_item_addresses(selection_state, project)
        .into_iter()
        .filter(|item| context.contains_item(project, item))
        .collect::<Vec<_>>();
    let members = if shift {
        vec![hit.clone()]
    } else {
        crate::items::expand_grouped_item_addresses(context, project, std::slice::from_ref(&hit))
    };

    if ctrl {
        if members.iter().all(|item| selected.contains(item)) {
            selected.retain(|item| !members.contains(item));
        } else {
            selected.extend(members);
        }
    } else if !selected.contains(&hit) || shift {
        selected = members;
    }

    let context_items = context.items(project);
    selected.sort_by_key(|item| {
        context_items
            .iter()
            .position(|candidate| candidate == item)
            .unwrap_or(usize::MAX)
    });
    selected.dedup();
    let focused = selected.contains(&hit).then_some(hit.clone());
    let hit_selected = focused.is_some();
    selection_state::set_selected_item_addresses(selection_state, project, selected, focused);
    hit_selected
}

pub fn select_item_group_in_context(
    context: &dyn TimelineOperationContext,
    project: &Project,
    selection_state: &selection_state::SharedSelectionState,
    hit: ItemAddress,
) {
    assert!(
        context.contains_item(project, &hit),
        "selected item must belong to its operation context"
    );
    let mut selected =
        crate::items::expand_grouped_item_addresses(context, project, std::slice::from_ref(&hit));
    let context_items = context.items(project);
    selected.sort_by_key(|item| {
        context_items
            .iter()
            .position(|candidate| candidate == item)
            .unwrap_or(usize::MAX)
    });
    selected.dedup();
    selection_state::set_selected_item_addresses(selection_state, project, selected, Some(hit));
}
