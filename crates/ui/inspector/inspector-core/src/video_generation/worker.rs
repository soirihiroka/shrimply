use shrimply_compute_client::CancellationToken;
use shrimply_video_generation::{GenerationResult, VideoGenerationModel, VideoGenerationSettings};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

pub(super) struct Generated {
    path: Option<PathBuf>,
    pub metadata: GenerationResult,
}
impl Generated {
    pub fn path(&self) -> &Path {
        self.path.as_deref().expect("unclaimed generated video")
    }
    pub fn keep(mut self) {
        self.path.take();
    }
}
impl Drop for Generated {
    fn drop(&mut self) {
        if let Some(path) = &self.path
            && let Err(error) = std::fs::remove_file(path)
        {
            tracing::error!(%error, "Could not remove unclaimed generated video");
        }
    }
}
pub(super) enum Event {
    Progress(String),
    Done(Result<Generated, String>),
}
pub(super) struct Task {
    cancellation: CancellationToken,
    receiver: Option<Receiver<Event>>,
}
impl Task {
    pub fn start(
        server: &str,
        model: VideoGenerationModel,
        settings: VideoGenerationSettings,
    ) -> Result<Self, String> {
        let cancellation = CancellationToken::new(server)?;
        let worker = cancellation.clone();
        let directory =
            shrimply_project_document::project::project_directory().join("media/video-generation");
        let destination = directory.join(format!("{}.mp4", uuid::Uuid::new_v4()));
        let server = server.to_owned();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("video-generation".into())
            .spawn(move || {
                let result = shrimply_video_generation::generation_request(&model, &settings)
                    .and_then(|request| {
                        shrimply_video_generation::generate(
                            &server,
                            &worker,
                            &request,
                            &destination,
                            |message| {
                                sender.send(Event::Progress(message.to_owned())).is_ok()
                                    && !worker.is_cancelled()
                            },
                        )
                        .map(|metadata| Generated {
                            path: Some(destination),
                            metadata,
                        })
                    });
                let _ = sender.send(Event::Done(result));
            })
            .map_err(|error| format!("Could not start video generation: {error}"))?;
        Ok(Self {
            cancellation,
            receiver: Some(receiver),
        })
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn poll(&mut self) -> Option<Event> {
        match self.receiver.as_ref()?.try_recv() {
            Ok(Event::Done(result)) => {
                self.receiver = None;
                Some(Event::Done(result.and_then(|generated| {
                    if self.cancelled() {
                        Err("Cancelled".into())
                    } else {
                        Ok(generated)
                    }
                })))
            }
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                Some(Event::Done(Err(
                    "Generation worker stopped unexpectedly".into()
                )))
            }
        }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        if self.receiver.is_some() {
            self.cancel();
        }
    }
}
