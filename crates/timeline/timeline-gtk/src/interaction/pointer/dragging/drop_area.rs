use super::*;
use crate::items::DragCollisionMode;
use crate::project::{AudioSource, ItemAddress, TrackAddress, VideoItemContent};
use crate::timeline_operation::TimelineOperationContext;

#[derive(Clone)]
pub(super) enum DropDestination {
    Track(TrackAddress),
    NewTrack(Vec<uuid::Uuid>),
}

pub(super) struct DragDropContext<'a> {
    pub(super) project: &'a Project,
    pub(super) view: TimelineViewState,
    pub(super) position: glam::DVec2,
    pub(super) collision_mode: DragCollisionMode,
}

#[derive(Clone)]
pub(super) struct ItemDrop {
    source: ItemAddress,
    destination: DropDestination,
    start: Time,
    end: Time,
    collision_mode: DragCollisionMode,
}

impl ItemDrop {
    pub(super) fn at(
        context: DragDropContext<'_>,
        operation: &dyn TimelineOperationContext,
        source: &ItemAddress,
        times: (Time, Time),
    ) -> Option<Self> {
        let (start, end) = times;
        let destination = target(
            context.project,
            context.view,
            context.position.x,
            context.position.y,
            source,
            operation,
        )?;
        Some(Self {
            source: source.clone(),
            destination,
            start,
            end,
            collision_mode: context.collision_mode,
        })
    }

    pub(super) fn target_track(&self) -> Option<&TrackAddress> {
        match &self.destination {
            DropDestination::Track(track) => Some(track),
            DropDestination::NewTrack(_) => None,
        }
    }

    pub(super) fn is_nested(&self) -> bool {
        self.destination.is_nested()
    }

    pub(super) fn can_apply(&self, project: &Project) -> bool {
        self.clone().apply(&mut project.clone()).is_some()
    }

    pub(super) fn apply(self, project: &mut Project) -> Option<ItemAddress> {
        self.destination.move_item(
            project,
            &self.source,
            self.start,
            self.end,
            self.collision_mode,
        )
    }
}

impl DropDestination {
    pub(super) fn move_item(
        &self,
        project: &mut Project,
        source: &ItemAddress,
        start: Time,
        end: Time,
        collision_mode: DragCollisionMode,
    ) -> Option<ItemAddress> {
        match self {
            Self::Track(track) => {
                let start = project
                    .timeline_time_to_sequence(track, start)?
                    .snapped(project.frame_step());
                let end = project
                    .timeline_time_to_sequence(track, end)?
                    .snapped(project.frame_step());
                let (start, end) = ordered_times(start, end);
                crate::folded_sequence::move_item_with_collision(
                    project,
                    source,
                    track,
                    start,
                    end,
                    collision_mode,
                )
            }
            Self::NewTrack(path) => {
                let start = project
                    .timeline_time_to_sequence_path(source.kind(), path, start)?
                    .snapped(project.frame_step());
                let end = project
                    .timeline_time_to_sequence_path(source.kind(), path, end)?
                    .snapped(project.frame_step());
                let (start, end) = ordered_times(start, end);
                let moved = project.move_item_to_new_track(source, path, start, end)?;
                if !project.expanded_sequence_paths.contains(path) {
                    project.expanded_sequence_paths.push(path.clone());
                }
                Some(moved)
            }
        }
    }

    pub(super) fn is_nested(&self) -> bool {
        match self {
            Self::Track(track) => !track.is_root(),
            Self::NewTrack(path) => !path.is_empty(),
        }
    }
}

trait DropArea {
    fn target(
        &self,
        project: &Project,
        view: TimelineViewState,
        x: f64,
        y: f64,
        source: &ItemAddress,
    ) -> Option<DropDestination>;
}

struct FoldedProxy;

impl DropArea for FoldedProxy {
    fn target(
        &self,
        project: &Project,
        view: TimelineViewState,
        x: f64,
        y: f64,
        source: &ItemAddress,
    ) -> Option<DropDestination> {
        let host = root_folder_at(project, view, x, y)
            .or_else(|| nested_folder_at(project, view, x, y))?;
        if host.item_id() == source.item_id() {
            return None;
        }
        let reference = folder_reference(project, &host)?;
        let host_scope = project.item_scope(&host)?;
        let target_scope = host_scope.child(reference.instance_id);
        let path = project.sequence_path_for_scope(source.kind(), &target_scope)?;
        (!path.contains(&source.item_id()) && project.can_move_item_to_sequence_path(source, &path))
            .then_some(DropDestination::NewTrack(path))
    }
}

struct TrackRow;

impl DropArea for TrackRow {
    fn target(
        &self,
        project: &Project,
        view: TimelineViewState,
        _x: f64,
        y: f64,
        source: &ItemAddress,
    ) -> Option<DropDestination> {
        if y < RULER_HEIGHT {
            return None;
        }
        let row = ((y + view.scroll_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
        let target = crate::items::track_rows(project).get(row)?.address.clone();
        (target.kind() == source.kind()
            && !target.sequence_path().contains(&source.item_id())
            && project.can_move_item_to_sequence_path(source, target.sequence_path()))
        .then_some(DropDestination::Track(target))
    }
}

fn target(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
    source: &ItemAddress,
    operation: &dyn TimelineOperationContext,
) -> Option<DropDestination> {
    let areas: [&dyn DropArea; 2] = [&FoldedProxy, &TrackRow];
    areas.into_iter().find_map(|area| {
        let destination = area.target(project, view, x, y, source)?;
        destination_allowed(project, &destination, source, operation).then_some(destination)
    })
}

fn destination_allowed(
    project: &Project,
    destination: &DropDestination,
    source: &ItemAddress,
    operation: &dyn TimelineOperationContext,
) -> bool {
    match destination {
        DropDestination::Track(target) => operation.allows_track_drop(project, source, target),
        DropDestination::NewTrack(path) => operation.allows_new_track_drop(project, source, path),
    }
}

fn ordered_times(first: Time, second: Time) -> (Time, Time) {
    (first.min(second), first.max(second))
}

fn root_folder_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<ItemAddress> {
    let key = hit_item_at(project, view, x, y)?;
    let address = selection_state::item_address(project, key)?;
    is_folder(project, &address).then_some(address)
}

fn nested_folder_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<ItemAddress> {
    let hit = crate::folded_sequence::hit_projected_item(project, view, x, y)?;
    let address = hit.key;
    is_folder(project, &address).then_some(address)
}

fn is_folder(project: &Project, address: &ItemAddress) -> bool {
    folder_reference(project, address).is_some()
}

fn folder_reference(
    project: &Project,
    address: &ItemAddress,
) -> Option<crate::project::SequenceReference> {
    match project.item(address) {
        Some(crate::project::ItemRef::Video(item)) => match item.content {
            VideoItemContent::FoldedSequence(reference) => Some(reference),
            _ => None,
        },
        Some(crate::project::ItemRef::Audio(item)) => match item.source {
            AudioSource::FoldedSequence(reference) => Some(reference),
            AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
        },
        _ => None,
    }
}
