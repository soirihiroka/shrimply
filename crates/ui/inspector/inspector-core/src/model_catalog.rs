use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

type Models<T> = Result<Vec<T>, String>;
type Response<T> = (String, Models<T>);

/// URL-scoped asynchronous catalogs survive inspector document rebuilds.
pub(crate) struct ModelCatalog<T = String> {
    states: HashMap<String, Option<Models<T>>>,
    failed_at: HashMap<String, Instant>,
    sender: mpsc::Sender<Response<T>>,
    receiver: mpsc::Receiver<Response<T>>,
}

impl<T> Default for ModelCatalog<T> {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            failed_at: HashMap::new(),
            sender,
            receiver,
        }
    }
}

impl<T: Clone + Send + 'static> ModelCatalog<T> {
    pub fn retry_failed(&mut self, server_url: &str) {
        if matches!(self.states.get(server_url), Some(Some(Err(_)))) {
            self.states.remove(server_url);
            self.failed_at.remove(server_url);
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((url, result)) = self.receiver.try_recv() {
            if result.is_err() {
                self.failed_at.insert(url.clone(), Instant::now());
            } else {
                self.failed_at.remove(&url);
            }
            self.states.insert(url, Some(result));
            changed = true;
        }
        changed
    }

    pub fn request(
        &mut self,
        server_url: &str,
        cached: Option<Models<T>>,
        load: fn(&str) -> Models<T>,
    ) -> Option<Models<T>> {
        if self
            .failed_at
            .get(server_url)
            .is_some_and(|failed| failed.elapsed() >= RETRY_INTERVAL)
        {
            self.retry_failed(server_url);
        }
        self.states
            .entry(server_url.to_string())
            .or_insert_with(|| {
                if cached.is_some() {
                    return cached;
                }
                let url = server_url.to_string();
                let sender = self.sender.clone();
                std::thread::spawn(move || {
                    let result = load(&url);
                    let _ = sender.send((url, result));
                });
                None
            })
            .clone()
    }
}
