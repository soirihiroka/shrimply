use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, mpsc};

use shrimply_project::project::{
    Asset, AssetSnapshot, Color, ManimItem, ManimParameter, ManimParameterControl,
    ManimParameterValue, VideoItem, VideoItemContent,
};
use shrimply_state::player_state;

use crate::{
    ControlKind, InspectorControl, InspectorController, InspectorSection, InspectorTarget,
    NumberSpec, VideoCard,
    item::HeaderAction,
    video::{ReloadKind, VideoCardAction},
};

pub const SCENE_PATH: &str = "/manim/scene";
pub const PARAMETER_PATH_PREFIX: &str = "/manim/parameters/";
const RELOAD_ICON: &str = "view-refresh-symbolic";
const RELOAD_TOOLTIP: &str = "Reload Python source and rebuild Manim scene states";

const LOADING_SCENES: &str = "Loading scenes...";
const SCENE_ERROR: &str = "Could not parse scenes";
const ANTI_ALIASING_VALUES: [(i64, &str); 5] = [
    (0, "Off"),
    (2, "2× MSAA"),
    (4, "4× MSAA"),
    (8, "8× MSAA"),
    (16, "16× MSAA"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct ManimPresentation {
    pub main: VideoCard,
    pub parameters: Option<VideoCard>,
    pub main_reset: ManimReset,
    pub parameters_reset: Option<ManimParametersReset>,
    pub current_scene: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManimReset {
    pub scene: String,
    pub anti_aliasing_key: Option<String>,
    pub commit_name: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManimParametersReset {
    pub keys: Vec<String>,
    pub commit_name: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManimSceneDiscovery {
    pub selected: String,
    pub options: Vec<String>,
    pub changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManimScenes {
    Loading,
    Ready(ManimSceneDiscovery),
    Failed(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SceneCacheKey {
    path: PathBuf,
    revision: u64,
}

struct LoadedScenes {
    key: SceneCacheKey,
    snapshot: AssetSnapshot,
    result: Result<Vec<String>, String>,
}

struct SceneCache {
    results: HashMap<SceneCacheKey, Result<Vec<String>, String>>,
    pending: HashSet<SceneCacheKey>,
    sender: mpsc::Sender<LoadedScenes>,
    receiver: mpsc::Receiver<LoadedScenes>,
}

impl Default for SceneCache {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            results: HashMap::new(),
            pending: HashSet::new(),
            sender,
            receiver,
        }
    }
}

pub fn presentation(item: &VideoItem) -> Option<ManimPresentation> {
    let VideoItemContent::Manim(manim) = &item.content else {
        return None;
    };
    let source_revision = item
        .file
        .snapshot()
        .map_or(0, |snapshot| snapshot.revision());
    let reflected = shrimply_state::manim_status::parameters(
        item.id,
        source_revision,
        &manim.scene,
        &manim.parameters,
    )
    .unwrap_or_default();
    let anti_aliasing = reflected
        .iter()
        .find(|parameter| matches!(parameter.control, ManimParameterControl::AntiAliasing));
    let mut section = InspectorSection::default();
    section.add(scene_control(&item.file, manim));
    if let Some(control) = anti_aliasing.and_then(anti_aliasing_control) {
        section.add(control);
    }
    if let Some(error) = shrimply_state::manim_status::error(
        item.id,
        source_revision,
        &manim.scene,
        &manim.parameters,
    ) {
        section.add(InspectorControl::new(ControlKind::ReadOnly, "", "").value(error));
    }

    let parameters = reflected
        .iter()
        .filter(|parameter| !matches!(parameter.control, ManimParameterControl::AntiAliasing))
        .filter_map(parameter_control)
        .collect::<Vec<_>>();
    let parameter_keys = reflected
        .iter()
        .filter(|parameter| !matches!(parameter.control, ManimParameterControl::AntiAliasing))
        .map(|parameter| parameter.key.clone())
        .collect::<Vec<_>>();
    let parameter_card = (!parameter_keys.is_empty()).then(|| {
        let section = InspectorSection {
            controls: parameters,
        };
        VideoCard::new("manim-parameters", "Parameters", section)
    });

    let main = VideoCard::new("manim", "Manim", section).actions([HeaderAction {
        icon: RELOAD_ICON,
        tooltip: RELOAD_TOOLTIP,
        sensitive: true,
        activate: VideoCardAction::ReloadAsset {
            asset: item.file.path().to_string_lossy().into_owned(),
            kind: ReloadKind::Manim,
        },
    }]);
    Some(ManimPresentation {
        main,
        parameters: parameter_card,
        main_reset: ManimReset {
            scene: ManimItem::default().scene,
            anti_aliasing_key: anti_aliasing.map(|parameter| parameter.key.clone()),
            commit_name: "reset-manim-scene",
        },
        parameters_reset: (!parameter_keys.is_empty()).then_some(ManimParametersReset {
            keys: parameter_keys,
            commit_name: "reset-manim-parameters",
        }),
        current_scene: manim.scene.clone(),
        source: item.file.path().to_string_lossy().into_owned(),
    })
}

pub fn scenes(source: &Asset, current: &str) -> ManimScenes {
    let snapshot = match source.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return ManimScenes::Failed(error),
    };
    let key = SceneCacheKey {
        path: snapshot.path().to_path_buf(),
        revision: snapshot.revision(),
    };
    let cache = scene_cache();
    let mut cache = cache.lock().expect("Manim scene cache mutex poisoned");
    receive_scenes(&mut cache);
    if let Some(result) = cache.results.get(&key) {
        return match result {
            Ok(options) => ManimScenes::Ready(
                resolve_scenes(options.clone(), current)
                    .expect("cached Manim scene list must not be empty"),
            ),
            Err(error) => ManimScenes::Failed(error.clone()),
        };
    }
    if cache.pending.insert(key.clone()) {
        let sender = cache.sender.clone();
        std::thread::spawn(move || {
            let result = shrimply_manim_parser::discover_scenes(snapshot.asset());
            let _ = sender.send(LoadedScenes {
                key,
                snapshot,
                result,
            });
        });
    }
    ManimScenes::Loading
}

pub fn poll_scenes() -> bool {
    let cache = scene_cache();
    receive_scenes(&mut cache.lock().expect("Manim scene cache mutex poisoned"))
}

fn resolve_scenes(options: Vec<String>, current: &str) -> Result<ManimSceneDiscovery, String> {
    let Some(first) = options.first() else {
        return Err("Manim scene discovery returned no scenes".to_string());
    };
    let selected = options
        .iter()
        .find(|scene| scene.as_str() == current)
        .cloned()
        .unwrap_or_else(|| first.clone());
    Ok(ManimSceneDiscovery {
        changed: selected != current,
        selected,
        options,
    })
}

pub fn discovered_scene_control(discovery: &ManimSceneDiscovery) -> InspectorControl {
    crate::selector::selector(
        SCENE_PATH,
        "Scene",
        discovery.selected.clone(),
        discovery
            .options
            .iter()
            .cloned()
            .map(|scene| (scene.clone(), scene)),
    )
    .immediate_commit("manim-scene")
}

pub fn failed_scene_control(error: &str) -> InspectorControl {
    crate::selector::selector(
        SCENE_PATH,
        "Scene",
        SCENE_ERROR,
        [(SCENE_ERROR.to_string(), SCENE_ERROR.to_string())],
    )
    .tooltip(error)
    .sensitive(false)
    .immediate_commit("manim-scene")
}

pub fn reload_source(source: &Asset) -> Result<(), String> {
    let invalidate = shrimply_manim_parser::invalidate_ir_cache(source);
    let dirty = source.mark_dirty();
    match (invalidate, dirty) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(invalidate), Ok(())) => {
            Err(format!("could not invalidate Manim source: {invalidate}"))
        }
        (Ok(()), Err(dirty)) => Err(format!("could not mark Manim source dirty: {dirty}")),
        (Err(invalidate), Err(dirty)) => Err(format!(
            "could not invalidate Manim source: {invalidate}; could not mark it dirty: {dirty}"
        )),
    }
}

pub fn parameter_key(path: &str) -> Option<&str> {
    path.strip_prefix(PARAMETER_PATH_PREFIX)
        .filter(|key| !key.is_empty())
}

impl InspectorController {
    pub fn set_manim_text_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
        commit_name: &str,
    ) -> Option<Result<(), String>> {
        if path == SCENE_PATH {
            return Some(self.set_manim_scene(target, text.to_string(), commit_name));
        }
        let key = parameter_key(path)?;
        Some(self.set_manim_parameter_text(target, key, text, commit_name))
    }

    pub fn set_manim_parameter_text(
        &self,
        target: &InspectorTarget,
        key: &str,
        text: &str,
        commit_name: &str,
    ) -> Result<(), String> {
        let parameter = self.manim_parameter(target, key)?;
        let value = text_value(&parameter.control, text)?;
        let value = validated_value(&parameter, value)?;
        self.store_manim_parameter(target, key, value, commit_name)
    }

    pub fn set_manim_fraction(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: shrimply_math_core::Fraction,
        commit_name: &str,
    ) -> Option<Result<(), String>> {
        let key = parameter_key(path)?;
        Some(self.set_manim_parameter(
            target,
            key,
            ManimParameterValue::Fraction {
                numerator: shrimply_math_core::fraction_numerator(value),
                denominator: shrimply_math_core::fraction_denominator(value),
            },
            commit_name,
        ))
    }

    pub fn set_manim_color(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: Color<u8>,
        commit_name: &str,
    ) -> Option<Result<(), String>> {
        let key = parameter_key(path)?;
        Some(self.set_manim_parameter(target, key, ManimParameterValue::Color(value), commit_name))
    }

    pub fn set_manim_scene(
        &self,
        target: &InspectorTarget,
        scene: String,
        commit_name: &str,
    ) -> Result<(), String> {
        validate_target(target, commit_name)?;
        let mut project = self.project.borrow_mut();
        let manim = manim_mut(&mut project, target)?;
        if manim.scene == scene {
            return Ok(());
        }
        manim.scene = scene;
        manim.parameters.clear();
        shrimply_project::project::commit_edit(&project, commit_name);
        drop(project);
        refresh(&self.player_state, false);
        Ok(())
    }

    pub fn set_manim_parameter(
        &self,
        target: &InspectorTarget,
        key: &str,
        value: ManimParameterValue,
        commit_name: &str,
    ) -> Result<(), String> {
        let definition = self.manim_parameter(target, key)?;
        let value = validated_value(&definition, value)?;
        self.store_manim_parameter(target, key, value, commit_name)
    }

    fn store_manim_parameter(
        &self,
        target: &InspectorTarget,
        key: &str,
        value: ManimParameterValue,
        commit_name: &str,
    ) -> Result<(), String> {
        validate_target(target, commit_name)?;
        let mut project = self.project.borrow_mut();
        let manim = manim_mut(&mut project, target)?;
        if manim.parameters.get(key) == Some(&value) {
            return Ok(());
        }
        manim.parameters.insert(key.to_string(), value);
        shrimply_project::project::commit_edit(&project, commit_name);
        drop(project);
        refresh(&self.player_state, false);
        Ok(())
    }

    pub fn reset_manim(&self, target: &InspectorTarget, reset: &ManimReset) -> Result<(), String> {
        validate_target(target, reset.commit_name)?;
        let mut project = self.project.borrow_mut();
        let manim = manim_mut(&mut project, target)?;
        let scene_changed = manim.scene != reset.scene;
        let anti_aliasing_changed = reset
            .anti_aliasing_key
            .as_ref()
            .is_some_and(|key| manim.parameters.contains_key(key));
        if !scene_changed && !anti_aliasing_changed {
            return Ok(());
        }
        if scene_changed {
            manim.scene.clone_from(&reset.scene);
            manim.parameters.clear();
        } else if let Some(key) = &reset.anti_aliasing_key {
            manim.parameters.remove(key);
        }
        shrimply_project::project::commit_edit(&project, reset.commit_name);
        drop(project);
        refresh(&self.player_state, false);
        Ok(())
    }

    pub fn reset_manim_parameters(
        &self,
        target: &InspectorTarget,
        reset: &ManimParametersReset,
    ) -> Result<(), String> {
        validate_target(target, reset.commit_name)?;
        let mut project = self.project.borrow_mut();
        let manim = manim_mut(&mut project, target)?;
        manim.parameters.retain(|key, _| !reset.keys.contains(key));
        shrimply_project::project::commit_edit(&project, reset.commit_name);
        drop(project);
        refresh(&self.player_state, true);
        Ok(())
    }

    fn manim_parameter(
        &self,
        target: &InspectorTarget,
        key: &str,
    ) -> Result<ManimParameter, String> {
        let InspectorTarget::Item(address) = target else {
            return Err("inspector target is not a video item".to_string());
        };
        let project = self.project.borrow();
        let item = project
            .video_item(address)
            .ok_or_else(|| "Manim item is no longer available".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("inspector target is not a Manim item".to_string());
        };
        let source_revision = item
            .file
            .snapshot()
            .map_or(0, |snapshot| snapshot.revision());
        shrimply_state::manim_status::parameters(
            item.id,
            source_revision,
            &manim.scene,
            &manim.parameters,
        )
        .and_then(|parameters| {
            parameters
                .into_iter()
                .find(|parameter| parameter.key == key)
        })
        .ok_or_else(|| format!("Manim parameter {key:?} is no longer available"))
    }
}

fn loading_scene_control(manim: &ManimItem) -> InspectorControl {
    let placeholder = if manim.scene.is_empty() {
        LOADING_SCENES
    } else {
        &manim.scene
    };
    crate::selector::selector(
        SCENE_PATH,
        "Scene",
        placeholder,
        [(placeholder.to_string(), placeholder.to_string())],
    )
    .sensitive(false)
    .immediate_commit("manim-scene")
}

fn scene_control(source: &Asset, manim: &ManimItem) -> InspectorControl {
    match scenes(source, &manim.scene) {
        ManimScenes::Loading => loading_scene_control(manim),
        ManimScenes::Ready(discovery) => discovered_scene_control(&discovery),
        ManimScenes::Failed(error) => failed_scene_control(&error),
    }
}

fn anti_aliasing_control(parameter: &ManimParameter) -> Option<InspectorControl> {
    let ManimParameterValue::Integer(value) = parameter.value else {
        return None;
    };
    Some(
        crate::selector::selector(
            parameter_path(&parameter.key),
            "Anti-aliasing",
            value.to_string(),
            ANTI_ALIASING_VALUES.map(|(value, label)| (value.to_string(), label.to_string())),
        )
        .immediate_commit("manim-parameter"),
    )
}

fn parameter_control(parameter: &ManimParameter) -> Option<InspectorControl> {
    let path = parameter_path(&parameter.key);
    let control = match (&parameter.control, &parameter.value) {
        (ManimParameterControl::AntiAliasing, _) => return None,
        (
            ManimParameterControl::Integer {
                minimum,
                maximum,
                step,
            },
            ManimParameterValue::Integer(value),
        ) => InspectorControl::new(ControlKind::Number, path, &parameter.label)
            .value(value.to_string())
            .number(NumberSpec {
                minimum: minimum.map_or(NumberSpec::default().minimum, |value| value as f64),
                maximum: maximum.map_or(NumberSpec::default().maximum, |value| value as f64),
                drag_step: *step as f64,
                digits: 0,
                ..NumberSpec::default()
            })
            .integer()
            .immediate_commit("manim-parameter"),
        (
            ManimParameterControl::Float {
                minimum,
                maximum,
                step,
            },
            ManimParameterValue::Float(value),
        ) => InspectorControl::new(ControlKind::Number, path, &parameter.label)
            .value(value.to_string())
            .number(NumberSpec {
                minimum: minimum.unwrap_or(NumberSpec::default().minimum),
                maximum: maximum.unwrap_or(NumberSpec::default().maximum),
                drag_step: *step,
                digits: decimal_digits(*step),
                ..NumberSpec::default()
            })
            .immediate_commit("manim-parameter"),
        (
            ManimParameterControl::Fraction,
            ManimParameterValue::Fraction {
                numerator,
                denominator,
            },
        ) => InspectorControl::new(ControlKind::Fraction, path, &parameter.label)
            .components(vec![numerator.to_string(), denominator.to_string()])
            .number(NumberSpec {
                drag_step: 0.05,
                digits: 2,
                ..NumberSpec::default()
            })
            .immediate_commit("manim-parameter"),
        (ManimParameterControl::Color, ManimParameterValue::Color(value)) => {
            InspectorControl::new(ControlKind::Color, path, &parameter.label)
                .components(
                    [value.r, value.g, value.b, u8::MAX]
                        .map(|value| value.to_string())
                        .to_vec(),
                )
                .without_alpha()
                .immediate_commit("manim-parameter")
        }
        (ManimParameterControl::Option { options }, ManimParameterValue::Option(value)) => {
            crate::selector::selector(
                path,
                &parameter.label,
                value,
                options.iter().cloned().map(|value| (value.clone(), value)),
            )
            .immediate_commit("manim-parameter")
        }
        (ManimParameterControl::Boolean, ManimParameterValue::Boolean(value)) => {
            InspectorControl::new(ControlKind::Boolean, path, &parameter.label)
                .value(value.to_string())
                .immediate_commit("manim-parameter")
        }
        (ManimParameterControl::String, ManimParameterValue::String(value)) => {
            InspectorControl::new(ControlKind::Text, path, &parameter.label)
                .value(value)
                .immediate_commit("manim-parameter")
        }
        _ => {
            tracing::warn!(
                key = %parameter.key,
                "Ignoring mismatched reflected Manim parameter metadata"
            );
            return None;
        }
    };
    Some(control)
}

fn parameter_path(key: &str) -> String {
    format!("{PARAMETER_PATH_PREFIX}{key}")
}

fn decimal_digits(step: f64) -> i32 {
    format!("{step:.8}")
        .trim_end_matches('0')
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len() as i32)
}

fn text_value(control: &ManimParameterControl, text: &str) -> Result<ManimParameterValue, String> {
    match control {
        ManimParameterControl::AntiAliasing | ManimParameterControl::Integer { .. } => text
            .parse()
            .map(ManimParameterValue::Integer)
            .map_err(|_| format!("invalid Manim integer: {text}")),
        ManimParameterControl::Float { .. } => text
            .parse()
            .map(ManimParameterValue::Float)
            .map_err(|_| format!("invalid Manim number: {text}")),
        ManimParameterControl::Option { .. } => Ok(ManimParameterValue::Option(text.to_string())),
        ManimParameterControl::Boolean => text
            .parse()
            .map(ManimParameterValue::Boolean)
            .map_err(|_| "Manim boolean must be true or false".to_string()),
        ManimParameterControl::String => Ok(ManimParameterValue::String(text.to_string())),
        ManimParameterControl::Fraction => {
            Err("Manim fraction must be edited as a fraction".to_string())
        }
        ManimParameterControl::Color => {
            Err("Manim color must be edited with a color picker".to_string())
        }
    }
}

fn validated_value(
    parameter: &ManimParameter,
    value: ManimParameterValue,
) -> Result<ManimParameterValue, String> {
    let valid = match (&parameter.control, &value) {
        (ManimParameterControl::AntiAliasing, ManimParameterValue::Integer(value)) => {
            ANTI_ALIASING_VALUES
                .iter()
                .any(|(allowed, _)| allowed == value)
        }
        (
            ManimParameterControl::Integer {
                minimum, maximum, ..
            },
            ManimParameterValue::Integer(value),
        ) => {
            minimum.is_none_or(|minimum| *value >= minimum)
                && maximum.is_none_or(|maximum| *value <= maximum)
        }
        (
            ManimParameterControl::Float {
                minimum, maximum, ..
            },
            ManimParameterValue::Float(value),
        ) => {
            value.is_finite()
                && minimum.is_none_or(|minimum| *value >= minimum)
                && maximum.is_none_or(|maximum| *value <= maximum)
        }
        (ManimParameterControl::Fraction, ManimParameterValue::Fraction { denominator, .. }) => {
            *denominator != 0
        }
        (ManimParameterControl::Color, ManimParameterValue::Color(_))
        | (ManimParameterControl::Boolean, ManimParameterValue::Boolean(_))
        | (ManimParameterControl::String, ManimParameterValue::String(_)) => true,
        (ManimParameterControl::Option { options }, ManimParameterValue::Option(value)) => {
            options.contains(value)
        }
        _ => false,
    };
    if !valid {
        return Err(format!(
            "value for Manim parameter {:?} does not match its reflected control",
            parameter.key
        ));
    }
    match value {
        ManimParameterValue::Fraction {
            numerator,
            denominator,
        } => {
            let value = shrimply_math_core::fraction_new(numerator, denominator);
            Ok(ManimParameterValue::Fraction {
                numerator: shrimply_math_core::fraction_numerator(value),
                denominator: shrimply_math_core::fraction_denominator(value),
            })
        }
        ManimParameterValue::Color(value) => Ok(ManimParameterValue::Color(Color::new(
            value.r,
            value.g,
            value.b,
            u8::MAX,
        ))),
        value => Ok(value),
    }
}

fn validate_target(target: &InspectorTarget, commit_name: &str) -> Result<(), String> {
    if commit_name.is_empty() {
        return Err("Manim edit has no commit name".to_string());
    }
    if matches!(
        target,
        InspectorTarget::Item(shrimply_project::project::ItemAddress::Video { .. })
    ) {
        Ok(())
    } else {
        Err("inspector target is not a video item".to_string())
    }
}

fn manim_mut<'a>(
    project: &'a mut shrimply_project::project::Project,
    target: &InspectorTarget,
) -> Result<&'a mut ManimItem, String> {
    let InspectorTarget::Item(address) = target else {
        return Err("inspector target is not a video item".to_string());
    };
    let item = project
        .video_item_mut(address)
        .ok_or_else(|| "Manim item is no longer available".to_string())?;
    let VideoItemContent::Manim(manim) = &mut item.content else {
        return Err("inspector target is not a Manim item".to_string());
    };
    Ok(manim)
}

fn refresh(player: &shrimply_state::player_state::SharedPlayerState, inspector: bool) {
    player_state::refresh_project(
        player,
        player_state::ProjectChange {
            video: true,
            inspector,
            ..player_state::ProjectChange::default()
        },
    );
}

fn receive_scenes(cache: &mut SceneCache) -> bool {
    let mut changed = false;
    while let Ok(loaded) = cache.receiver.try_recv() {
        cache.pending.remove(&loaded.key);
        changed = true;
        if !loaded.snapshot.is_current() {
            continue;
        }
        cache
            .results
            .retain(|key, _| key.path != loaded.key.path || key == &loaded.key);
        cache.results.insert(loaded.key, loaded.result);
    }
    changed
}

fn scene_cache() -> &'static Mutex<SceneCache> {
    static CACHE: OnceLock<Mutex<SceneCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SceneCache::default()))
}
