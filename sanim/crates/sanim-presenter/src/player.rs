use sanim_core::{Fraction, math};
use sanim_lang::{Engine, FileSources, PageStop, Preparation};
use sanim_render::{Frame, Options, Renderer, Scene};
use shrimply_math_core::{Time, nonnegative_frame_index};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Instant};

struct Playback {
    started: Instant,
    from: Fraction,
    next_stop: usize,
}

pub struct Player {
    renderer: Renderer,
    scene: Scene,
    options: Options,
    stops: Vec<PageStop>,
    reached: Option<usize>,
    playback: Option<Playback>,
    position: Fraction,
    displayed: Option<(Fraction, Arc<Frame>)>,
}

impl Player {
    pub fn open(path: &Path) -> Result<(&str, Self), String> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Sanim filename must be UTF-8")?;
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        let root = path.parent().ok_or("Sanim source directory is missing")?;
        let sources = FileSources::new(root)?;
        let mut engine = Engine::default();
        engine.preparation = Preparation::Demand;
        let prepared = engine
            .prepare(name, &sources, &BTreeMap::new())
            .map_err(|e| e.to_string())?;
        let deck = prepared.presentation().map_err(|e| e.to_string())?;
        let mut renderer = Renderer::default();
        let scene = renderer.prepare(deck.animation)?;
        renderer.validate_geometry(&scene)?;
        let options = Options::for_animation(scene.animation());

        Ok((
            name,
            Self {
                renderer,
                scene,
                options,
                stops: deck.stops,
                reached: None,
                playback: None,
                position: 0.into(),
                displayed: None,
            },
        ))
    }

    pub fn playing(&self) -> bool {
        self.playback.is_some()
    }

    #[cfg(target_os = "macos")]
    pub fn frame_period(&self) -> Result<std::time::Duration, String> {
        Ok(Time {
            seconds: self.scene.animation().frame_time(1)?,
        }
        .as_nonnegative_duration_ceil())
    }

    pub fn next(&mut self, now: Instant) {
        let next_stop = self.reached.map_or(0, |index| index + 1);
        if self.playing() || next_stop >= self.stops.len() {
            return;
        }
        let from = self
            .reached
            .map_or(self.position, |index| self.stops[index].resume);
        self.playback = Some(Playback {
            started: now,
            from,
            next_stop,
        });
    }

    pub fn previous(&mut self) {
        if let Some(index) = self.reached {
            let previous = if self.playback.take().is_some() {
                index
            } else {
                index.saturating_sub(1)
            };
            self.reached = Some(previous);
            self.position = self.stops[previous].at;
        }
    }

    pub fn frame(&mut self, now: Instant) -> Result<Option<Arc<Frame>>, String> {
        if let Some(playback) = &self.playback {
            let elapsed = Time::from_duration(now.saturating_duration_since(playback.started));
            let stop = self.stops[playback.next_stop].at;
            if math::temporal::end(playback.from, elapsed.seconds)? >= stop {
                self.position = stop;
                self.reached = Some(playback.next_stop);
                self.playback = None;
            } else {
                let animation = self.scene.animation();
                let frame = nonnegative_frame_index(elapsed, animation.rate)
                    .ok_or("Sanim playback frame is out of range")?;
                self.position = math::temporal::end(playback.from, animation.frame_time(frame)?)?;
            }
        }
        if self
            .displayed
            .as_ref()
            .is_some_and(|(time, _)| *time == self.position)
        {
            return Ok(None);
        }
        let frame = self
            .renderer
            .render_opaque(&self.scene, self.position, &self.options)?;
        let changed = self
            .displayed
            .as_ref()
            .is_none_or(|(_, old)| !Arc::ptr_eq(old, &frame));
        self.displayed = Some((self.position, frame.clone()));
        Ok(changed.then_some(frame))
    }
}
