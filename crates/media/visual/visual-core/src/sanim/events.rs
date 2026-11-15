use super::*;
use shrimply_project_document::source_events::{self, ClipContext, Events};
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct Publication(Mutex<Option<Configuration>>);
struct Configuration {
    clip: ClipContext,
    inputs: u64,
    fingerprint: Option<String>,
}
impl Publication {
    pub fn configure(&self, item: &VideoItem, canvas: CanvasSize, inputs: u64) {
        let mut state = self.0.lock().expect("Sanim event publication poisoned");
        if state
            .as_ref()
            .is_none_or(|state| state.inputs != inputs || !state.clip.matches(item, canvas))
        {
            *state = Some(Configuration {
                clip: ClipContext::for_item(item, canvas)
                    .expect("Sanim event publication requires a Sanim clip"),
                inputs,
                fingerprint: None,
            });
        }
    }
    pub fn update(&self, snapshot: &sanim_runtime::Snapshot) {
        if snapshot.preparing
            || snapshot.diagnostic.is_some()
            || snapshot.requested_inputs != snapshot.applied_inputs
        {
            return;
        }
        let Some(prepared) = &snapshot.prepared else {
            return;
        };
        let publication = {
            let mut state = self.0.lock().expect("Sanim event publication poisoned");
            let Some(state) = state.as_mut() else { return };
            if state.inputs != snapshot.applied_inputs
                || state.fingerprint.as_ref() == Some(&prepared.fingerprint)
            {
                return;
            }
            state.fingerprint = Some(prepared.fingerprint.clone());
            state.clip.clone()
        };
        source_events::publish_clip(
            &publication,
            Events {
                duration: Time {
                    seconds: prepared.animation.duration,
                },
                index: prepared.time_map.event_index(),
            },
        );
    }
}

#[cfg(test)]
mod tests;
