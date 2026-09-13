use std::sync::atomic::{AtomicU64, Ordering};

mod pool;
mod session;
mod startup;
mod track;

pub use pool::{DecodeRequest, PendingDecode, VideoDecoderHandle, VideoDecoderPool};
pub use session::{DecodeControl, DecodeOutcome, DecodedVisual};
pub use startup::is_decoder_startup_pressure;
pub use track::{VideoDecoderOwner, VideoPlane};

pub const DEFAULT_VIDEO_DECODER_POOL_SIZE: usize = 16;

pub fn take_decoder_pressure() -> Option<u64> {
    match DECODER_PRESSURE_BYTES.swap(0, Ordering::AcqRel) {
        0 => None,
        u64::MAX => Some(u64::MAX),
        bytes => Some(bytes - 1),
    }
}

fn report_decoder_pressure(startup_bytes: u64) {
    DECODER_PRESSURE_BYTES.fetch_max(startup_bytes.saturating_add(1), Ordering::Release);
}

const LOCAL_FORWARD_DECODE_SECONDS: i64 = 1;
const MAX_LATEST_REQUEST_DISTANCE_FRAMES: i64 = 4;
const MAX_HANDOFF_FORWARD_FRAMES: i64 = 120;
const MAX_NONADVANCING_FRAMES: usize = 32;
const DECODER_FREE_MEMORY_RESERVE_DIVISOR: usize = 16;

static NEXT_DECODER_WORKER_ID: AtomicU64 = AtomicU64::new(0);
static NEXT_TEMPORAL_CONSUMER_ID: AtomicU64 = AtomicU64::new(1);
static TEMPORAL_CURRENT_BYTES: AtomicU64 = AtomicU64::new(0);
static TEMPORAL_CURRENT_FRAMES: AtomicU64 = AtomicU64::new(0);
static DECODER_PRESSURE_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use std::process::Command;

    use shrimply_asset::Asset;
    use shrimply_math_core::{Time, fraction_new};
    use shrimply_project_document::project::{CanvasSize, VideoItem, VideoItemContent};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn accurate_out_of_order_requests_map_30fps_positions_to_24fps_frames() {
        let directory = tempfile::tempdir().expect("create decoder cadence test directory");
        let video = directory.path().join("24fps.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=96x64:r=24:d=4",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed to generate the test video");

        let canvas = CanvasSize {
            width: 96,
            height: 64,
        };
        let mut item = VideoItem::background_item(
            canvas,
            Time::from_fraction(393, 2),
            Time::from_fraction(4_938, 25),
        );
        item.content = VideoItemContent::Media;
        item.file = Asset::new(video);
        item.source_width = canvas.width;
        item.source_height = canvas.height;
        item.source_duration = Time::from_seconds(4);
        item.time_offset = Time::from_fraction(103_101_571, 50_000_000);
        item.playback_fps = fraction_new(24, 1);

        let mut pool = VideoDecoderPool::new(1);
        let owner = pool.owner(&[], Uuid::new_v4(), item.id, VideoPlane::Color);
        let decoder = pool.decoder(&item, owner).expect("create test decoder");
        for project_frame in [
            5_897, 5_895, 5_899, 5_896, 5_902, 5_898, 5_909, 5_900, 5_925, 5_910, 5_924, 5_911,
            5_923, 5_912, 5_922,
        ] {
            let project_position = Time::from_fraction(project_frame, 30);
            let requested =
                shrimply_project_document::project::video_source_time_at(&item, project_position)
                    .expect("map project frame to source time");
            let DecodeOutcome::Frame(Some((decoded, _))) = decoder
                .request(DecodeRequest::accurate(requested))
                .expect("request exact decoded frame")
                .receive()
                .expect("receive exact decoded frame")
            else {
                panic!("exact decode returned no frame");
            };
            let source_frame = shrimply_math_core::frame_index(requested, fraction_new(24, 1))
                .expect("map requested source time to source frame");
            assert_eq!(
                decoded,
                Time::from_fraction(source_frame, 24),
                "absolute 30 fps project frame {project_frame} resolved to the wrong offset 24 fps source frame"
            );
        }
    }
}
