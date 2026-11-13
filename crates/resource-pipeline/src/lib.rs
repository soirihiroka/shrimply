pub mod cache;
pub mod fingerprint;

use std::any::Any;
use std::collections::HashMap;
use std::hash::Hash;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub type Job = Box<dyn FnOnce() + Send>;
pub type Spawn = Arc<dyn Fn(Job) + Send + Sync>;
type ProgressMerge<P> = Arc<dyn Fn(&mut P, P) + Send + Sync>;

pub trait Processor<K>: Send + Sync + 'static {
    type Progress: Clone + Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    fn process(&self, key: K, context: &JobContext<Self::Progress>)
    -> Result<Self::Output, String>;
}

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct JobContext<P> {
    cancellation: CancelToken,
    report: Arc<dyn Fn(P) -> bool + Send + Sync>,
}

impl<P> JobContext<P> {
    pub fn cancellation(&self) -> &CancelToken {
        &self.cancellation
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn report(&self, progress: P) -> bool {
        !self.is_cancelled() && (self.report)(progress)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDisposition {
    Started,
    Joined,
}

#[derive(Debug)]
pub enum Event<P, O> {
    Progress(Arc<P>),
    Finished(Arc<O>),
    Failed(Arc<str>),
    Cancelled,
}

impl<P, O> Event<P, O> {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Progress(_))
    }
}

#[derive(Debug)]
pub enum TryNext<P, O> {
    Event(Event<P, O>),
    Empty,
    Closed,
}

pub struct Subscription<K, P, O> {
    key: K,
    mailbox: Arc<Mailbox<P, O>>,
    detach: Option<Box<dyn FnOnce() + Send>>,
}

impl<K, P, O> Subscription<K, P, O> {
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn try_next(&mut self) -> TryNext<P, O> {
        self.mailbox.try_next()
    }

    pub fn cancel(mut self) {
        self.detach();
    }

    fn detach(&mut self) {
        if let Some(detach) = self.detach.take() {
            detach();
        }
    }
}

impl<K, P, O> Drop for Subscription<K, P, O> {
    fn drop(&mut self) {
        self.detach();
    }
}

pub struct Pipeline<K, R>
where
    R: Processor<K>,
{
    inner: Arc<Inner<K, R>>,
}

impl<K, R> Clone for Pipeline<K, R>
where
    R: Processor<K>,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, R> Pipeline<K, R>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    R: Processor<K>,
{
    /// The spawner must schedule the supplied job without running it inline.
    pub fn new(processor: R, spawn: impl Fn(Job) + Send + Sync + 'static) -> Self {
        Self::new_with_progress_merge(processor, spawn, |current, next| *current = next)
    }

    pub fn new_with_progress_merge(
        processor: R,
        spawn: impl Fn(Job) + Send + Sync + 'static,
        merge_progress: impl Fn(&mut R::Progress, R::Progress) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                processor,
                spawn: Arc::new(spawn),
                merge_progress: Arc::new(merge_progress),
                state: Mutex::new(State::default()),
                next_generation: AtomicU64::new(1),
                next_subscriber: AtomicU64::new(1),
            }),
        }
    }

    pub fn request(&self, key: K) -> (RequestDisposition, Subscription<K, R::Progress, R::Output>) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("resource pipeline state lock poisoned");
        if let Some(active) = state.jobs.get_mut(&key) {
            let subscription =
                subscribe_to(&self.inner, key, active.generation, &mut active.subscribers);
            return (RequestDisposition::Joined, subscription);
        }

        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancelToken::default();
        let mut subscribers = HashMap::new();
        let subscription = subscribe_to(&self.inner, key.clone(), generation, &mut subscribers);
        state.jobs.insert(
            key.clone(),
            Active {
                generation,
                cancellation: cancellation.clone(),
                subscribers,
            },
        );
        drop(state);

        let inner = self.inner.clone();
        let job_key = key.clone();
        let context = JobContext {
            cancellation: cancellation.clone(),
            report: progress_reporter(&self.inner, key.clone(), generation),
        };
        let job = Box::new(move || {
            let completion = CompletionGuard::new(inner.clone(), job_key.clone(), generation);
            let result = catch_unwind(AssertUnwindSafe(|| {
                inner.processor.process(job_key.clone(), &context)
            }));
            match result {
                Ok(Ok(output)) => completion.publish(Terminal::Finished(Arc::new(output))),
                Ok(Err(error)) => completion.publish(Terminal::Failed(error.into())),
                Err(payload) => completion.publish(Terminal::Failed(panic_message(payload).into())),
            }
        });
        let spawn = self.inner.spawn.clone();
        if catch_unwind(AssertUnwindSafe(|| spawn(job))).is_err() {
            finish(
                &self.inner,
                &key,
                generation,
                Terminal::Failed("resource job spawner panicked".into()),
            );
        }
        (RequestDisposition::Started, subscription)
    }

    pub fn subscribe(&self, key: &K) -> Option<Subscription<K, R::Progress, R::Output>> {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("resource pipeline state lock poisoned");
        let active = state.jobs.get_mut(key)?;
        Some(subscribe_to(
            &self.inner,
            key.clone(),
            active.generation,
            &mut active.subscribers,
        ))
    }

    pub fn cancel(&self, key: &K) -> bool {
        cancel(&self.inner, key)
    }
}

struct Inner<K, R>
where
    R: Processor<K>,
{
    processor: R,
    spawn: Spawn,
    merge_progress: ProgressMerge<R::Progress>,
    state: Mutex<State<K, R::Progress, R::Output>>,
    next_generation: AtomicU64,
    next_subscriber: AtomicU64,
}

struct State<K, P, O> {
    jobs: HashMap<K, Active<P, O>>,
}

impl<K, P, O> Default for State<K, P, O> {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }
}

struct Active<P, O> {
    generation: u64,
    cancellation: CancelToken,
    subscribers: Subscribers<P, O>,
}

type Subscribers<P, O> = HashMap<u64, Arc<Mailbox<P, O>>>;

struct Mailbox<P, O> {
    state: Mutex<MailboxState<P, O>>,
    merge_progress: ProgressMerge<P>,
}

struct MailboxState<P, O> {
    progress: Option<P>,
    terminal: Option<Terminal<O>>,
    closed: bool,
}

enum Terminal<O> {
    Finished(Arc<O>),
    Failed(Arc<str>),
    Cancelled,
}

struct CompletionGuard<K, R>
where
    K: Eq + Hash,
    R: Processor<K>,
{
    inner: Arc<Inner<K, R>>,
    key: K,
    generation: u64,
    published: bool,
}

impl<K, R> CompletionGuard<K, R>
where
    K: Eq + Hash,
    R: Processor<K>,
{
    fn new(inner: Arc<Inner<K, R>>, key: K, generation: u64) -> Self {
        Self {
            inner,
            key,
            generation,
            published: false,
        }
    }

    fn publish(mut self, terminal: Terminal<R::Output>) {
        self.published = true;
        finish(&self.inner, &self.key, self.generation, terminal);
    }
}

impl<K, R> Drop for CompletionGuard<K, R>
where
    K: Eq + Hash,
    R: Processor<K>,
{
    fn drop(&mut self) {
        if !self.published {
            finish(
                &self.inner,
                &self.key,
                self.generation,
                Terminal::Failed("resource job exited without a result".into()),
            );
        }
    }
}

impl<P, O> Mailbox<P, O> {
    fn new(merge_progress: ProgressMerge<P>) -> Self {
        Self {
            state: Mutex::new(MailboxState {
                progress: None,
                terminal: None,
                closed: false,
            }),
            merge_progress,
        }
    }

    fn report(&self, progress: P) {
        let mut state = self.state.lock().expect("resource mailbox lock poisoned");
        if state.terminal.is_none() && !state.closed {
            if let Some(current) = &mut state.progress {
                (self.merge_progress)(current, progress);
            } else {
                state.progress = Some(progress);
            }
        }
    }

    fn finish(&self, terminal: Terminal<O>) {
        let mut state = self.state.lock().expect("resource mailbox lock poisoned");
        if state.terminal.is_none() && !state.closed {
            state.terminal = Some(terminal);
        }
    }

    fn try_next(&self) -> TryNext<P, O> {
        let mut state = self.state.lock().expect("resource mailbox lock poisoned");
        if let Some(progress) = state.progress.take() {
            return TryNext::Event(Event::Progress(Arc::new(progress)));
        }
        if let Some(terminal) = state.terminal.take() {
            state.closed = true;
            return TryNext::Event(match terminal {
                Terminal::Finished(output) => Event::Finished(output),
                Terminal::Failed(error) => Event::Failed(error),
                Terminal::Cancelled => Event::Cancelled,
            });
        }
        if state.closed {
            TryNext::Closed
        } else {
            TryNext::Empty
        }
    }
}

fn subscribe_to<K, R>(
    inner: &Arc<Inner<K, R>>,
    key: K,
    generation: u64,
    subscribers: &mut Subscribers<R::Progress, R::Output>,
) -> Subscription<K, R::Progress, R::Output>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    R: Processor<K>,
{
    let subscriber = inner.next_subscriber.fetch_add(1, Ordering::Relaxed);
    let mailbox = Arc::new(Mailbox::new(inner.merge_progress.clone()));
    subscribers.insert(subscriber, mailbox.clone());
    let weak = Arc::downgrade(inner);
    let detach_key = key.clone();
    Subscription {
        key,
        mailbox,
        detach: Some(Box::new(move || {
            detach(&weak, &detach_key, generation, subscriber);
        })),
    }
}

fn progress_reporter<K, R>(
    inner: &Arc<Inner<K, R>>,
    key: K,
    generation: u64,
) -> Arc<dyn Fn(R::Progress) -> bool + Send + Sync>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    R: Processor<K>,
{
    let inner = Arc::downgrade(inner);
    Arc::new(move |progress| {
        let Some(inner) = inner.upgrade() else {
            return false;
        };
        let subscribers = {
            let state = inner
                .state
                .lock()
                .expect("resource pipeline state lock poisoned");
            let Some(active) = state
                .jobs
                .get(&key)
                .filter(|active| active.generation == generation)
            else {
                return false;
            };
            if active.cancellation.is_cancelled() || active.subscribers.is_empty() {
                return false;
            }
            active.subscribers.values().cloned().collect::<Vec<_>>()
        };
        let mut subscribers = subscribers.into_iter();
        let Some(last) = subscribers.next_back() else {
            return false;
        };
        for subscriber in subscribers {
            subscriber.report(progress.clone());
        }
        last.report(progress);
        true
    })
}

fn finish<K, R>(inner: &Arc<Inner<K, R>>, key: &K, generation: u64, terminal: Terminal<R::Output>)
where
    K: Eq + Hash,
    R: Processor<K>,
{
    let subscribers = {
        let mut state = inner
            .state
            .lock()
            .expect("resource pipeline state lock poisoned");
        if state
            .jobs
            .get(key)
            .is_none_or(|active| active.generation != generation)
        {
            return;
        }
        let active = state
            .jobs
            .remove(key)
            .expect("current resource job disappeared");
        let cancelled = active.cancellation.is_cancelled();
        (
            active.subscribers.into_values().collect::<Vec<_>>(),
            cancelled,
        )
    };
    let (subscribers, cancelled) = subscribers;
    publish_terminal(
        subscribers,
        if cancelled {
            Terminal::Cancelled
        } else {
            terminal
        },
    );
}

fn cancel<K, R>(inner: &Arc<Inner<K, R>>, key: &K) -> bool
where
    K: Eq + Hash,
    R: Processor<K>,
{
    let active = inner
        .state
        .lock()
        .expect("resource pipeline state lock poisoned")
        .jobs
        .remove(key);
    let Some(active) = active else {
        return false;
    };
    active.cancellation.cancel();
    publish_terminal(
        active.subscribers.into_values().collect(),
        Terminal::Cancelled,
    );
    true
}

fn detach<K, R>(inner: &Weak<Inner<K, R>>, key: &K, generation: u64, subscriber: u64)
where
    K: Eq + Hash,
    R: Processor<K>,
{
    let Some(inner) = inner.upgrade() else {
        return;
    };
    let cancellation = {
        let mut state = inner
            .state
            .lock()
            .expect("resource pipeline state lock poisoned");
        let Some(active) = state
            .jobs
            .get_mut(key)
            .filter(|active| active.generation == generation)
        else {
            return;
        };
        active.subscribers.remove(&subscriber);
        if !active.subscribers.is_empty() {
            return;
        }
        state.jobs.remove(key).map(|active| active.cancellation)
    };
    if let Some(cancellation) = cancellation {
        cancellation.cancel();
    }
}

fn publish_terminal<P, O>(subscribers: Vec<Arc<Mailbox<P, O>>>, terminal: Terminal<O>) {
    match terminal {
        Terminal::Finished(output) => {
            for subscriber in subscribers {
                subscriber.finish(Terminal::Finished(output.clone()));
            }
        }
        Terminal::Failed(error) => {
            for subscriber in subscribers {
                subscriber.finish(Terminal::Failed(error.clone()));
            }
        }
        Terminal::Cancelled => {
            for subscriber in subscribers {
                subscriber.finish(Terminal::Cancelled);
            }
        }
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    detail.map_or_else(
        || "resource processor panicked".to_string(),
        |detail| format!("resource processor panicked: {detail}"),
    )
}
