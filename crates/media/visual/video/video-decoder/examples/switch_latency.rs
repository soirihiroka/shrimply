use shrimply_asset::Asset;
use shrimply_math_core::{Time, fraction_new};
use shrimply_project_document::project::{CanvasSize, VideoItem, VideoItemContent};
use shrimply_video_decoder::{DecodeRequest, VideoDecoderPool, VideoPlane};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let maximum = args[1].parse().unwrap();
    let warm = args[2] == "warm";
    let local = args[3] == "local";
    let mut pool = VideoDecoderPool::new(maximum);
    let track = Uuid::new_v4();
    let canvas = CanvasSize {
        width: 1280,
        height: 720,
    };
    let items: Vec<_> = ["a.mp4", "b.mp4"]
        .into_iter()
        .map(|name| {
            let mut item = VideoItem::background_item(canvas, Time::ZERO, Time::from_seconds(10));
            item.content = VideoItemContent::Media;
            item.file = Asset::new(std::path::Path::new("/tmp/shrimply-decoder-switch").join(name));
            item.source_width = canvas.width;
            item.source_height = canvas.height;
            item.source_duration = Time::from_seconds(10);
            item.playback_fps = fraction_new(30, 1);
            item
        })
        .collect();
    let owners: Vec<_> = items
        .iter()
        .map(|item| pool.owner(&[], track, item.id, VideoPlane::Color))
        .collect();
    let handles: Vec<_> = items
        .iter()
        .zip(&owners)
        .map(|(item, owner)| pool.decoder(item, owner.clone()).unwrap())
        .collect();
    handles[0]
        .request(DecodeRequest::accurate(Time::ZERO))
        .unwrap()
        .receive()
        .unwrap();
    if warm && pool.prepare(&items[1], owners[1].clone()).unwrap() {
        handles[1]
            .try_request(DecodeRequest::continuous(Time::ZERO), false)
            .unwrap()
            .unwrap()
            .receive()
            .unwrap();
    }
    if args.get(4).is_some_and(|scenario| scenario == "cancel") {
        let generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let started = Instant::now();
        for revision in 0..100 {
            generation.store(revision, std::sync::atomic::Ordering::Release);
            handles[1].try_latest(
                DecodeRequest::local_scrub(Time::from_seconds(if revision % 2 == 0 { 7 } else { 3 }))
                    .control(Some(shrimply_video_decoder::DecodeControl::new(revision, generation.clone()))),
                true,
            ).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        while !handles[1].current().is_some_and(|frame| frame.0 == Time::from_seconds(3)) {
            handles[1].try_latest(DecodeRequest::local_scrub(Time::from_seconds(3)), true).unwrap();
            assert!(started.elapsed() < Duration::from_secs(10), "cancelled startup did not recover");
            std::thread::sleep(Duration::from_millis(1));
        }
        println!("cancel_recovered_us={}", started.elapsed().as_micros());
    }
    if args.get(4).is_some_and(|scenario| scenario == "parallel") {
        let mut distant = items[0].clone();
        distant.id = Uuid::new_v4();
        let owner = pool.owner(&[], track, distant.id, VideoPlane::Alpha);
        let distant = pool.decoder(&distant, owner).unwrap();
        let started = Instant::now();
        distant.try_latest(DecodeRequest::continuous(Time::from_seconds(7)), true).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        let mut incoming_ready = None;
        let mut distant_ready = None;
        while incoming_ready.is_none() || distant_ready.is_none() {
            if handles[1].current().is_some_and(|frame| frame.0 == Time::ZERO) && incoming_ready.is_none() {
                incoming_ready = Some(started.elapsed());
            }
            if distant.current().is_some_and(|frame| frame.0 == Time::from_seconds(7)) && distant_ready.is_none() {
                distant_ready = Some(started.elapsed());
            }
            if incoming_ready.is_none() { handles[1].try_latest(DecodeRequest::continuous(Time::ZERO), true).unwrap(); }
            if distant_ready.is_none() { distant.try_latest(DecodeRequest::continuous(Time::from_seconds(7)), true).unwrap(); }
            assert!(started.elapsed() < Duration::from_secs(10), "parallel initialization did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
        println!("parallel_incoming_us={} parallel_distant_us={}", incoming_ready.unwrap().as_micros(), distant_ready.unwrap().as_micros());
    }
    for switch in 0..6 {
        let from = switch % 2;
        let to = 1 - from;
        handles[from].touch_foreground();
        let target = Time::from_fraction(switch as i64, 30);
        let started = Instant::now();
        let mut admission = None;
        let mut max_submit = Duration::ZERO;
        loop {
            if handles[to].current().is_some_and(|frame| frame.0 == target) {
                break;
            }
            let submit = Instant::now();
            let request = if local {
                DecodeRequest::local_scrub(target)
            } else {
                DecodeRequest::continuous(target)
            };
            if handles[to].try_latest(request, true).unwrap() && admission.is_none() {
                admission = Some(started.elapsed());
            }
            max_submit = max_submit.max(submit.elapsed());
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "decoder switch timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        println!(
            "pool={maximum} warm={warm} local={local} switch={switch} admission_us={} first_frame_us={} max_submit_us={} sessions={}",
            admission.unwrap_or_default().as_micros(),
            started.elapsed().as_micros(),
            max_submit.as_micros(),
            pool.session_count()
        );
    }
    let retired = Instant::now();
    pool.reclaim_idle();
    println!("reclaim_us={}", retired.elapsed().as_micros());
    while pool.session_count() != 0 {
        assert!(
            retired.elapsed() < Duration::from_secs(10),
            "decoder retirement timed out"
        );
        pool.reclaim_idle();
        std::thread::sleep(Duration::from_millis(1));
    }
    println!("retirement_complete_us={}", retired.elapsed().as_micros());
    println!("{}", shrimply_profiling::report_json());
}
