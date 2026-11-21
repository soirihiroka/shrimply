use shrimply_math_core::Time;

pub trait TLSnappable {
    fn snap_times(self) -> Vec<Time>;
}

pub struct SnapSources<S> {
    pub snappables: S,
    pub candidates: Vec<Time>,
    pub offsets: Vec<Time>,
    pub frame_step: Time,
    pub distance: Option<Time>,
}

pub struct SnapQuery {
    pub candidates: Vec<Time>,
    pub offsets: Vec<Time>,
    pub distance: Option<Time>,
    pub dynamic_time: Option<Time>,
}

#[derive(Default)]
pub struct SnapRepo {
    times: Vec<Time>,
    candidates: Vec<Time>,
    offsets: Vec<Time>,
    frame_step: Time,
    distance: Option<Time>,
    dynamic_time: Option<Time>,
    nearest_sources: Vec<Box<dyn Fn(Time) -> Option<Time>>>,
}

impl SnapRepo {
    pub fn new<S, N>(sources: SnapSources<S>) -> Self
    where
        S: IntoIterator<Item = N>,
        N: TLSnappable,
    {
        let mut times = sources
            .snappables
            .into_iter()
            .flat_map(TLSnappable::snap_times)
            .collect::<Vec<_>>();
        times.sort_unstable();
        times.dedup();
        let mut candidates = sources.candidates;
        candidates.sort_unstable();
        candidates.dedup();
        let mut offsets = sources.offsets;
        offsets.sort_unstable();
        offsets.dedup();
        Self {
            times,
            candidates,
            offsets,
            frame_step: sources.frame_step,
            distance: sources.distance,
            dynamic_time: None,
            nearest_sources: Vec::new(),
        }
    }

    /// Update moving anchors and interaction offsets without rebuilding fixed edges.
    pub fn update_query(&mut self, mut query: SnapQuery) {
        query.candidates.sort_unstable();
        query.candidates.dedup();
        query.offsets.sort_unstable();
        query.offsets.dedup();
        self.candidates = query.candidates;
        self.offsets = query.offsets;
        self.distance = query.distance;
        self.dynamic_time = query.dynamic_time;
    }

    /// Indexed or periodic sources can answer lazily without materializing their repeats.
    pub fn set_nearest_sources(&mut self, sources: Vec<Box<dyn Fn(Time) -> Option<Time>>>) {
        self.nearest_sources = sources;
    }

    pub fn snap(&self, time: Time) -> Option<Time> {
        let distance = self.distance?;
        let time = time.snapped(self.frame_step);
        self.offsets
            .iter()
            .filter_map(|offset| {
                let at = time.saturating_add(*offset);
                let target = [nearest(&self.times, at), self.dynamic_time]
                    .into_iter()
                    .flatten()
                    .chain(self.nearest_sources.iter().filter_map(|source| source(at)))
                    .min_by_key(|target| (at.abs_diff(*target), *target))?;
                let candidate = target.signed_sub(*offset);
                Some((time.abs_diff(candidate), candidate))
            })
            .chain(
                nearest(&self.candidates, time)
                    .map(|candidate| (time.abs_diff(candidate), candidate)),
            )
            .min()
            .filter(|(candidate_distance, _)| *candidate_distance <= distance)
            .map(|(_, candidate)| candidate.snapped(self.frame_step))
            .or(Some(time))
    }
}

fn nearest(times: &[Time], time: Time) -> Option<Time> {
    let index = times.partition_point(|target| *target < time);
    [
        index.checked_sub(1).map(|index| times[index]),
        times.get(index).copied(),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|target| (time.abs_diff(*target), *target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shrimply_math_core::Fraction;

    #[test]
    fn sanim_event_sources_snap_moving_anchors_without_expanding_source_events() {
        struct Empty;
        impl TLSnappable for Empty {
            fn snap_times(self) -> Vec<Time> {
                Vec::new()
            }
        }
        let mut repository = SnapRepo::new(SnapSources {
            snappables: Vec::<Empty>::new(),
            candidates: Vec::new(),
            offsets: vec![Time::from_seconds(1)],
            frame_step: Time {
                seconds: Fraction::new(1u64, 30u64),
            },
            distance: Some(Time {
                seconds: Fraction::new(1u64, 5u64),
            }),
        });
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        repository.set_nearest_sources(vec![Box::new({
            let calls = calls.clone();
            move |_| {
                calls.set(calls.get() + 1);
                Some(Time::from_seconds(2))
            }
        })]);
        let at = Time {
            seconds: Fraction::new(11u64, 10u64),
        };
        assert_eq!(repository.snap(at), Some(Time::from_seconds(1)));
        assert_eq!(
            calls.get(),
            1,
            "source is queried only for the moving anchor"
        );
        repository.set_nearest_sources(Vec::new());
        assert_eq!(repository.snap(at), Some(at));
    }
}
