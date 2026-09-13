mod activity;
mod retirement;

use activity::DecoderActivity;
pub use activity::DecoderActivityGuard;

use std::hash::Hash;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use hashbrown::HashMap;
use retirement::Retirement;
use shrimply_math_core::Time;

const FOREGROUND_RECONFIGURE_DELAY: Duration = Duration::from_secs(1);

struct TimeEntry<S, D> {
    source: S,
    decoder: D,
    activity: DecoderActivity,
    last_access: Instant,
    last_foreground_use: Option<Instant>,
}

pub trait TemporalDecoder<S>: Sized {
    type Error;
    type Context;
    type Metadata;
    type Current;
    type Request;
    type Response;

    fn create(source: &S, context: &Self::Context) -> Result<Self, Self::Error>;
    fn try_create(source: &S, context: &Self::Context) -> Result<Option<Self>, Self::Error> {
        Self::create(source, context).map(Some)
    }
    fn metadata(&self) -> Self::Metadata;
    fn current(&self) -> Option<Self::Current>;
    fn request(
        &mut self,
        position: Time,
        request: Self::Request,
        activity: DecoderActivityGuard,
    ) -> Result<Self::Response, Self::Error>;
}

pub struct TemporalDecoderPool<S, O, D>
where
    S: Send,
    O: Send,
    D: TemporalDecoder<S> + Send,
    D::Context: Send,
{
    state: Mutex<PoolState<S, O, D>>,
}

struct PoolState<S, O, D>
where
    D: TemporalDecoder<S>,
{
    decoders: HashMap<O, TimeEntry<S, D>>,
    orphaned: Vec<TimeEntry<S, D>>,
    preparing: usize,
    maximum: usize,
    context: D::Context,
    retirement: Retirement<D>,
}

impl<S, O, D> TemporalDecoderPool<S, O, D>
where
    S: Clone + Eq + Send,
    O: Clone + Eq + Hash + Send,
    D: TemporalDecoder<S> + Send + 'static,
    D::Context: Clone + Send,
{
    pub fn new(maximum: usize, context: D::Context) -> Self {
        assert!(
            maximum > 0,
            "a decoder pool must allow at least one decoder"
        );
        Self {
            state: Mutex::new(PoolState {
                decoders: HashMap::new(),
                orphaned: Vec::new(),
                preparing: 0,
                maximum,
                context,
                retirement: Retirement::new(),
            }),
        }
    }

    pub fn set_maximum(&self, maximum: usize) {
        self.state
            .lock()
            .expect("decoder pool mutex poisoned")
            .set_maximum(maximum);
    }

    /// Submits required work, temporarily growing beyond the configured maximum when every
    /// decoder is busy. The maximum is a best-effort scheduling limit, not a correctness limit.
    pub fn request(
        &self,
        source: S,
        owner: O,
        handoff_from: Option<O>,
        position: Time,
        foreground: bool,
        request: D::Request,
    ) -> Result<D::Response, D::Error> {
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        state.ensure_decoder(&source, &owner, handoff_from.as_ref(), foreground, true)?;
        let entry = state
            .decoders
            .get_mut(&owner)
            .expect("decoder owner disappeared after allocation");
        let activity = entry.activity.begin();
        entry.decoder.request(position, request, activity)
    }

    /// Submits work to the decoder owned by this item, transfers an explicitly supplied
    /// predecessor, or creates a decoder asynchronously. Foreground work may exceed the soft
    /// limit; speculative work must leave room for retiring and preparing decoders.
    pub fn try_request(
        &self,
        source: S,
        owner: O,
        handoff_from: Option<O>,
        position: Time,
        foreground: bool,
        request: D::Request,
    ) -> Result<Option<D::Response>, D::Error> {
        let mut state = self
            .state
            .lock()
            .expect("temporal decoder pool mutex poisoned");
        if !state.ensure_decoder(&source, &owner, handoff_from.as_ref(), foreground, false)? {
            return Ok(None);
        }
        let entry = state
            .decoders
            .get_mut(&owner)
            .expect("decoder owner disappeared after allocation");
        let activity = entry.activity.begin();
        entry.decoder.request(position, request, activity).map(Some)
    }

    pub fn current_for_owner(&self, source: &S, owner: &O) -> Option<D::Current> {
        self.state
            .lock()
            .expect("temporal decoder pool mutex poisoned")
            .decoders
            .get(owner)
            .filter(|entry| &entry.source == source)
            .and_then(|entry| entry.decoder.current())
    }

    pub fn metadata_for_owner(&self, source: &S, owner: &O) -> Option<D::Metadata> {
        self.state
            .lock()
            .expect("temporal decoder pool mutex poisoned")
            .decoders
            .get(owner)
            .filter(|entry| &entry.source == source)
            .map(|entry| entry.decoder.metadata())
    }

    pub fn contains_owner(&self, source: &S, owner: &O) -> bool {
        self.state
            .lock()
            .expect("decoder pool mutex poisoned")
            .decoders
            .get(owner)
            .is_some_and(|entry| &entry.source == source)
    }

    pub fn touch_foreground(&self, source: &S, owner: &O) {
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        let Some(entry) = state
            .decoders
            .get_mut(owner)
            .filter(|entry| &entry.source == source)
        else {
            return;
        };
        let now = Instant::now();
        entry.last_access = now;
        entry.last_foreground_use = Some(now);
    }

    pub fn prepare(&self, source: S, owner: O, reserved: usize) -> Result<bool, D::Error> {
        let context = {
            let mut state = self.state.lock().expect("decoder pool mutex poisoned");
            state.remove_incompatible_owner(&source, &owner);
            if state.has_owner(&source, &owner) {
                return Ok(true);
            }
            let limit = state.maximum.saturating_sub(reserved);
            if limit == 0 {
                return Ok(false);
            }
            state.trim_reconfigurable_to_limit(limit.saturating_sub(1));
            if state.len() + state.preparing >= limit {
                return Ok(false);
            }
            state.preparing += 1;
            state.context.clone()
        };
        let decoder = match D::try_create(&source, &context) {
            Ok(Some(decoder)) => decoder,
            Ok(None) => {
                self.state
                    .lock()
                    .expect("decoder pool mutex poisoned")
                    .preparing -= 1;
                return Ok(false);
            }
            Err(error) => {
                self.state
                    .lock()
                    .expect("decoder pool mutex poisoned")
                    .preparing -= 1;
                return Err(error);
            }
        };
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        state.preparing -= 1;
        state.remove_incompatible_owner(&source, &owner);
        if state.has_owner(&source, &owner) {
            state.retirement.retire(decoder);
            return Ok(true);
        }
        let limit = state.maximum.saturating_sub(reserved);
        if limit == 0 {
            state.retirement.retire(decoder);
            return Ok(false);
        }
        state.trim_reconfigurable_to_limit(limit.saturating_sub(1));
        if state.len() >= limit {
            state.retirement.retire(decoder);
            return Ok(false);
        }
        state.decoders.insert(
            owner,
            TimeEntry {
                source,
                decoder,
                activity: DecoderActivity::new(),
                last_access: Instant::now(),
                last_foreground_use: None,
            },
        );
        Ok(true)
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .expect("decoder pool mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn has_activity_capacity(&self, reserved: usize) -> bool {
        let state = self.state.lock().expect("decoder pool mutex poisoned");
        let active = state
            .decoders
            .values()
            .chain(&state.orphaned)
            .filter(|entry| !entry.activity.is_idle())
            .count();
        state.maximum.saturating_sub(
            active + state.preparing + state.retirement.pending.load(Ordering::Acquire),
        ) > reserved
    }

    pub fn retire_idle(&self) {
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        state.trim_idle_to_limit(0);
    }

    /// Releases idle decoders before a blocking memory-pressure retry. Never wait while holding
    /// the scheduling mutex: foreground requests must remain able to submit work.
    pub fn reclaim_idle(&self) {
        let completion = {
            let mut state = self.state.lock().expect("decoder pool mutex poisoned");
            state.trim_idle_to_limit(0);
            state.retirement.barrier()
        };
        completion.recv().expect("decoder retirement worker stopped before reclamation completed");
    }

    pub fn evict_idle_owners(&self, mut evict: impl FnMut(&O, &S) -> bool) {
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        let PoolState {
            decoders,
            retirement,
            ..
        } = &mut *state;
        for (_, entry) in decoders
            .extract_if(|owner, entry| entry.activity.is_idle() && evict(owner, &entry.source))
        {
            retirement.retire(entry.decoder);
        }
    }

    pub fn retain_owners(&self, mut retain: impl FnMut(&O, &S) -> bool) {
        let mut state = self.state.lock().expect("decoder pool mutex poisoned");
        let removed = state
            .decoders
            .iter()
            .filter(|(owner, entry)| !retain(owner, &entry.source))
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        for owner in removed {
            let entry = state
                .decoders
                .remove(&owner)
                .expect("retained decoder owner disappeared");
            if !entry.activity.is_idle() {
                state.orphaned.push(entry);
            } else {
                state.retirement.retire(entry.decoder);
            }
        }
        let PoolState {
            orphaned,
            retirement,
            ..
        } = &mut *state;
        for entry in orphaned.extract_if(.., |entry| entry.activity.is_idle()) {
            retirement.retire(entry.decoder);
        }
    }
}

impl<S, O, D> PoolState<S, O, D>
where
    S: Clone + Eq,
    O: Clone + Eq + Hash,
    D: TemporalDecoder<S>,
{
    fn len(&self) -> usize {
        self.decoders.len() + self.orphaned.len() + self.retirement.pending.load(Ordering::Acquire)
    }

    fn set_maximum(&mut self, maximum: usize) {
        assert!(
            maximum > 0,
            "a decoder pool must allow at least one decoder"
        );
        self.maximum = maximum;
        self.trim_idle_to_limit(maximum);
    }

    fn trim_idle_to_limit(&mut self, limit: usize) {
        self.trim_to_limit(limit, Duration::ZERO);
    }

    fn trim_reconfigurable_to_limit(&mut self, limit: usize) {
        self.trim_to_limit(limit, FOREGROUND_RECONFIGURE_DELAY);
    }

    fn trim_to_limit(&mut self, limit: usize, foreground_age: Duration) {
        // Retiring decoders still consume capacity, but selecting them again must not evict
        // additional owners while their destruction runs in the background.
        while self.decoders.len() + self.orphaned.len() > limit {
            let owned = self
                .decoders
                .iter()
                .filter(|(_, entry)| {
                    entry.activity.is_idle()
                        && entry
                            .last_foreground_use
                            .is_none_or(|last| last.elapsed() >= foreground_age)
                })
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(owner, entry)| (owner.clone(), entry.last_access));
            let orphaned = self
                .orphaned
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.activity.is_idle()
                        && entry
                            .last_foreground_use
                            .is_none_or(|last| last.elapsed() >= foreground_age)
                })
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(index, entry)| (index, entry.last_access));
            match (owned, orphaned) {
                (Some((owner, owned_access)), Some((index, orphaned_access))) => {
                    if owned_access <= orphaned_access {
                        let entry = self
                            .decoders
                            .remove(&owner)
                            .expect("idle decoder disappeared");
                        self.retirement.retire(entry.decoder);
                    } else {
                        let entry = self.orphaned.swap_remove(index);
                        self.retirement.retire(entry.decoder);
                    }
                }
                (Some((owner, _)), None) => {
                    let entry = self
                        .decoders
                        .remove(&owner)
                        .expect("idle decoder disappeared");
                    self.retirement.retire(entry.decoder);
                }
                (None, Some((index, _))) => {
                    let entry = self.orphaned.swap_remove(index);
                    self.retirement.retire(entry.decoder);
                }
                (None, None) => break,
            }
        }
    }

    fn has_owner(&self, source: &S, owner: &O) -> bool {
        self.decoders
            .get(owner)
            .is_some_and(|entry| &entry.source == source)
    }

    fn remove_incompatible_owner(&mut self, source: &S, owner: &O) {
        if !self
            .decoders
            .get(owner)
            .is_some_and(|entry| &entry.source != source)
        {
            return;
        }
        let entry = self
            .decoders
            .remove(owner)
            .expect("incompatible decoder owner disappeared");
        if !entry.activity.is_idle() {
            self.orphaned.push(entry);
        } else {
            self.retirement.retire(entry.decoder);
        }
    }

    fn ensure_decoder(
        &mut self,
        source: &S,
        owner: &O,
        handoff_from: Option<&O>,
        foreground: bool,
        allow_overflow: bool,
    ) -> Result<bool, D::Error> {
        self.remove_incompatible_owner(source, owner);
        let now = Instant::now();

        if let Some(entry) = self.decoders.get_mut(owner) {
            entry.last_access = now;
            if foreground {
                entry.last_foreground_use = Some(now);
                self.trim_reconfigurable_to_limit(self.maximum);
            }
            return Ok(true);
        }

        if let Some(previous) = handoff_from
            && self
                .decoders
                .get(previous)
                .is_some_and(|entry| &entry.source == source)
        {
            let mut entry = self
                .decoders
                .remove(previous)
                .expect("handoff decoder owner disappeared");
            entry.last_access = now;
            if foreground {
                entry.last_foreground_use = Some(now);
            }
            self.decoders.insert(owner.clone(), entry);
            if foreground {
                self.trim_reconfigurable_to_limit(self.maximum);
            }
            return Ok(true);
        }

        if self.len() >= self.maximum.saturating_sub(self.preparing) {
            self.trim_reconfigurable_to_limit(self.maximum.saturating_sub(1));
        }
        if !allow_overflow
            && !foreground
            && self.len() >= self.maximum.saturating_sub(self.preparing)
        {
            return Ok(false);
        }

        let decoder = if allow_overflow {
            D::create(source, &self.context)?
        } else {
            let Some(decoder) = D::try_create(source, &self.context)? else {
                return Ok(false);
            };
            decoder
        };
        self.decoders.insert(
            owner.clone(),
            TimeEntry {
                source: source.clone(),
                decoder,
                activity: DecoderActivity::new(),
                last_access: now,
                last_foreground_use: foreground.then_some(now),
            },
        );
        Ok(true)
    }
}
