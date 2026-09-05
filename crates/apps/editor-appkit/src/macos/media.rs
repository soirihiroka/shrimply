use objc2::{ClassType, rc::Retained};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSPasteboard};
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_state::{player_state, preferences};
use shrimply_timeline_core::{
    DragCollisionMode, TrackKey, import,
    import_queue::{ImportQueue, Placement},
    items::NewItemTarget,
};
use std::path::PathBuf;

pub(super) struct ScopedUrl {
    url: Retained<NSURL>,
    scoped: bool,
}

impl ScopedUrl {
    pub(super) fn new(url: Retained<NSURL>) -> Self {
        let scoped = unsafe { url.startAccessingSecurityScopedResource() };
        Self { url, scoped }
    }
}

impl Drop for ScopedUrl {
    fn drop(&mut self) {
        if self.scoped {
            // Balanced with the successful start; the retained URL outlives every media reader.
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

#[derive(Default)]
pub struct Imports {
    queue: ImportQueue,
    urls: Vec<ScopedUrl>,
}

pub enum Destination {
    Timeline(Placement),
    Tracks(Vec<TrackKey>),
}

impl Imports {
    pub(super) fn retain_scopes(&self) -> Vec<ScopedUrl> {
        self.urls
            .iter()
            .map(|scope| ScopedUrl::new(scope.url.clone()))
            .collect()
    }

    pub fn enqueue(
        &mut self,
        urls: impl IntoIterator<Item = Retained<NSURL>>,
        session: &EditorSession,
        destination: Destination,
    ) -> Result<(), String> {
        let mut paths = Vec::<PathBuf>::new();
        let mut scopes = Vec::new();
        for url in urls {
            paths.push(
                url.to_file_path()
                    .ok_or("only local files can be imported")?,
            );
            // Retain access for the editor lifetime, including background preview/audio reads.
            scopes.push(ScopedUrl::new(url));
        }
        if paths.is_empty() {
            return Err("drop contains no files".into());
        }
        let duration = preferences::snapshot(&session.preferences).default_visual_duration;
        let project = session.project.borrow();
        match destination {
            Destination::Timeline(placement) => {
                self.queue.enqueue(paths, &project, placement, duration)?
            }
            Destination::Tracks(tracks) => self.queue.enqueue_tracks(
                paths,
                &project,
                &tracks,
                player_state::current_time(&session.player_state),
                duration,
            )?,
        }
        self.urls.extend(scopes);
        Ok(())
    }

    pub fn poll(&mut self, session: &EditorSession) -> Result<(), String> {
        loop {
            let result = self.queue.poll(&mut session.project.borrow_mut());
            let Some(result) = result else {
                return Ok(());
            };
            import::finish_track_import(&session.player_state, &session.selection_state, result)?;
        }
    }
}

pub fn file_urls(pasteboard: &NSPasteboard) -> Vec<Retained<NSURL>> {
    // Request NSURL objects so AppKit performs percent decoding and preserves file-system paths.
    unsafe {
        pasteboard.readObjectsForClasses_options(&NSArray::from_slice(&[NSURL::class()]), None)
    }
    .map(|objects| {
        objects
            .iter()
            .filter_map(|object| object.downcast::<NSURL>().ok())
            .collect()
    })
    .unwrap_or_default()
}

pub fn choose_files(
    imports: &std::cell::RefCell<Imports>,
    session: &EditorSession,
    tracks: &[TrackKey],
    mtm: MainThreadMarker,
) -> Result<(), String> {
    let addresses = {
        let project = session.project.borrow();
        tracks
            .iter()
            .map(|key| {
                shrimply_timeline_core::selection_state::track_address(&project, *key)
                    .ok_or("import destination track no longer exists")
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    if panel.runModal() != NSModalResponseOK {
        return Ok(());
    }
    let tracks = {
        let project = session.project.borrow();
        addresses
            .iter()
            .map(|address| {
                shrimply_timeline_core::selection_state::track_key(&project, address)
                    .ok_or("import destination track was removed while choosing files")
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    imports.borrow_mut().enqueue(
        panel.URLs(),
        session,
        if tracks.is_empty() {
            Destination::Timeline(Placement {
                start: player_state::current_time(&session.player_state),
                target: NewItemTarget::Automatic,
                collision: DragCollisionMode::NewTrack,
            })
        } else {
            Destination::Tracks(tracks.to_vec())
        },
    )
}
