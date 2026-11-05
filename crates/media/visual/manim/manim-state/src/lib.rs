use hashbrown::HashMap;
use shrimply_math_core::Time;
use shrimply_project_document::project::{
    ManimParameter, ManimParameterValue, Project, VideoItem, VideoItemContent,
    clamp_item_end_to_next_start,
};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

type ErrorStatus = (u64, String, HashMap<String, ManimParameterValue>, String);
static ERRORS: OnceLock<Mutex<HashMap<Uuid, ErrorStatus>>> = OnceLock::new();
type ParameterStatus = (
    u64,
    String,
    HashMap<String, ManimParameterValue>,
    Vec<ManimParameter>,
);
static PARAMETERS: OnceLock<Mutex<HashMap<Uuid, ParameterStatus>>> = OnceLock::new();

#[derive(Clone)]
pub struct SourceIdentity {
    pub item_id: Uuid,
    pub source_revision: u64,
    pub scene: String,
    pub input_parameters: HashMap<String, ManimParameterValue>,
}

impl SourceIdentity {
    pub fn duration(&self, duration: shrimply_math_core::Time) -> Update {
        Update::Duration {
            source: self.clone(),
            duration,
        }
    }

    pub fn parameters(&self, parameters: Vec<ManimParameter>, render_is_current: bool) -> Update {
        Update::Parameters {
            source: self.clone(),
            parameters,
            render_is_current,
        }
    }

    pub fn error(&self, error: Option<String>) -> Update {
        Update::Error {
            source: self.clone(),
            error,
        }
    }
}

#[derive(Clone)]
pub enum Update {
    Duration {
        source: SourceIdentity,
        duration: shrimply_math_core::Time,
    },
    Parameters {
        source: SourceIdentity,
        parameters: Vec<ManimParameter>,
        render_is_current: bool,
    },
    Error {
        source: SourceIdentity,
        error: Option<String>,
    },
}

/// Accurate captures use an immutable project snapshot: parameter reconciliation
/// requires an updated project, so waiting for it here cannot make progress.
pub fn validate_render_updates(updates: Vec<Update>) -> Result<(), String> {
    for update in updates {
        match update {
            Update::Parameters {
                render_is_current: false,
                ..
            } => {
                return Err("Manim parameters changed while preparing the frame; wait for the preview to update and try again".into());
            }
            Update::Error {
                error: Some(error), ..
            } => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplyResult {
    pub duration: Option<Time>,
    pub video: bool,
    pub inspector: bool,
}

struct ParameterUpdate {
    item_id: Uuid,
    source_revision: u64,
    scene: String,
    input_parameters: HashMap<String, ManimParameterValue>,
    parameters: Vec<ManimParameter>,
    render_is_current: bool,
}

pub fn apply(project: &mut Project, update: Update) -> Option<ApplyResult> {
    match update {
        Update::Duration { source, duration } => apply_duration(project, &source, duration),
        Update::Parameters {
            source,
            parameters,
            render_is_current,
        } => apply_parameters(
            project,
            ParameterUpdate {
                item_id: source.item_id,
                source_revision: source.source_revision,
                scene: source.scene,
                input_parameters: source.input_parameters,
                parameters,
                render_is_current,
            },
        ),
        Update::Error { source, error } => apply_error(project, source, error),
    }
}

fn apply_duration(
    project: &mut Project,
    source: &SourceIdentity,
    duration: Time,
) -> Option<ApplyResult> {
    let next_start = project
        .video_tracks
        .iter()
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.video_tracks),
        )
        .find_map(|track| {
            track
                .items
                .iter()
                .position(|item| item.id == source.item_id)
                .and_then(|index| track.items.get(index + 1))
                .map(|item| item.start)
        });
    let item = project.video_item_by_id_mut(source.item_id)?;
    if !source_matches(item, source) || item.source_duration == duration {
        return None;
    }
    let previous_natural_end = shrimply_project_document::project::media_item_natural_end_position(
        item.start,
        item.animation_time_offset,
        item.source_duration,
        item.playback_speed,
        item.repeat_strategy,
    );
    let followed_natural_end = follows_natural_end(item.end, previous_natural_end, next_start);
    item.source_duration = duration;
    if followed_natural_end
        && let Some(end) = shrimply_project_document::project::media_item_natural_end_position(
            item.start,
            item.animation_time_offset,
            duration,
            item.playback_speed,
            item.repeat_strategy,
        )
    {
        item.end = clamp_item_end_to_next_start(end, next_start);
    }
    shrimply_project_document::project::commit_edit(project, "manim-source-duration");
    Some(ApplyResult {
        duration: Some(project.duration()),
        video: true,
        inspector: true,
    })
}

fn follows_natural_end(
    item_end: Time,
    natural_end: Option<Time>,
    next_start: Option<Time>,
) -> bool {
    natural_end.map(|end| clamp_item_end_to_next_start(end, next_start)) == Some(item_end)
}

fn apply_parameters(project: &mut Project, update: ParameterUpdate) -> Option<ApplyResult> {
    let ParameterUpdate {
        item_id,
        source_revision,
        scene,
        input_parameters,
        parameters,
        render_is_current,
    } = update;
    let item = project.video_item_by_id_mut(item_id)?;
    let source = SourceIdentity {
        item_id,
        source_revision,
        scene: scene.clone(),
        input_parameters: input_parameters.clone(),
    };
    if !source_matches(item, &source) {
        return None;
    }
    let VideoItemContent::Manim(manim) = &mut item.content else {
        return None;
    };
    let mut reflected_values = manim.parameters.clone();
    reflected_values.clear();
    reflected_values.extend(
        parameters
            .iter()
            .map(|parameter| (parameter.key.clone(), parameter.value.clone())),
    );
    let changed = set_parameters(
        item_id,
        source_revision,
        scene,
        input_parameters,
        parameters,
    );
    let reconciled = !render_is_current && manim.parameters != reflected_values;
    if reconciled {
        manim.parameters = reflected_values;
        shrimply_project_document::project::commit_edit(project, "reconcile-manim-parameters");
    }
    (changed || reconciled).then_some(ApplyResult {
        video: reconciled,
        inspector: true,
        ..Default::default()
    })
}

fn apply_error(
    project: &Project,
    source: SourceIdentity,
    error: Option<String>,
) -> Option<ApplyResult> {
    let current = project
        .video_item_by_id(source.item_id)
        .is_some_and(|item| error_source_matches(item, &source));
    (current
        && set_error(
            source.item_id,
            source.source_revision,
            source.scene,
            source.input_parameters,
            error,
        ))
    .then_some(ApplyResult {
        inspector: true,
        ..Default::default()
    })
}

fn source_matches(item: &VideoItem, source: &SourceIdentity) -> bool {
    let VideoItemContent::Manim(manim) = &item.content else {
        return false;
    };
    item.file
        .snapshot()
        .is_ok_and(|snapshot| snapshot.revision() == source.source_revision)
        && manim.scene == source.scene
        && manim.parameters == source.input_parameters
}

fn error_source_matches(item: &VideoItem, source: &SourceIdentity) -> bool {
    let VideoItemContent::Manim(manim) = &item.content else {
        return false;
    };
    manim.scene == source.scene
        && manim.parameters == source.input_parameters
        && item.file.snapshot().map_or(true, |snapshot| {
            snapshot.revision() == source.source_revision
        })
}

pub fn set_parameters(
    item_id: Uuid,
    source_revision: u64,
    scene: String,
    input_parameters: HashMap<String, ManimParameterValue>,
    parameters: Vec<ManimParameter>,
) -> bool {
    let mut values = PARAMETERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Manim parameters lock is poisoned");
    let next = (source_revision, scene, input_parameters, parameters);
    if values.get(&item_id) == Some(&next) {
        false
    } else {
        values.insert(item_id, next);
        true
    }
}

pub fn parameters(
    item_id: Uuid,
    source_revision: u64,
    scene: &str,
    input_parameters: &HashMap<String, ManimParameterValue>,
) -> Option<Vec<ManimParameter>> {
    PARAMETERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Manim parameters lock is poisoned")
        .get(&item_id)
        .filter(|(revision, stored_scene, _, _)| {
            *revision == source_revision && stored_scene == scene
        })
        .map(|(_, _, stored_parameters, parameters)| {
            let mut parameters = parameters.clone();
            if stored_parameters != input_parameters {
                for parameter in &mut parameters {
                    parameter.value.clone_from(
                        input_parameters
                            .get(&parameter.key)
                            .unwrap_or(&parameter.default),
                    );
                }
            }
            parameters
        })
}

fn set_error(
    item_id: Uuid,
    source_revision: u64,
    scene: String,
    input_parameters: HashMap<String, ManimParameterValue>,
    error: Option<String>,
) -> bool {
    let mut errors = ERRORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Manim status lock is poisoned");
    match error {
        Some(error) => {
            let next = (source_revision, scene, input_parameters, error);
            if errors.get(&item_id) == Some(&next) {
                false
            } else {
                errors.insert(item_id, next);
                true
            }
        }
        None => errors.remove(&item_id).is_some(),
    }
}

pub fn error(
    item_id: Uuid,
    source_revision: u64,
    scene: &str,
    input_parameters: &HashMap<String, ManimParameterValue>,
) -> Option<String> {
    ERRORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("Manim status lock is poisoned")
        .get(&item_id)
        .filter(|(revision, stored_scene, stored_parameters, _)| {
            *revision == source_revision
                && stored_scene == scene
                && stored_parameters == input_parameters
        })
        .map(|(_, _, _, error)| error.clone())
}

#[cfg(test)]
mod tests {
    use super::follows_natural_end;
    use shrimply_math_core::Time;

    #[test]
    fn clamped_clip_still_follows_its_natural_end() {
        assert!(follows_natural_end(
            Time::from_seconds(8),
            Some(Time::from_seconds(12)),
            Some(Time::from_seconds(8)),
        ));
    }

    #[test]
    fn manually_trimmed_clip_does_not_follow_its_natural_end() {
        assert!(!follows_natural_end(
            Time::from_seconds(7),
            Some(Time::from_seconds(12)),
            Some(Time::from_seconds(8)),
        ));
    }
}
