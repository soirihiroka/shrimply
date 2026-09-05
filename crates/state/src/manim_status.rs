use hashbrown::HashMap;
use shrimply_manim_core::SourceIdentity;
pub use shrimply_manim_core::Update;
use std::sync::{Mutex, OnceLock};

use uuid::Uuid;

type ErrorStatus = (
    u64,
    String,
    HashMap<String, shrimply_project::project::ManimParameterValue>,
    String,
);
static ERRORS: OnceLock<Mutex<HashMap<Uuid, ErrorStatus>>> = OnceLock::new();
type ParameterStatus = (
    u64,
    String,
    HashMap<String, shrimply_project::project::ManimParameterValue>,
    Vec<shrimply_project::project::ManimParameter>,
);
static PARAMETERS: OnceLock<Mutex<HashMap<Uuid, ParameterStatus>>> = OnceLock::new();

struct ParameterUpdate {
    item_id: Uuid,
    source_revision: u64,
    scene: String,
    input_parameters: HashMap<String, shrimply_project::project::ManimParameterValue>,
    parameters: Vec<shrimply_project::project::ManimParameter>,
    render_is_current: bool,
}

pub fn apply(
    project: &std::rc::Rc<std::cell::RefCell<shrimply_project::project::Project>>,
    player: &crate::player_state::SharedPlayerState,
    update: Update,
) {
    match update {
        Update::Duration { source, duration } => {
            let SourceIdentity {
                item_id,
                source_revision,
                scene,
                input_parameters,
            } = source;
            apply_duration(
                project,
                player,
                item_id,
                source_revision,
                &scene,
                &input_parameters,
                duration,
            )
        }
        Update::Parameters {
            source,
            parameters,
            render_is_current,
        } => {
            let SourceIdentity {
                item_id,
                source_revision,
                scene,
                input_parameters,
            } = source;
            apply_parameters(
                project,
                player,
                ParameterUpdate {
                    item_id,
                    source_revision,
                    scene,
                    input_parameters,
                    parameters,
                    render_is_current,
                },
            )
        }
        Update::Error { source, error } => {
            let SourceIdentity {
                item_id,
                source_revision,
                scene,
                input_parameters,
            } = source;
            let current = project
                .borrow()
                .video_item_by_id(item_id)
                .is_some_and(|item| {
                    error_source_matches(item, source_revision, &scene, &input_parameters)
                });
            if current && set_error(item_id, source_revision, scene, input_parameters, error) {
                crate::player_state::refresh_project(
                    player,
                    crate::player_state::ProjectChange {
                        inspector: true,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

fn apply_duration(
    project: &std::rc::Rc<std::cell::RefCell<shrimply_project::project::Project>>,
    player: &crate::player_state::SharedPlayerState,
    item_id: Uuid,
    source_revision: u64,
    scene: &str,
    input_parameters: &HashMap<String, shrimply_project::project::ManimParameterValue>,
    duration: shrimply_math_core::Time,
) {
    let mut project = project.borrow_mut();
    let Some(item) = project.video_item_by_id_mut(item_id) else {
        return;
    };
    if !source_matches(item, source_revision, scene, input_parameters)
        || item.source_duration == duration
    {
        return;
    }
    let previous_natural_end = shrimply_project::project::media_item_natural_end_position(
        item.start,
        item.animation_time_offset,
        item.source_duration,
        item.playback_speed,
        item.repeat_strategy,
    );
    let followed_natural_end = previous_natural_end == Some(item.end);
    item.source_duration = duration;
    if followed_natural_end
        && let Some(end) = shrimply_project::project::media_item_natural_end_position(
            item.start,
            item.animation_time_offset,
            duration,
            item.playback_speed,
            item.repeat_strategy,
        )
    {
        item.end = end;
    }
    shrimply_project::project::commit_edit(&project, "manim-source-duration");
    let duration = project.duration();
    drop(project);
    crate::player_state::refresh_project(
        player,
        crate::player_state::ProjectChange {
            duration: Some(duration),
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

fn apply_parameters(
    project: &std::rc::Rc<std::cell::RefCell<shrimply_project::project::Project>>,
    player: &crate::player_state::SharedPlayerState,
    update: ParameterUpdate,
) {
    let ParameterUpdate {
        item_id,
        source_revision,
        scene,
        input_parameters,
        parameters,
        render_is_current,
    } = update;
    let (changed, reconciled) = {
        let mut project = project.borrow_mut();
        let Some(item) = project.video_item_by_id_mut(item_id) else {
            return;
        };
        if !source_matches(item, source_revision, &scene, &input_parameters) {
            return;
        }
        let shrimply_project::project::VideoItemContent::Manim(manim) = &mut item.content else {
            return;
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
            shrimply_project::project::commit_edit(&project, "reconcile-manim-parameters");
        }
        (changed, reconciled)
    };
    if changed || reconciled {
        crate::player_state::refresh_project(
            player,
            crate::player_state::ProjectChange {
                inspector: true,
                video: reconciled,
                ..Default::default()
            },
        );
    }
}

fn source_matches(
    item: &shrimply_project::project::VideoItem,
    source_revision: u64,
    scene: &str,
    input_parameters: &HashMap<String, shrimply_project::project::ManimParameterValue>,
) -> bool {
    let shrimply_project::project::VideoItemContent::Manim(manim) = &item.content else {
        return false;
    };
    item.file
        .snapshot()
        .is_ok_and(|snapshot| snapshot.revision() == source_revision)
        && manim.scene == scene
        && manim.parameters == *input_parameters
}

fn error_source_matches(
    item: &shrimply_project::project::VideoItem,
    source_revision: u64,
    scene: &str,
    input_parameters: &HashMap<String, shrimply_project::project::ManimParameterValue>,
) -> bool {
    let shrimply_project::project::VideoItemContent::Manim(manim) = &item.content else {
        return false;
    };
    manim.scene == scene
        && manim.parameters == *input_parameters
        && item
            .file
            .snapshot()
            .map_or(true, |snapshot| snapshot.revision() == source_revision)
}

pub fn set_parameters(
    item_id: Uuid,
    source_revision: u64,
    scene: String,
    input_parameters: HashMap<String, shrimply_project::project::ManimParameterValue>,
    parameters: Vec<shrimply_project::project::ManimParameter>,
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
    input_parameters: &HashMap<String, shrimply_project::project::ManimParameterValue>,
) -> Option<Vec<shrimply_project::project::ManimParameter>> {
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
    input_parameters: HashMap<String, shrimply_project::project::ManimParameterValue>,
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
    input_parameters: &HashMap<String, shrimply_project::project::ManimParameterValue>,
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
