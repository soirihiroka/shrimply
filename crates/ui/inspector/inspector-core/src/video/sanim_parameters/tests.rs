use super::*;
use shrimply_project_document::project::{
    Asset, CanvasSize, Project, Time, VideoItem, VideoItemContent, VisualTrack,
};
use shrimply_timeline_edit::selection_state;
use shrimply_visual_core::sanim::{Source, Status};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const SOURCE: &str = "let accent=reflect(Color(\"red3\"),key=\"theme.accent\",label=\"Display Color\")\nlet title=reflect(\"Before\",key=\"chart.title\",label=\"Chart Title\")\nadd(Circle(id=\"marker\",radius=12pt,fill=accent))\nwait(1s)\n";
const CANVAS: CanvasSize = CanvasSize {
    width: 100,
    height: 100,
};
const TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    controller: InspectorController,
    address: ItemAddress,
    directory: tempfile::TempDir,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("scene.sanim");
        std::fs::write(&source, text).unwrap();
        let mut item = VideoItem::shape_item(CANVAS, Time::ZERO, Time { seconds: 1.into() });
        item.file = Asset::new(source);
        item.content = VideoItemContent::Sanim(Box::default());
        item.modifiers.clear();
        item.source_duration = Time { seconds: 1.into() };
        let mut track = VisualTrack::default();
        let address = ItemAddress::Video {
            sequence_path: vec![],
            track_id: track.id,
            item_id: item.id,
        };
        track.items.push(item);
        let project = Project {
            canvas_size: CANVAS,
            fps: 30.into(),
            video_tracks: vec![track],
            ..Default::default()
        };
        let player = player_state::new(project.duration(), project.fps);
        let selection = selection_state::new();
        selection_state::set_selected_item_addresses(
            &selection,
            &project,
            vec![address.clone()],
            Some(address.clone()),
        );
        let controller =
            InspectorController::new(Rc::new(RefCell::new(project)), player, selection);
        Self {
            controller,
            address,
            directory,
        }
    }
    fn item(&self) -> VideoItem {
        self.controller
            .project
            .borrow()
            .video_item(&self.address)
            .unwrap()
            .clone()
    }
    fn card(&self) -> VideoCard {
        let snapshot = self.controller.snapshot();
        assert_eq!(snapshot.target, InspectorTarget::Item(self.address.clone()));
        let mut cards = snapshot
            .video
            .expect("selected Sanim must have a video presentation")
            .visual;
        assert_eq!(
            cards[0].key, "sanim-parameters",
            "Sanim controls must precede generic Skia/compositing cards, including during preparation or failure"
        );
        cards.remove(0)
    }
    fn ready_card(&self) -> VideoCard {
        let initial = self.card();
        if initial
            .section
            .controls
            .iter()
            .any(|control| control.path == "/sanim/parameters/chart.title")
        {
            return initial;
        }
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if self.controller.poll_sanim_parameters() {
                let card = self.card();
                if card
                    .section
                    .controls
                    .iter()
                    .any(|control| control.path == "/sanim/parameters/chart.title")
                {
                    return card;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "reflected controls did not become available: {:?}",
            self.card()
        );
    }
    fn activate_history(&self) {
        let path = self.directory.path().join("reflection.shrimp");
        shrimply_project_document::project::create_new_project_file(
            &path,
            &self.controller.project.borrow(),
        )
        .unwrap();
        let project = shrimply_project_document::project::activate_project(
            shrimply_project_document::project::prepare_project(&path).unwrap(),
        );
        *self.controller.project.borrow_mut() = project;
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.controller.sanim_reflection.borrow_mut().clear();
        shrimply_project_document::project::shutdown_history().unwrap();
    }
}

fn render(source: &mut Source, item: &VideoItem) -> [u8; 4] {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        match source
            .poll(item, Time::ZERO, CANVAS, 30.into(), false)
            .unwrap()
        {
            Status::Ready(recording) => {
                let image = recording.rasterize(false).unwrap();
                let index = ((image.height / 2 * image.width + image.width / 2) * 4) as usize;
                return image.rgba[index..index + 4].try_into().unwrap();
            }
            Status::Failed(_) => panic!("Sanim preview failed: {:?}", source.take_error()),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    panic!("Sanim preview did not finish preparing");
}

#[test]
fn sanim_inspector_always_exposes_a_card_and_refreshes_off_playhead_controls() {
    let fixture = Fixture::new(SOURCE);
    let first = fixture.card();
    assert!(!first.section.controls.is_empty());
    assert!(
        crate::list::InspectorListState::default()
            .expanded(&InspectorTarget::Item(fixture.address.clone()), first.key)
    );
    let card = fixture.ready_card();
    let title = card
        .section
        .controls
        .iter()
        .find(|control| control.path == "/sanim/parameters/chart.title")
        .unwrap();
    assert_eq!(title.label, "Chart Title");
    assert_eq!(title.value, "Before");
    assert!(title.tooltip.contains("scene.sanim"));
    assert!(title.tooltip.contains("String"));
    let color = card
        .section
        .controls
        .iter()
        .find(|control| control.path == "/sanim/parameters/theme.accent")
        .unwrap();
    assert_eq!(color.kind, ControlKind::Color);
    assert_eq!(color.label, "Display Color");
}

#[test]
fn sanim_inspector_control_edits_update_live_preview_without_changing_source() {
    let fixture = Fixture::new(SOURCE);
    fixture.activate_history();
    let mut source = Source::new(&fixture.item()).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    source.set_waker(Some(Arc::new({
        let wakes = wakes.clone();
        move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        }
    })));
    let red = render(&mut source, &fixture.item());
    let card = fixture.ready_card();
    let color = card
        .section
        .controls
        .iter()
        .find(|control| control.path == "/sanim/parameters/theme.accent")
        .unwrap();
    let events = Rc::new(Cell::new(0));
    player_state::connect_named(
        &fixture.controller.player_state,
        "Sanim reflected edit test",
        {
            let events = events.clone();
            move |event| {
                if matches!(
                    event,
                    player_state::PlayerEvent::Project(player_state::ProjectChange {
                        video: true,
                        inspector: true,
                        ..
                    })
                ) {
                    events.set(events.get() + 1);
                }
            }
        },
    );
    wakes.store(0, Ordering::Relaxed);
    let blue_channels = source::sanim_lang::core::Color::parse("blue3")
        .unwrap()
        .0
        .map(|channel| (source::math::to_f64(channel).unwrap() * 255.).round());
    fixture
        .controller
        .set_basic_control_components(
            &InspectorTarget::Item(fixture.address.clone()),
            color,
            &blue_channels,
        )
        .unwrap();
    assert_eq!(
        events.get(),
        1,
        "ordinary inspector edits must notify the preview and inspector"
    );
    let item = fixture.item();
    let VideoItemContent::Sanim(settings) = &item.content else {
        unreachable!()
    };
    assert!(settings.parameters.contains_key("theme.accent"));
    let deadline = Instant::now() + TIMEOUT;
    let mut blue = render(&mut source, &item);
    while blue == red && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        blue = render(&mut source, &item);
    }
    assert_ne!(red, blue);
    assert!(red[0] > red[2] && blue[2] > blue[0]);
    assert!(
        wakes.load(Ordering::Relaxed) > 0,
        "reflected input preparation must wake a paused preview"
    );
    assert_eq!(std::fs::read_to_string(item.file.path()).unwrap(), SOURCE);
}

#[test]
fn sanim_inspector_keeps_preparation_errors_visible() {
    let fixture = Fixture::new("let title=reflect(\"Before\")\nunknown_function()\n");
    let _ = fixture.card();
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if fixture.controller.poll_sanim_parameters() {
            let card = fixture.card();
            if card
                .section
                .controls
                .iter()
                .any(|control| control.value.contains("unknown_function"))
            {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("source diagnostic was not displayed: {:?}", fixture.card());
}

#[test]
fn sanim_inspector_union_inputs_keep_all_alternatives_editable() {
    let declaration = source::sanim_lang::core::reflection::Options::default()
        .resolve(
            "scene.sanim",
            "choice",
            source::Type::union([source::Type::Color, source::Type::Bool]).unwrap(),
            Default::default(),
        )
        .unwrap();
    let default = source::Value::Bool(true);
    let input = source::ReflectedInput {
        declaration,
        default: default.clone(),
        value: default,
    };
    let widget = control(&input).unwrap();
    assert_eq!(widget.kind, ControlKind::Text);
    assert_eq!(widget.value, "true");
    assert!(widget.tooltip.contains("Bool") && widget.tooltip.contains("Color"));
    let colored = source::ReflectedInput {
        value: source::Value::Color(source::sanim_lang::core::Color::parse("blue3").unwrap()),
        ..input
    };
    assert_eq!(control(&colored).unwrap().kind, ControlKind::Text);
}

#[test]
#[ignore = "microbenchmark: run with --ignored --nocapture"]
fn sanim_inspector_snapshot_poll_benchmark() {
    let fixture = Fixture::new(SOURCE);
    fixture.ready_card();
    fixture.controller.poll_sanim_parameters();
    let iterations = 100_000;
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(fixture.controller.poll_sanim_parameters());
    }
    eprintln!(
        "Sanim inspector unchanged-state poll: {:.1} ns/op ({iterations} iterations)",
        start.elapsed().as_nanos() as f64 / f64::from(iterations)
    );
}
