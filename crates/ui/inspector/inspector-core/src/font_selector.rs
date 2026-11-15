use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use shrimply_project_document::project::FontFamily as ProjectFontFamily;

use crate::font_cache::{self, FontCatalog, FontFamily, FontSource, GoogleFamily};

pub const GOOGLE_LOOKUP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PreviewFormat {
    Typeface,
    FontFile,
}
type PreviewKey = (FontSource, String, i64, PreviewFormat);

enum Message {
    Catalog(FontCatalog),
    Preview {
        family: FontFamily,
        format: PreviewFormat,
        generation: u64,
        result: Option<Result<(), String>>,
    },
    Lookup {
        generation: u64,
        result: Result<GoogleFamily, String>,
    },
    Activation(Result<FontFamily, String>),
}

#[derive(Default)]
pub struct Poll {
    pub changed: bool,
    pub visible_changed: bool,
    pub activations: Vec<Result<FontFamily, String>>,
}

pub struct Browser {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    families: Vec<FontFamily>,
    visible: Vec<FontFamily>,
    lookup: Option<GoogleFamily>,
    query: String,
    status: String,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    lookup_generation: Option<u64>,
    pending_previews: HashMap<PreviewKey, u64>,
    preview_queue: VecDeque<(FontFamily, PreviewFormat)>,
    active_previews: usize,
    catalog_loaded: bool,
    loading_catalog: bool,
    looking_up: bool,
    activating: bool,
}

impl Default for Browser {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            families: Vec::new(),
            visible: Vec::new(),
            lookup: None,
            query: String::new(),
            status: String::new(),
            generation: 0,
            current_generation: Arc::new(AtomicU64::new(0)),
            lookup_generation: None,
            pending_previews: HashMap::new(),
            preview_queue: VecDeque::new(),
            active_previews: 0,
            catalog_loaded: false,
            loading_catalog: false,
            looking_up: false,
            activating: false,
        }
    }
}

impl Browser {
    pub fn open(&mut self) -> bool {
        if self.catalog_loaded || self.loading_catalog {
            return false;
        }
        self.loading_catalog = true;
        if self.families.is_empty() {
            self.status = "Loading installed fonts…".to_string();
        }
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::Catalog(font_cache::available_families()));
        });
        true
    }

    pub fn search(&mut self, query: impl Into<String>) -> bool {
        let previous_generation = self.generation;
        let generation = self.set_query(query);
        let lookup_started = self.begin_lookup(generation);
        previous_generation != generation || lookup_started
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> u64 {
        let query = query.into();
        if self.query == query {
            return self.generation;
        }
        self.query = query;
        self.lookup = None;
        self.generation = self.generation.wrapping_add(1);
        self.current_generation
            .store(self.generation, Ordering::Release);
        self.lookup_generation = None;
        self.looking_up = false;
        self.pending_previews.clear();
        self.preview_queue.clear();
        if self.loading_catalog && self.families.is_empty() {
            self.status = "Loading installed fonts…".to_string();
        } else {
            self.status.clear();
        }
        self.rebuild();
        self.generation
    }

    pub fn begin_lookup(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.lookup_generation == Some(generation) {
            return false;
        }
        if self.loading_catalog && self.families.is_empty() {
            return false;
        }
        if !font_cache::google_lookup_needed(&self.families, &self.query) {
            return false;
        }
        self.lookup_generation = Some(generation);
        self.looking_up = true;
        self.status = "Searching Google Fonts…".to_string();
        let generation = self.generation;
        let query = self.query.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::Lookup {
                generation,
                result: font_cache::lookup_google_family(&query),
            });
        });
        true
    }

    pub fn request_previews(&mut self, range: Range<usize>, format: PreviewFormat) {
        let end = range.end.min(self.visible.len());
        let wanted = self.visible[range.start.min(end)..end]
            .iter()
            .map(|family| preview_key(family, format))
            .collect::<HashSet<_>>();
        self.pending_previews
            .retain(|key, _| key.3 != format || wanted.contains(key));
        self.preview_queue.retain(|(family, queued_format)| {
            self.pending_previews
                .contains_key(&preview_key(family, *queued_format))
        });
        for family in self
            .visible
            .get(range.start.min(end)..end)
            .into_iter()
            .flatten()
        {
            let ready = match format {
                PreviewFormat::Typeface => {
                    family.source != FontSource::Google
                        || family.revision < 0
                        || font_cache::preview_source(family, self.lookup.as_ref()).is_ok()
                }
                PreviewFormat::FontFile => font_cache::native_preview_ready(family),
            };
            if ready {
                continue;
            }
            let key = preview_key(family, format);
            if let Some(generation) = self.pending_previews.get_mut(&key) {
                *generation = self.generation;
            } else {
                self.pending_previews.insert(key, self.generation);
                self.preview_queue.push_back((family.clone(), format));
            }
        }
        self.start_preview_jobs();
    }

    pub fn activate(&mut self, family: FontFamily) -> Result<(), String> {
        if self.activating {
            return Err("A font is already being activated".to_string());
        }
        self.activating = true;
        self.status = if family.revision < 0 {
            "Downloading font family…"
        } else {
            "Loading cached font…"
        }
        .to_string();
        let lookup = self.lookup.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Message::Activation(font_cache::activate_selection(
                family,
                lookup.as_ref(),
            )));
        });
        Ok(())
    }

    pub fn poll(&mut self) -> Poll {
        let mut poll = Poll::default();
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Catalog(catalog) => {
                    poll.changed = true;
                    poll.visible_changed = true;
                    self.loading_catalog = false;
                    self.catalog_loaded = true;
                    self.families = catalog.families;
                    if let Some(error) = catalog.cache_error {
                        self.status = error;
                    } else if !self.looking_up {
                        self.status.clear();
                    }
                    self.rebuild();
                    self.begin_lookup(self.generation);
                }
                Message::Preview {
                    family,
                    format,
                    generation,
                    result,
                } => {
                    self.active_previews = self.active_previews.saturating_sub(1);
                    let key = preview_key(&family, format);
                    let requested_generation = self.pending_previews.get(&key).copied();
                    if requested_generation == Some(generation) {
                        self.pending_previews.remove(&key);
                    }
                    let current = self.visible.iter().any(|candidate| {
                        candidate.source == family.source
                            && candidate.revision == family.revision
                            && candidate.name.eq_ignore_ascii_case(&family.name)
                    });
                    match result {
                        Some(Ok(()))
                            if generation == self.generation
                                && current
                                && requested_generation == Some(generation) =>
                        {
                            poll.changed = true;
                        }
                        Some(Ok(())) | None => {}
                        Some(Err(error)) if generation == self.generation => tracing::warn!(
                            family = family.name,
                            revision = family.revision,
                            "Could not prepare Google font preview: {error}"
                        ),
                        Some(Err(_)) => {}
                    }
                    self.start_preview_jobs();
                }
                Message::Lookup { generation, result } if generation == self.generation => {
                    poll.changed = true;
                    poll.visible_changed = true;
                    self.looking_up = false;
                    match result {
                        Ok(family) => {
                            self.lookup = Some(family);
                            self.status.clear();
                        }
                        Err(error) => self.status = error,
                    }
                    self.rebuild();
                }
                Message::Lookup { .. } => {}
                Message::Activation(result) => {
                    poll.changed = true;
                    poll.visible_changed = true;
                    self.activating = false;
                    match &result {
                        Ok(family) => {
                            if let Some(existing) = self.families.iter_mut().find(|candidate| {
                                candidate.source == family.source
                                    && candidate.name.eq_ignore_ascii_case(&family.name)
                            }) {
                                existing.clone_from(family);
                            } else {
                                self.families.push(family.clone());
                            }
                            self.status.clear();
                            self.rebuild();
                        }
                        Err(error) => self.status.clone_from(error),
                    }
                    poll.activations.push(result);
                }
            }
        }
        poll
    }

    pub fn visible(&self) -> &[FontFamily] {
        &self.visible
    }

    pub fn lookup(&self) -> Option<&GoogleFamily> {
        self.lookup.as_ref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn busy(&self) -> bool {
        self.loading_catalog || self.looking_up || self.activating
    }

    fn rebuild(&mut self) {
        self.visible =
            font_cache::matching_families(&self.families, &self.query, self.lookup.as_ref());
    }

    fn start_preview_jobs(&mut self) {
        const MAX_PREVIEW_WORKERS: usize = 2;
        while self.active_previews < MAX_PREVIEW_WORKERS {
            let Some((family, format)) = self.preview_queue.pop_front() else {
                break;
            };
            if !self
                .pending_previews
                .contains_key(&preview_key(&family, format))
            {
                continue;
            }
            let generation = self.generation;
            self.active_previews += 1;
            let sender = self.sender.clone();
            let current_generation = Arc::clone(&self.current_generation);
            let lookup = self.lookup.clone();
            thread::spawn(move || {
                let result = (current_generation.load(Ordering::Acquire) == generation)
                    .then(|| match format {
                        PreviewFormat::Typeface => font_cache::prepare_cached_preview(&family),
                        PreviewFormat::FontFile => {
                            font_cache::prepare_native_preview(&family, lookup.as_ref())
                        }
                    })
                    .filter(|_| current_generation.load(Ordering::Acquire) == generation);
                let _ = sender.send(Message::Preview {
                    family,
                    format,
                    generation,
                    result,
                });
            });
        }
    }
}

fn preview_key(family: &FontFamily, format: PreviewFormat) -> PreviewKey {
    (
        family.source,
        family.name.to_lowercase(),
        family.revision,
        format,
    )
}

pub fn normalized_families(families: &[ProjectFontFamily]) -> Vec<ProjectFontFamily> {
    families
        .iter()
        .filter_map(|family| {
            let name = family.name().trim();
            (!name.is_empty()).then(|| with_name(family, name.to_string()))
        })
        .collect()
}

pub fn replace_family(
    families: &[ProjectFontFamily],
    index: usize,
    family: ProjectFontFamily,
) -> Option<Vec<ProjectFontFamily>> {
    let mut next = normalized_families(families);
    if index >= next.len()
        || next.iter().enumerate().any(|(candidate_index, candidate)| {
            candidate_index != index && candidate.name().eq_ignore_ascii_case(family.name())
        })
    {
        return None;
    }
    next[index] = family;
    Some(next)
}

pub fn append_family(
    families: &[ProjectFontFamily],
    family: ProjectFontFamily,
) -> Option<Vec<ProjectFontFamily>> {
    let mut next = normalized_families(families);
    if next
        .iter()
        .any(|candidate| candidate.name().eq_ignore_ascii_case(family.name()))
    {
        return None;
    }
    next.push(family);
    Some(next)
}

pub fn move_family(
    families: &[ProjectFontFamily],
    index: usize,
    offset: isize,
) -> Option<Vec<ProjectFontFamily>> {
    let mut next = normalized_families(families);
    let destination = index.checked_add_signed(offset)?;
    if index >= next.len() || destination >= next.len() {
        return None;
    }
    next.swap(index, destination);
    Some(next)
}

pub fn remove_family(
    families: &[ProjectFontFamily],
    index: usize,
) -> Option<Vec<ProjectFontFamily>> {
    let mut next = normalized_families(families);
    (index < next.len()).then(|| {
        next.remove(index);
        next
    })
}

fn with_name(family: &ProjectFontFamily, name: String) -> ProjectFontFamily {
    match family {
        ProjectFontFamily::Local { .. } => ProjectFontFamily::Local { name },
        ProjectFontFamily::GoogleFonts { .. } => ProjectFontFamily::GoogleFonts { name },
    }
}

#[derive(Clone)]
pub enum FamilyEdit {
    Append(ProjectFontFamily),
    Replace {
        index: usize,
        family: ProjectFontFamily,
    },
    Remove(usize),
    Move {
        index: usize,
        offset: isize,
    },
}

impl crate::InspectorController {
    pub fn control_font_families(
        &self,
        target: &crate::InspectorTarget,
        control: &crate::InspectorControl,
    ) -> Result<Vec<ProjectFontFamily>, String> {
        let (value, _) = self.control_graph_source(target)?;
        read_control_font_families(&value, control)
    }

    pub fn edit_control_font_family(
        &self,
        target: &crate::InspectorTarget,
        control: &crate::InspectorControl,
        edit: FamilyEdit,
    ) -> Result<(), String> {
        let families = self.control_font_families(target, control)?;
        let next = match edit {
            FamilyEdit::Append(family) => append_family(&families, family),
            FamilyEdit::Replace { index, family } => replace_family(&families, index, family),
            FamilyEdit::Remove(index) => remove_family(&families, index),
            FamilyEdit::Move { index, offset } => move_family(&families, index, offset),
        };
        if let Some(next) = next {
            self.set_basic_control_value(
                target,
                control,
                &serde_json::to_string(&next).expect("font families serialize"),
            )?;
        }
        Ok(())
    }
}

fn read_control_font_families(
    value: &serde_json::Value,
    control: &crate::InspectorControl,
) -> Result<Vec<ProjectFontFamily>, String> {
    if control.kind != crate::ControlKind::FontFamilies {
        return Err("font list is unavailable".to_string());
    }
    if let Some(families) = value.pointer(&control.path) {
        return serde_json::from_value(families.clone())
            .map_err(|error| format!("invalid font list: {error}"));
    }
    let Some(parent_path) = control.path.strip_suffix("/font_families") else {
        return Err("font list is unavailable".to_string());
    };
    if matches!(
        value.pointer(parent_path),
        Some(serde_json::Value::Object(_))
    ) {
        Ok(Vec::new())
    } else {
        Err("font list is unavailable".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlKind, InspectorControl, InspectorController, InspectorTarget};
    use shrimply_math_core::Fraction;
    use shrimply_project_document::project::{
        CanvasSize, ItemAddress, ItemRef, Project, Time, VideoItem, VideoItemContent, VisualTrack,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    fn test_controller_with_fonts(
        font_families: Vec<shrimply_property_model::FontFamily>,
    ) -> (
        InspectorController,
        Rc<RefCell<Project>>,
        InspectorTarget,
        InspectorControl,
    ) {
        let canvas_size = CanvasSize {
            width: 1920,
            height: 1080,
        };
        let mut text_item = VideoItem::text_item(canvas_size, Time::ZERO, Time::from_seconds(5));
        let item_id = text_item.id;
        if let VideoItemContent::Text(text) = &mut text_item.content {
            text.font_families = font_families;
        } else {
            unreachable!();
        }

        let track_id = uuid::Uuid::new_v4();
        let mut track = VisualTrack::default();
        track.id = track_id;
        track.items.push(text_item);

        let project = Project {
            name: "Test Project".to_string(),
            fps: Fraction::from(30),
            canvas_size,
            video_tracks: vec![track],
            ..Default::default()
        };

        let project_cell = Rc::new(RefCell::new(project));
        let player =
            shrimply_editor_state::player_state::new(Time::from_seconds(5), Fraction::from(30));
        let selection = shrimply_timeline_edit::selection_state::new();
        let controller = InspectorController::new(project_cell.clone(), player, selection);
        let target = InspectorTarget::Item(ItemAddress::Video {
            sequence_path: Vec::new(),
            track_id,
            item_id,
        });
        let control = InspectorControl::new(
            ControlKind::FontFamilies,
            crate::generated::text::FONT_FAMILIES_PATH,
            "Fonts",
        )
        .immediate_commit(crate::generated::text::FONT_EDIT_COMMIT);
        (controller, project_cell, target, control)
    }

    fn current_font_names(
        project_cell: &Rc<RefCell<Project>>,
        target: &InspectorTarget,
    ) -> Vec<String> {
        let project = project_cell.borrow();
        let InspectorTarget::Item(address) = target else {
            panic!("expected item target");
        };
        let ItemRef::Video(video_item) = project.item(address).expect("video item") else {
            panic!("expected video item");
        };
        let VideoItemContent::Text(text) = &video_item.content else {
            panic!("expected text item");
        };
        text.font_families
            .iter()
            .map(|f| f.name().to_string())
            .collect()
    }

    #[test]
    fn case_1_existing_font_and_append() {
        let (controller, project, target, control) =
            test_controller_with_fonts(vec![ProjectFontFamily::GoogleFonts {
                name: "Inter".to_string(),
            }]);
        assert_eq!(current_font_names(&project, &target), vec!["Inter"]);

        controller
            .edit_control_font_family(
                &target,
                &control,
                FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                    name: "Roboto".to_string(),
                }),
            )
            .expect("append should succeed");

        assert_eq!(
            current_font_names(&project, &target),
            vec!["Inter", "Roboto"]
        );
    }

    #[test]
    fn case_2_remove_last_font_then_append() {
        let (controller, project, target, control) =
            test_controller_with_fonts(vec![ProjectFontFamily::GoogleFonts {
                name: "Inter".to_string(),
            }]);
        assert_eq!(current_font_names(&project, &target), vec!["Inter"]);

        // Remove the default font
        controller
            .edit_control_font_family(&target, &control, FamilyEdit::Remove(0))
            .expect("remove should succeed");
        assert!(current_font_names(&project, &target).is_empty());

        // Verify that the serialized model omitted the font_families key due to skip_serializing_if
        let (serialized, _) = controller
            .control_graph_source(&target)
            .expect("target must serialize");
        assert!(
            serialized.pointer("/content/font_families").is_none(),
            "empty font_families must be omitted in serialized JSON"
        );
        assert!(
            serialized.pointer("/content").is_some(),
            "parent content object must exist"
        );

        // Append a new font now that font_families is empty
        controller
            .edit_control_font_family(
                &target,
                &control,
                FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                    name: "Roboto".to_string(),
                }),
            )
            .expect("append after removing last font must succeed");

        assert_eq!(current_font_names(&project, &target), vec!["Roboto"]);
    }

    #[test]
    fn case_3_empty_list_directly() {
        let (controller, project, target, control) = test_controller_with_fonts(Vec::new());
        assert!(current_font_names(&project, &target).is_empty());

        let (serialized, _) = controller
            .control_graph_source(&target)
            .expect("target must serialize");
        assert!(serialized.pointer("/content/font_families").is_none());

        controller
            .edit_control_font_family(
                &target,
                &control,
                FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                    name: "Open Sans".to_string(),
                }),
            )
            .expect("append to empty list directly must succeed");

        assert_eq!(current_font_names(&project, &target), vec!["Open Sans"]);
    }

    #[test]
    fn case_4_invalid_inspector_paths_return_errors() {
        let (controller, _project, target, _control) = test_controller_with_fonts(Vec::new());

        // Missing parent path
        let invalid_path_control = InspectorControl::new(
            ControlKind::FontFamilies,
            "/nonexistent/parent/font_families",
            "Fonts",
        );
        let result = controller.edit_control_font_family(
            &target,
            &invalid_path_control,
            FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                name: "Roboto".to_string(),
            }),
        );
        assert!(result.is_err(), "missing parent path must return an error");

        // Wrong field name (not ending in /font_families)
        let wrong_field_control = InspectorControl::new(
            ControlKind::FontFamilies,
            "/content/nonexistent_field",
            "Fonts",
        );
        let result = controller.edit_control_font_family(
            &target,
            &wrong_field_control,
            FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                name: "Roboto".to_string(),
            }),
        );
        assert!(result.is_err(), "wrong field name must return an error");

        // Wrong control kind
        let wrong_kind_control = InspectorControl::new(
            ControlKind::Text,
            crate::generated::text::FONT_FAMILIES_PATH,
            "Fonts",
        );
        let result = controller.edit_control_font_family(
            &target,
            &wrong_kind_control,
            FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                name: "Roboto".to_string(),
            }),
        );
        assert!(result.is_err(), "wrong control kind must return an error");
    }

    #[test]
    fn case_5_duplicate_protection() {
        let (controller, project, target, control) =
            test_controller_with_fonts(vec![ProjectFontFamily::GoogleFonts {
                name: "Inter".to_string(),
            }]);

        controller
            .edit_control_font_family(
                &target,
                &control,
                FamilyEdit::Append(ProjectFontFamily::GoogleFonts {
                    name: "Inter".to_string(),
                }),
            )
            .expect("duplicate append should not error");

        assert_eq!(
            current_font_names(&project, &target),
            vec!["Inter"],
            "duplicate font must not be added"
        );
    }

    #[test]
    fn case_6_move_and_remove_on_empty_list() {
        let (controller, project, target, control) = test_controller_with_fonts(Vec::new());

        let remove_result =
            controller.edit_control_font_family(&target, &control, FamilyEdit::Remove(0));
        assert!(remove_result.is_ok());
        assert!(current_font_names(&project, &target).is_empty());

        let move_result = controller.edit_control_font_family(
            &target,
            &control,
            FamilyEdit::Move {
                index: 0,
                offset: 1,
            },
        );
        assert!(move_result.is_ok());
        assert!(current_font_names(&project, &target).is_empty());
    }

    #[test]
    fn case_7_modifier_nested_empty_font_families_and_stale_path() {
        let control = InspectorControl::new(
            ControlKind::FontFamilies,
            "/modifiers/0/effect/effect/config/font_families",
            "Fonts",
        );

        // When parent config object exists, but font_families is omitted:
        let value = serde_json::json!({
            "modifiers": [
                {
                    "effect": {
                        "effect": {
                            "config": {
                                "text": "Hello 3D"
                            }
                        }
                    }
                }
            ]
        });
        let result = read_control_font_families(&value, &control);
        assert_eq!(result.unwrap(), Vec::<ProjectFontFamily>::new());

        // When parent config object does not exist (stale/invalid modifier index):
        let stale_value = serde_json::json!({
            "modifiers": []
        });
        let stale_result = read_control_font_families(&stale_value, &control);
        assert!(stale_result.is_err());
    }
}
