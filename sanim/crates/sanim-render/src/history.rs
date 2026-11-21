//! Reuse held element snapshots and Skia pictures independently of moving siblings.
use crate::{
    FrameState, Options, RecordKey, Recording, Renderer, Scene, StateKey, draw, state_bytes,
};
use sanim_core::{Element, Fraction, Reveal, Value, math::to_f64};
use skia_safe::{Canvas, Rect};
use std::sync::Arc;

impl Recording {
    pub(super) fn bytes(&self) -> usize {
        self.vector_bytes() + self.resources.retained_bytes()
    }
    fn vector_bytes(&self) -> usize {
        self.picture.approximate_bytes_used() + std::mem::size_of::<Self>() + self.nested_bytes
    }
}

impl Renderer {
    fn layer_key(scene: &Scene, key: &StateKey, root: usize) -> Result<Option<StateKey>, String> {
        Ok(scene.layer_times[root]
            .constant_time(key.time)?
            .map(|time| {
                let mut key = key.clone();
                key.root = Some(root);
                key.time = time;
                key
            }))
    }

    pub(super) fn layout_history(
        &mut self,
        scene: &Scene,
        key: &StateKey,
        options: &Options,
    ) -> Result<(Vec<Element>, Vec<usize>), String> {
        let mut keys = Vec::with_capacity(scene.animation.layers.len());
        let mut cached = Vec::with_capacity(scene.animation.layers.len());
        for (root, layer) in scene.animation.layers.iter().enumerate() {
            let layer_key = if layer.visible_at(key.time)? {
                Self::layer_key(scene, key, root)?
            } else {
                None
            };
            let snapshot = layer_key.as_ref().and_then(|key| self.states.get(key));
            self.layer_state_hits += u64::from(snapshot.is_some());
            keys.push(layer_key);
            cached.push(snapshot);
        }
        let limits = sanim_data::Limits::default();
        let mut generated_bytes = limits.max_bytes;
        let mut generated_items = limits.max_values;
        let mut fonts = sanim_core::value_budget::Budget::default();
        let mut roots = Vec::new();
        let elements = scene.animation.layout_with_reuse(
            key.time,
            options.logical_width,
            options.logical_height,
            |element, time| {
                let pending = sanim_data::requires_resolution(element);
                sanim_data::resolve_pending(element, time)?;
                if pending {
                    sanim_core::value_budget::charge_with_budget(
                        &Value::Element(Arc::new(element.clone())),
                        &mut generated_bytes,
                        &mut generated_items,
                        limits.max_depth,
                        &mut fonts,
                    )
                    .map_err(|error| {
                        format!("native frame geometry for '{}': {error}", element.id)
                    })?;
                }
                Ok(())
            },
            |root, _| {
                roots.push(root);
                cached[root].as_ref().map(|state| state.elements[0].clone())
            },
        )?;
        for (&root, element) in roots.iter().zip(&elements) {
            if cached[root].is_some() {
                continue;
            }
            self.layers_computed += 1;
            if let Some(key) = keys[root].take() {
                let elements = vec![element.clone()];
                let bytes = state_bytes(&elements, limits.max_depth)?;
                self.states.insert(
                    key,
                    Arc::new(FrameState {
                        elements,
                        roots: vec![root],
                        compute_micros: 0,
                    }),
                    bytes,
                );
            }
        }
        Ok((elements, roots))
    }

    fn record_element(
        &mut self,
        element: &Element,
        key: RecordKey,
        options: &Options,
    ) -> Result<Arc<Recording>, String> {
        if let Some(recording) = self.recordings.get(&key) {
            self.layer_recording_hits += 1;
            return Ok(recording);
        }
        let mut resources = draw::RecordingOutput::default();
        let mut work = 0;
        let mut recording = Recording::capture(options.width, options.height, |canvas| {
            scale(canvas, options)?;
            let mut drawing = draw::Drawing {
                renderer: self,
                pixels: options.width as usize * options.height as usize,
                work: options.max_work,
                output: draw::RecordingOutput::default(),
            };
            draw::element_draw(canvas, element, Reveal::default(), &mut drawing)?;
            work = options.max_work - drawing.work;
            resources = drawing.output;
            Ok(())
        })?;
        recording.work = work;
        recording.resources = resources;
        let bytes = recording.bytes();
        let recording = Arc::new(recording);
        self.recordings.insert(key, recording.clone(), bytes);
        self.layers_recorded += 1;
        Ok(recording)
    }

    pub(super) fn draw_history(
        &mut self,
        canvas: &Canvas,
        scene: &Scene,
        state: &FrameState,
        key: &RecordKey,
        options: &Options,
    ) -> Result<(usize, draw::RecordingOutput, usize), String> {
        let save = canvas.save();
        scale(canvas, options)?;
        let background = draw::paint(&scene.animation.background, Fraction::from(1))?;
        canvas.draw_rect(
            Rect::from_wh(
                to_f64(options.logical_width)? as f32,
                to_f64(options.logical_height)? as f32,
            ),
            &background,
        );
        canvas.restore_to_count(save);
        let mut remaining = options.max_work;
        let mut nested_bytes = 0usize;
        let mut resources = draw::RecordingOutput::default();
        for (&root, element) in state.roots.iter().zip(&state.elements) {
            if let Some(layer_key) = Self::layer_key(scene, &key.state, root)? {
                let mut record_key = key.clone();
                record_key.state = layer_key;
                let recording = self.record_element(element, record_key, options)?;
                remaining = remaining
                    .checked_sub(recording.work)
                    .ok_or("render work budget exceeded")?;
                resources.merge(&recording.resources)?;
                nested_bytes = nested_bytes
                    .checked_add(recording.vector_bytes())
                    .ok_or("nested picture byte budget overflow")?;
                recording.draw(canvas);
            } else {
                let save = canvas.save();
                scale(canvas, options)?;
                let mut drawing = draw::Drawing {
                    renderer: self,
                    pixels: options.width as usize * options.height as usize,
                    work: remaining,
                    output: resources,
                };
                draw::element_draw(canvas, element, Reveal::default(), &mut drawing)?;
                remaining = drawing.work;
                resources = drawing.output;
                canvas.restore_to_count(save);
            }
        }
        Ok((options.max_work - remaining, resources, nested_bytes))
    }
}

fn scale(canvas: &Canvas, options: &Options) -> Result<(), String> {
    canvas.scale((
        options.width as f32 / to_f64(options.logical_width)? as f32,
        options.height as f32 / to_f64(options.logical_height)? as f32,
    ));
    Ok(())
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
