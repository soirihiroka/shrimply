use hashbrown::HashSet;
use std::cell::RefCell;
use std::rc::Rc;

use shrimply_audio_modifiers::AudioModifier;
use shrimply_project_document::project::{
    AudioItem, AudioSource, ItemAddress, ItemKind, Project, VideoItem, VideoItemContent,
    VisualModifier, ensure_alpha_mask_ids,
};
use shrimply_visual_modifiers::{
    ModifierEffect, ModifierLifecycle, ModifierModel as _, ModifierState, VisualKind,
};
use uuid::Uuid;

pub type SharedClipboard = Rc<RefCell<Clipboard>>;

#[derive(Default)]
pub struct Clipboard {
    payload: Option<Payload>,
}

#[derive(Clone)]
enum Payload {
    Video(Box<VideoItem>),
    Audio(Box<AudioItem>),
    VisualModifier(Box<VisualModifier>),
    AudioModifier(AudioModifier),
}

#[derive(Clone, Copy, Default)]
pub struct PasteResult {
    pub changed: bool,
    pub changed_items: usize,
    pub modifiers_added: usize,
    pub video: bool,
    pub audio: bool,
    pub audio_waveforms: bool,
    pub audio_beats: bool,
    pub stabilization: bool,
}

pub fn new_clipboard() -> SharedClipboard {
    Rc::new(RefCell::new(Clipboard::default()))
}

impl Clipboard {
    pub fn clear(&mut self) {
        self.payload = None;
    }

    pub fn copy_item(&mut self, project: &Project, address: &ItemAddress) -> bool {
        self.payload = match address.kind() {
            ItemKind::Video => project
                .video_item(address)
                .cloned()
                .map(Box::new)
                .map(Payload::Video),
            ItemKind::Audio => project
                .audio_item(address)
                .cloned()
                .map(Box::new)
                .map(Payload::Audio),
            ItemKind::Comment | ItemKind::Caption => None,
        };
        self.payload.is_some()
    }

    pub fn copy_visual_modifier(&mut self, modifier: &VisualModifier) {
        self.payload = Some(Payload::VisualModifier(Box::new(modifier.clone())));
    }

    pub fn copy_audio_modifier(&mut self, modifier: &AudioModifier) {
        self.payload = Some(Payload::AudioModifier(modifier.clone()));
    }

    pub fn can_replace_properties(&self, project: &Project, targets: &[ItemAddress]) -> bool {
        targets
            .iter()
            .any(|target| match (&self.payload, target.kind()) {
                (Some(Payload::Video(_)), ItemKind::Video) => project.video_item(target).is_some(),
                (Some(Payload::Audio(_)), ItemKind::Audio) => project.audio_item(target).is_some(),
                _ => false,
            })
    }

    pub fn can_append_modifiers(&self, project: &Project, targets: &[ItemAddress]) -> bool {
        targets
            .iter()
            .any(|target| match (&self.payload, target.kind()) {
                (Some(Payload::Video(source)), ItemKind::Video) => project
                    .video_item(target)
                    .and_then(|item| item.modifier_output_state().ok())
                    .is_some_and(|state| {
                        !adapt_visual_modifiers(state, &source.modifiers).is_empty()
                    }),
                (Some(Payload::VisualModifier(source)), ItemKind::Video) => project
                    .video_item(target)
                    .and_then(|item| item.modifier_output_state().ok())
                    .is_some_and(|state| {
                        !adapt_visual_modifiers(state, std::slice::from_ref(source.as_ref()))
                            .is_empty()
                    }),
                (Some(Payload::Audio(source)), ItemKind::Audio) => {
                    project.audio_item(target).is_some() && !source.modifiers.is_empty()
                }
                (Some(Payload::AudioModifier(_)), ItemKind::Audio) => {
                    project.audio_item(target).is_some()
                }
                _ => false,
            })
    }

    pub fn replace_properties(
        &self,
        project: &mut Project,
        targets: &[ItemAddress],
    ) -> PasteResult {
        let mut result = PasteResult::default();
        for target in targets {
            match &self.payload {
                Some(Payload::Video(source)) if target.kind() == ItemKind::Video => {
                    let mut source = source.as_ref().clone();
                    Project::regenerate_video_property_ids(&mut source);
                    let Some(target) = project.video_item_mut(target) else {
                        continue;
                    };
                    for adapter in VISUAL_PROPERTY_ADAPTERS {
                        adapter(&source, target);
                    }
                    result.changed = true;
                    result.changed_items += 1;
                    result.video = true;
                    result.stabilization |= target.stabilize_video;
                }
                Some(Payload::Audio(source)) if target.kind() == ItemKind::Audio => {
                    let mut source = source.clone();
                    Project::regenerate_audio_property_ids(&mut source);
                    let Some(target) = project.audio_item_mut(target) else {
                        continue;
                    };
                    for adapter in AUDIO_PROPERTY_ADAPTERS {
                        adapter(&source, target);
                    }
                    result.changed = true;
                    result.changed_items += 1;
                    result.audio = true;
                    result.audio_waveforms = true;
                    result.audio_beats |= target.beat_detection;
                }
                _ => {}
            }
        }
        result
    }

    pub fn append_modifiers(&self, project: &mut Project, targets: &[ItemAddress]) -> PasteResult {
        let mut result = PasteResult::default();
        for target in targets {
            match (&self.payload, target.kind()) {
                (Some(Payload::Video(source)), ItemKind::Video) => {
                    append_visual_modifiers(project, target, &source.modifiers, &mut result);
                }
                (Some(Payload::VisualModifier(source)), ItemKind::Video) => {
                    append_visual_modifiers(
                        project,
                        target,
                        std::slice::from_ref(source.as_ref()),
                        &mut result,
                    );
                }
                (Some(Payload::Audio(source)), ItemKind::Audio) => {
                    append_audio_modifiers(project, target, &source.modifiers, &mut result);
                }
                (Some(Payload::AudioModifier(source)), ItemKind::Audio) => {
                    append_audio_modifiers(
                        project,
                        target,
                        std::slice::from_ref(source),
                        &mut result,
                    );
                }
                _ => {}
            }
        }
        result
    }
}

type VisualItemAdapter = fn(&VideoItem, &mut VideoItem);
type AudioPropertyAdapter = fn(&AudioItem, &mut AudioItem);

const VISUAL_PROPERTY_ADAPTERS: &[VisualItemAdapter] = &[
    adapt_visual_common,
    adapt_visual_playback,
    adapt_stabilization,
    adapt_visual_transform,
    adapt_visual_renderer,
    adapt_source_properties,
    replace_visual_modifiers,
];

const AUDIO_PROPERTY_ADAPTERS: &[AudioPropertyAdapter] = &[
    adapt_audio_common,
    replace_audio_generator,
    replace_audio_modifiers,
];

fn adapt_visual_common(source: &VideoItem, target: &mut VideoItem) {
    target.repeat_strategy = source.repeat_strategy;
    if supports_motion_blur(source) && supports_motion_blur(target) {
        target.motion_blur = source.motion_blur;
    }
    target.visibility = source.visibility.clone();
    target.compositing = source.compositing.clone();
}

fn supports_motion_blur(item: &VideoItem) -> bool {
    !matches!(
        item.content,
        VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
    )
}

fn adapt_visual_playback(source: &VideoItem, target: &mut VideoItem) {
    if supports_speed(source) && supports_speed(target) {
        target.playback_speed = source.playback_speed;
    }
    if supports_frame_rate(source) && supports_frame_rate(target) {
        target.playback_fps = source.playback_fps;
    }
}

fn supports_speed(item: &VideoItem) -> bool {
    matches!(
        item.content,
        VideoItemContent::Media
            | VideoItemContent::Image
            | VideoItemContent::Gif
            | VideoItemContent::Svg
            | VideoItemContent::Pdf(_)
            | VideoItemContent::Manim(_)
            | VideoItemContent::Sanim(_)
            | VideoItemContent::Blender(_)
            | VideoItemContent::LayeredImage(_)
            | VideoItemContent::FoldedSequence(_)
    ) && !item.is_static_visual_media()
}

fn supports_frame_rate(item: &VideoItem) -> bool {
    !matches!(
        item.content,
        VideoItemContent::Media | VideoItemContent::Gif
    )
}

fn adapt_stabilization(source: &VideoItem, target: &mut VideoItem) {
    if !source.is_video_media()
        || !target.is_video_media()
        || source.alpha_mask_video.is_some()
        || target.alpha_mask_video.is_some()
    {
        return;
    }
    target.stabilize_video = source.stabilize_video;
    target.stabilization_method = source.stabilization_method;
    target.stabilization_crop_ratio = source.stabilization_crop_ratio;
    target.stabilization_first_derivative_weight = source.stabilization_first_derivative_weight;
    target.stabilization_second_derivative_weight = source.stabilization_second_derivative_weight;
    target.stabilization_third_derivative_weight = source.stabilization_third_derivative_weight;
    target.mesh_flow_rows = source.mesh_flow_rows;
    target.mesh_flow_columns = source.mesh_flow_columns;
    target.mesh_flow_smoothing_radius = source.mesh_flow_smoothing_radius;
    target.mesh_flow_iterations = source.mesh_flow_iterations;
    target.mesh_flow_adaptive_weights = source.mesh_flow_adaptive_weights;
}

fn adapt_visual_transform(source: &VideoItem, target: &mut VideoItem) {
    if supports_transform(source) && supports_transform(target) {
        target.transform = source.transform.clone();
    }
}

fn supports_transform(item: &VideoItem) -> bool {
    !matches!(
        item.content,
        VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_) | VideoItemContent::Manim(_)
    )
}

fn adapt_visual_renderer(source: &VideoItem, target: &mut VideoItem) {
    match (source.source_visual_kind(), target.source_visual_kind()) {
        (VisualKind::Raster, VisualKind::Raster) => {
            target.sample_method = source.sample_method.clone();
        }
        (VisualKind::Vector, VisualKind::Vector) => {
            target.skia_drawing_strategy = source.skia_drawing_strategy;
        }
        _ => {}
    }
}

fn adapt_source_properties(source: &VideoItem, target: &mut VideoItem) {
    match (&source.content, &mut target.content) {
        (VideoItemContent::Svg, VideoItemContent::Svg) => {
            target.svg_color_overrides = source.svg_color_overrides.clone();
        }
        (VideoItemContent::LayeredImage(source), VideoItemContent::LayeredImage(target)) => {
            for target_layer in &mut target.layers {
                let source_layer = source
                    .layers
                    .iter()
                    .find(|source_layer| source_layer.path == target_layer.path);
                if let Some(source_layer) = source_layer {
                    target_layer.visibility = source_layer.visibility.clone();
                }
            }
        }
        (VideoItemContent::Text(source), VideoItemContent::Text(target)) => {
            let text = target.text.clone();
            **target = source.as_ref().clone();
            target.text = text;
        }
        (VideoItemContent::Shape(source), VideoItemContent::Shape(target)) => {
            let shape = target.shape.clone();
            **target = source.as_ref().clone();
            target.shape = shape;
        }
        (VideoItemContent::Paint(source), VideoItemContent::Paint(target)) => {
            let revision = target.revision;
            let drawing = target.drawing.clone();
            **target = source.as_ref().clone();
            target.revision = revision;
            target.drawing = drawing;
        }
        (VideoItemContent::Text(source), VideoItemContent::Shape(target)) => {
            target.fill = source.color.clone();
            target.outline_color = source.outline_color.clone();
            target.outline_width = source.outline_width.clone();
            target.shadow_color = source.shadow_color.clone();
            target.shadow_distance = source.shadow_distance.clone();
            target.shadow_direction_degrees = source.shadow_direction_degrees.clone();
            target.shadow_width = source.shadow_width.clone();
            target.shadow_blur = source.shadow_blur.clone();
        }
        (VideoItemContent::Shape(source), VideoItemContent::Text(target)) => {
            target.color = source.fill.clone();
            target.outline_color = source.outline_color.clone();
            target.outline_width = source.outline_width.clone();
            target.shadow_color = source.shadow_color.clone();
            target.shadow_distance = source.shadow_distance.clone();
            target.shadow_direction_degrees = source.shadow_direction_degrees.clone();
            target.shadow_width = source.shadow_width.clone();
            target.shadow_blur = source.shadow_blur.clone();
        }
        (VideoItemContent::Obj(source), VideoItemContent::Obj(target)) => {
            **target = source.as_ref().clone();
        }
        (VideoItemContent::Gaussian(source), VideoItemContent::Gaussian(target)) => {
            **target = source.as_ref().clone();
        }
        (VideoItemContent::Obj(source), VideoItemContent::Gaussian(target)) => {
            target.model = source.model.clone();
            target.camera.source = source.camera.source.clone();
            target.camera.projection = source.camera.projection;
            target.camera.position = source.camera.position.clone();
            target.camera.rotation_degrees = source.camera.rotation_degrees.clone();
            target.camera.vertical_fov_degrees = source.camera.vertical_fov_degrees.clone();
            target.camera.orthographic_height = source.camera.orthographic_height.clone();
            target.camera.focus_distance = source.camera.focus_distance.clone();
            target.camera.f_stop = source.camera.f_stop.clone();
            target.camera.exposure_ev = source.camera.exposure_ev.clone();
        }
        (VideoItemContent::Gaussian(source), VideoItemContent::Obj(target)) => {
            target.model = source.model.clone();
            target.camera.source = source.camera.source.clone();
            target.camera.projection = source.camera.projection;
            target.camera.position = source.camera.position.clone();
            target.camera.rotation_degrees = source.camera.rotation_degrees.clone();
            target.camera.vertical_fov_degrees = source.camera.vertical_fov_degrees.clone();
            target.camera.orthographic_height = source.camera.orthographic_height.clone();
            target.camera.focus_distance = source.camera.focus_distance.clone();
            target.camera.f_stop = source.camera.f_stop.clone();
            target.camera.exposure_ev = source.camera.exposure_ev.clone();
        }
        _ => {}
    }
}

fn replace_visual_modifiers(source: &VideoItem, target: &mut VideoItem) {
    let state = target
        .modifier_output_state_for(&[])
        .expect("an empty modifier chain is valid");
    target.modifiers = adapt_visual_modifiers(state, &source.modifiers);
    if target.modifier_output_kind().ok() != Some(VisualKind::Raster) {
        target.compositing.alpha_mask = None;
    }
}

fn adapt_audio_common(source: &AudioItem, target: &mut AudioItem) {
    target.enabled = source.enabled;
    target.gain = source.gain.clone();
    target.playback_speed = source.playback_speed;
    target.repeat_strategy = source.repeat_strategy;
    target.speed_method = source.speed_method;
    if matches!(&source.source, AudioSource::Media | AudioSource::Tts(_))
        && matches!(&target.source, AudioSource::Media | AudioSource::Tts(_))
    {
        target.beat_detection = source.beat_detection;
    }
}

fn replace_audio_modifiers(source: &AudioItem, target: &mut AudioItem) {
    target.modifiers = source.modifiers.clone();
}

fn replace_audio_generator(source: &AudioItem, target: &mut AudioItem) {
    if let (AudioSource::Generator(source), AudioSource::Generator(target)) =
        (&source.source, &mut target.source)
    {
        **target = source.as_ref().clone();
    }
}

fn adapt_visual_modifiers(
    initial: ModifierState,
    source: &[VisualModifier],
) -> Vec<VisualModifier> {
    let mut lifecycle = ModifierLifecycle::new(initial);
    source
        .iter()
        .enumerate()
        .filter_map(|(index, modifier)| {
            let effect = modifier.effect.adapted_for(lifecycle.state())?;
            lifecycle
                .apply(index, modifier.enabled, &effect)
                .expect("adapted modifier satisfies its lifecycle contract");
            let mut modifier = modifier.clone();
            modifier.effect = effect;
            if !matches!(modifier.effect, ModifierEffect::Raster(_)) {
                modifier.alpha_mask = None;
            }
            Some(modifier)
        })
        .collect()
}

fn append_visual_modifiers(
    project: &mut Project,
    target: &ItemAddress,
    source: &[VisualModifier],
    result: &mut PasteResult,
) {
    let Some(item) = project.video_item_mut(target) else {
        return;
    };
    let Some(state) = item.modifier_output_state().ok() else {
        return;
    };
    let mut source = source.to_vec();
    freshen_visual_modifiers(&mut source);
    let modifiers = adapt_visual_modifiers(state, &source);
    if modifiers.is_empty() {
        return;
    }
    result.modifiers_added += modifiers.len();
    item.modifiers.extend(modifiers);
    result.changed = true;
    result.changed_items += 1;
    result.video = true;
}

fn append_audio_modifiers(
    project: &mut Project,
    target: &ItemAddress,
    source: &[AudioModifier],
    result: &mut PasteResult,
) {
    if source.is_empty() {
        return;
    }
    let Some(item) = project.audio_item_mut(target) else {
        return;
    };
    let mut source = source.to_vec();
    freshen_audio_modifiers(&mut source);
    result.modifiers_added += source.len();
    item.modifiers.extend(source);
    result.changed = true;
    result.changed_items += 1;
    result.audio = true;
    result.audio_waveforms = true;
}

fn freshen_visual_modifiers(modifiers: &mut [VisualModifier]) {
    for modifier in modifiers {
        let mut seen = HashSet::new();
        modifier.effect.clone().ensure_ids(&mut seen);
        if let Some(mut mask) = modifier.alpha_mask.clone() {
            ensure_alpha_mask_ids(&mut mask, &mut seen);
        }
        modifier.id = Uuid::new_v4();
        modifier.effect.ensure_ids(&mut seen);
        if let Some(mask) = &mut modifier.alpha_mask {
            ensure_alpha_mask_ids(mask, &mut seen);
        }
    }
}

fn freshen_audio_modifiers(modifiers: &mut [AudioModifier]) {
    for modifier in modifiers {
        let mut seen = HashSet::new();
        modifier.effect.clone().ensure_ids(&mut seen);
        modifier.id = Uuid::new_v4();
        modifier.effect.ensure_ids(&mut seen);
    }
}
