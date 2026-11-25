use std::collections::BTreeMap;
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use ffmpeg_next::{self as ffmpeg, packet::Ref as _, sys};
use shrimply_math_core::Time;
use shrimply_visual_frame::VideoFrame;

use crate::session::DecodedVisual;

/// Hard ceiling across RAM + VRAM (1 GB = 1,000,000,000 bytes, strictly <= 1 GiB as well).
pub const MAX_SCRUB_CACHE_BYTES: u64 = 1_000_000_000;
/// Default budget leaves generous headroom below 1 GB for any allocator rounding.
const DEFAULT_SCRUB_CACHE_BYTES: u64 = 900_000_000;
/// Cap compressed GOP packet storage so most of the budget serves decoded VRAM frames,
/// while keeping ~200+ compressed 1080p GOPs in RAM to avoid repeated disk seeks.
const MAX_GOP_PACKET_BUDGET_BYTES: u64 = 256_000_000;
/// Conservative per-entry BTreeMap indexing overhead in bytes.
const INDEX_ENTRY_OVERHEAD_BYTES: u64 = 128;
/// Guard against degenerate single-GOP files exceeding the packet cache limit.
const MAX_PACKETS_PER_GOP: usize = 1024;
/// Bounded container seek/advance retries when locating the GOP covering a target PTS.
const MAX_GOP_SEEK_ATTEMPTS: usize = 3;

const SHRIMPLY_SCRUB_OK: c_int = 0;
const SHRIMPLY_SCRUB_FALLBACK: c_int = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactScrubBackend {
    Nvdec,
    Ffmpeg,
}

pub fn exact_scrub_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SHRIMPLY_EXACT_SCRUB")
            .ok()
            .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
    })
}

pub fn exact_scrub_backend() -> ExactScrubBackend {
    static BACKEND: OnceLock<ExactScrubBackend> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        match std::env::var("SHRIMPLY_EXACT_SCRUB_BACKEND")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ffmpeg") => ExactScrubBackend::Ffmpeg,
            _ => ExactScrubBackend::Nvdec,
        }
    })
}

pub fn exact_scrub_budget_bytes() -> u64 {
    static BUDGET: OnceLock<u64> = OnceLock::new();
    *BUDGET.get_or_init(|| {
        std::env::var("SHRIMPLY_EXACT_SCRUB_BUDGET_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|mb| mb.saturating_mul(1_000_000).min(MAX_SCRUB_CACHE_BYTES))
            .filter(|bytes| *bytes > 0)
            .unwrap_or(DEFAULT_SCRUB_CACHE_BYTES)
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScrubTelemetry {
    pub vram_frames_bytes: u64,
    pub ram_packets_bytes: u64,
    pub index_bytes: u64,
    pub staging_bytes: u64,
    pub total_bytes: u64,
    pub peak_total_bytes: u64,
    pub budget_bytes: u64,
    pub cached_frames: u64,
    pub cached_gops: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub live_surface_hits: u64,
    pub live_dpb_hits: u64,
    pub forward_continuations: u64,
    pub nvdec_decodes: u64,
    pub ffmpeg_fallbacks: u64,
    pub pictures_decoded: u64,
    pub prep_wall_us: u64,
    pub io_us: u64,
    pub parse_us: u64,
    pub nvdec_submit_us: u64,
    pub map_copy_us: u64,
}

static TELEM_VRAM_FRAMES: AtomicU64 = AtomicU64::new(0);
static TELEM_RAM_PACKETS: AtomicU64 = AtomicU64::new(0);
static TELEM_INDEX_BYTES: AtomicU64 = AtomicU64::new(0);
static TELEM_STAGING_BYTES: AtomicU64 = AtomicU64::new(0);
static TELEM_PEAK_TOTAL: AtomicU64 = AtomicU64::new(0);
static TELEM_CACHED_FRAMES: AtomicU64 = AtomicU64::new(0);
static TELEM_CACHED_GOPS: AtomicU64 = AtomicU64::new(0);
static TELEM_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static TELEM_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static TELEM_LIVE_SURFACE_HITS: AtomicU64 = AtomicU64::new(0);
static TELEM_LIVE_DPB_HITS: AtomicU64 = AtomicU64::new(0);
static TELEM_FORWARD_CONT: AtomicU64 = AtomicU64::new(0);
static TELEM_NVDEC_DECODES: AtomicU64 = AtomicU64::new(0);
static TELEM_FFMPEG_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static TELEM_PICTURES_DECODED: AtomicU64 = AtomicU64::new(0);
static TELEM_PREP_WALL_US: AtomicU64 = AtomicU64::new(0);
static TELEM_IO_US: AtomicU64 = AtomicU64::new(0);
static TELEM_PARSE_US: AtomicU64 = AtomicU64::new(0);
static TELEM_NVDEC_SUBMIT_US: AtomicU64 = AtomicU64::new(0);
static TELEM_MAP_COPY_US: AtomicU64 = AtomicU64::new(0);

fn global_total_bytes() -> u64 {
    TELEM_VRAM_FRAMES
        .load(Ordering::Acquire)
        .saturating_add(TELEM_RAM_PACKETS.load(Ordering::Acquire))
        .saturating_add(TELEM_INDEX_BYTES.load(Ordering::Acquire))
        .saturating_add(TELEM_STAGING_BYTES.load(Ordering::Acquire))
}

fn update_peak() {
    TELEM_PEAK_TOTAL.fetch_max(global_total_bytes(), Ordering::AcqRel);
}

fn adjust_atomic(counter: &AtomicU64, prev: u64, next: u64) {
    if next >= prev {
        counter.fetch_add(next - prev, Ordering::AcqRel);
    } else {
        counter.fetch_sub(prev - next, Ordering::AcqRel);
    }
    update_peak();
}

pub fn scrub_telemetry() -> ScrubTelemetry {
    let vram = TELEM_VRAM_FRAMES.load(Ordering::Acquire);
    let ram = TELEM_RAM_PACKETS.load(Ordering::Acquire);
    let index = TELEM_INDEX_BYTES.load(Ordering::Acquire);
    let staging = TELEM_STAGING_BYTES.load(Ordering::Acquire);
    let total = vram
        .saturating_add(ram)
        .saturating_add(index)
        .saturating_add(staging);
    ScrubTelemetry {
        vram_frames_bytes: vram,
        ram_packets_bytes: ram,
        index_bytes: index,
        staging_bytes: staging,
        total_bytes: total,
        peak_total_bytes: TELEM_PEAK_TOTAL.load(Ordering::Acquire).max(total),
        budget_bytes: exact_scrub_budget_bytes(),
        cached_frames: TELEM_CACHED_FRAMES.load(Ordering::Acquire),
        cached_gops: TELEM_CACHED_GOPS.load(Ordering::Acquire),
        cache_hits: TELEM_CACHE_HITS.load(Ordering::Acquire),
        cache_misses: TELEM_CACHE_MISSES.load(Ordering::Acquire),
        live_surface_hits: TELEM_LIVE_SURFACE_HITS.load(Ordering::Acquire),
        live_dpb_hits: TELEM_LIVE_DPB_HITS.load(Ordering::Acquire),
        forward_continuations: TELEM_FORWARD_CONT.load(Ordering::Acquire),
        nvdec_decodes: TELEM_NVDEC_DECODES.load(Ordering::Acquire),
        ffmpeg_fallbacks: TELEM_FFMPEG_FALLBACKS.load(Ordering::Acquire),
        pictures_decoded: TELEM_PICTURES_DECODED.load(Ordering::Acquire),
        prep_wall_us: TELEM_PREP_WALL_US.load(Ordering::Acquire),
        io_us: TELEM_IO_US.load(Ordering::Acquire),
        parse_us: TELEM_PARSE_US.load(Ordering::Acquire),
        nvdec_submit_us: TELEM_NVDEC_SUBMIT_US.load(Ordering::Acquire),
        map_copy_us: TELEM_MAP_COPY_US.load(Ordering::Acquire),
    }
}

pub fn reset_scrub_telemetry() {
    TELEM_CACHE_HITS.store(0, Ordering::Release);
    TELEM_CACHE_MISSES.store(0, Ordering::Release);
    TELEM_LIVE_SURFACE_HITS.store(0, Ordering::Release);
    TELEM_LIVE_DPB_HITS.store(0, Ordering::Release);
    TELEM_FORWARD_CONT.store(0, Ordering::Release);
    TELEM_NVDEC_DECODES.store(0, Ordering::Release);
    TELEM_FFMPEG_FALLBACKS.store(0, Ordering::Release);
    TELEM_PICTURES_DECODED.store(0, Ordering::Release);
    TELEM_PREP_WALL_US.store(0, Ordering::Release);
    TELEM_IO_US.store(0, Ordering::Release);
    TELEM_PARSE_US.store(0, Ordering::Release);
    TELEM_NVDEC_SUBMIT_US.store(0, Ordering::Release);
    TELEM_MAP_COPY_US.store(0, Ordering::Release);
    TELEM_PEAK_TOTAL.store(global_total_bytes(), Ordering::Release);
}

#[repr(C)]
struct ShrimplyNvdecScrub {
    _opaque: [u8; 0],
}

#[repr(C)]
struct ShrimplyGopPacket {
    data: *const u8,
    size: c_int,
    pts: i64,
    dts: i64,
    duration: i64,
}

#[repr(C)]
#[derive(Default)]
struct ShrimplyScrubStats {
    pictures_decoded: c_int,
    live_surface_hit: c_int,
    live_dpb_hit: c_int,
    forward_continued: c_int,
    parse_us: u64,
    decode_us: u64,
    map_copy_us: u64,
}

unsafe extern "C" {
    fn shrimply_nvdec_scrub_new(
        hw_device_ctx: *mut sys::AVBufferRef,
        codecpar: *const sys::AVCodecParameters,
        time_base: sys::AVRational,
    ) -> *mut ShrimplyNvdecScrub;
    fn shrimply_nvdec_scrub_free(ctx: *mut ShrimplyNvdecScrub);
    fn shrimply_nvdec_scrub_flush_bsf(ctx: *mut ShrimplyNvdecScrub);
    fn shrimply_nvdec_scrub_filter_packet(
        ctx: *mut ShrimplyNvdecScrub,
        in_pkt: *const sys::AVPacket,
        out_data: *mut *mut u8,
        out_size: *mut c_int,
    ) -> c_int;
    fn shrimply_nvdec_scrub_staging_bytes(ctx: *const ShrimplyNvdecScrub) -> u64;
    fn shrimply_nvdec_scrub_decode(
        ctx: *mut ShrimplyNvdecScrub,
        gop_id: u64,
        packets: *const ShrimplyGopPacket,
        packet_count: c_int,
        target_packet_index: c_int,
        out_frame: *mut sys::AVFrame,
        stats: *mut ShrimplyScrubStats,
    ) -> c_int;
}

struct GopPacketMeta {
    offset: usize,
    len: usize,
    pts: i64,
    dts: i64,
    duration: i64,
}

struct CachedGop {
    gop_id: u64,
    anchor_pts: i64,
    next_gop_pts: Option<i64>,
    /// Single contiguous Annex-B byte arena for all packets in this GOP.
    blob: Vec<u8>,
    packets: Vec<GopPacketMeta>,
    /// Sorted by display PTS: `(pts, packet_index)` for O(log N) target selection.
    display_order: Vec<(i64, usize)>,
    has_leading_before_anchor: bool,
    bytes: u64,
    last_used: u64,
}

struct CachedFrame {
    end_pts: i64,
    visual: DecodedVisual,
    vram_bytes: u64,
    last_used: u64,
}

pub(crate) enum DirectScrubOutcome {
    Frame(DecodedVisual),
    Fallback,
}

pub(crate) struct ScrubState {
    native: *mut ShrimplyNvdecScrub,
    budget_bytes: u64,
    index_bytes: u64,
    staging_bytes: u64,
    vram_frames_bytes: u64,
    ram_packets_bytes: u64,
    tick: u64,
    next_gop_id: u64,
    /// Keyed by `start_pts` -> cached decoded VRAM frame with proven display interval `[start_pts, end_pts)`.
    frames: BTreeMap<i64, CachedFrame>,
    /// Keyed by `anchor_pts` (actual keyframe PTS) -> cached compressed Annex-B packets for that GOP.
    gops: BTreeMap<i64, CachedGop>,
    /// Reusable FFI packet descriptor scratch buffer.
    ffi_packets: Vec<ShrimplyGopPacket>,
    pub(crate) ffmpeg_cursor_dirty: bool,
}

// ScrubState lives exclusively inside a single VideoDecoderSession worker thread.
unsafe impl Send for ScrubState {}

impl Drop for ScrubState {
    fn drop(&mut self) {
        if !self.native.is_null() {
            unsafe { shrimply_nvdec_scrub_free(self.native) };
            self.native = ptr::null_mut();
        }
        let frames_count = self.frames.len() as u64;
        let gops_count = self.gops.len() as u64;
        self.frames.clear();
        self.gops.clear();
        adjust_atomic(&TELEM_VRAM_FRAMES, self.vram_frames_bytes, 0);
        adjust_atomic(&TELEM_RAM_PACKETS, self.ram_packets_bytes, 0);
        adjust_atomic(&TELEM_INDEX_BYTES, self.index_bytes, 0);
        adjust_atomic(&TELEM_STAGING_BYTES, self.staging_bytes, 0);
        adjust_atomic(&TELEM_CACHED_FRAMES, frames_count, 0);
        adjust_atomic(&TELEM_CACHED_GOPS, gops_count, 0);
    }
}

impl ScrubState {
    pub(crate) fn new(
        hw_device_ctx: *mut sys::AVBufferRef,
        codecpar: *const sys::AVCodecParameters,
        time_base: ffmpeg::Rational,
    ) -> Self {
        let prep_start = Instant::now();
        let budget_bytes = exact_scrub_budget_bytes();
        let native = if exact_scrub_backend() == ExactScrubBackend::Nvdec {
            unsafe { shrimply_nvdec_scrub_new(hw_device_ctx, codecpar, time_base.into()) }
        } else {
            ptr::null_mut()
        };
        let mut state = Self {
            native,
            budget_bytes,
            index_bytes: 0,
            staging_bytes: 0,
            vram_frames_bytes: 0,
            ram_packets_bytes: 0,
            tick: 0,
            next_gop_id: 1,
            frames: BTreeMap::new(),
            gops: BTreeMap::new(),
            ffi_packets: Vec::with_capacity(64),
            ffmpeg_cursor_dirty: false,
        };
        state.sync_staging_bytes();
        let prep_us = prep_start.elapsed().as_micros() as u64;
        TELEM_PREP_WALL_US.fetch_add(prep_us, Ordering::AcqRel);
        state
    }

    fn sync_index_bytes(&mut self) {
        // Account conservatively for BTreeMap node & key/value indexing overhead.
        let entries = (self.frames.len() + self.gops.len()) as u64;
        let next = entries.saturating_mul(INDEX_ENTRY_OVERHEAD_BYTES);
        if next != self.index_bytes {
            adjust_atomic(&TELEM_INDEX_BYTES, self.index_bytes, next);
            self.index_bytes = next;
        }
    }

    fn sync_staging_bytes(&mut self) {
        let next = if self.native.is_null() {
            0
        } else {
            unsafe { shrimply_nvdec_scrub_staging_bytes(self.native) }
        };
        if next != self.staging_bytes {
            adjust_atomic(&TELEM_STAGING_BYTES, self.staging_bytes, next);
            self.staging_bytes = next;
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn evict_to_fit(&mut self, incoming_bytes: u64) {
        let limit = self.budget_bytes;
        while global_total_bytes().saturating_add(incoming_bytes) > limit {
            // Prefer evicting decoded frames if VRAM frames dominate; evict GOPs if packets exceed cap.
            if self.ram_packets_bytes > MAX_GOP_PACKET_BUDGET_BYTES && !self.gops.is_empty() {
                self.evict_lru_gop();
            } else if !self.frames.is_empty() {
                self.evict_lru_frame();
            } else if !self.gops.is_empty() {
                self.evict_lru_gop();
            } else {
                break;
            }
        }
    }

    fn evict_lru_frame(&mut self) {
        let Some(&victim_key) = self
            .frames
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(k, _)| k)
        else {
            return;
        };
        if let Some(removed) = self.frames.remove(&victim_key) {
            let prev = self.vram_frames_bytes;
            self.vram_frames_bytes = prev.saturating_sub(removed.vram_bytes);
            adjust_atomic(&TELEM_VRAM_FRAMES, prev, self.vram_frames_bytes);
            TELEM_CACHED_FRAMES.fetch_sub(1, Ordering::AcqRel);
            self.sync_index_bytes();
        }
    }

    fn evict_lru_gop(&mut self) {
        let Some(&victim_key) = self
            .gops
            .iter()
            .min_by_key(|(_, gop)| gop.last_used)
            .map(|(k, _)| k)
        else {
            return;
        };
        if let Some(removed) = self.gops.remove(&victim_key) {
            let prev = self.ram_packets_bytes;
            self.ram_packets_bytes = prev.saturating_sub(removed.bytes);
            adjust_atomic(&TELEM_RAM_PACKETS, prev, self.ram_packets_bytes);
            TELEM_CACHED_GOPS.fetch_sub(1, Ordering::AcqRel);
            self.sync_index_bytes();
        }
    }

    pub(crate) fn lookup_frame(&mut self, target_pts: i64) -> Option<DecodedVisual> {
        let tick = self.next_tick();
        // Find the greatest start_pts <= target_pts and check target_pts < end_pts.
        let key = self
            .frames
            .range(..=target_pts)
            .next_back()
            .filter(|(_, entry)| target_pts < entry.end_pts)
            .map(|(k, _)| *k)?;
        let entry = self.frames.get_mut(&key)?;
        entry.last_used = tick;
        TELEM_CACHE_HITS.fetch_add(1, Ordering::AcqRel);
        shrimply_profiling::increment("Exact scrub / Frame cache hit");
        Some(entry.visual.clone())
    }

    pub(crate) fn insert_frame(&mut self, start_pts: i64, end_pts: i64, visual: DecodedVisual) {
        if end_pts <= start_pts {
            return;
        }
        let vram_bytes = frame_gpu_bytes(&visual.1);
        if self.frames.contains_key(&start_pts) {
            let tick = self.next_tick();
            if let Some(existing) = self.frames.get_mut(&start_pts) {
                existing.end_pts = existing.end_pts.max(end_pts);
                existing.last_used = tick;
            }
            return;
        }
        let incoming = vram_bytes.saturating_add(INDEX_ENTRY_OVERHEAD_BYTES);
        self.sync_staging_bytes();
        self.sync_index_bytes();
        self.evict_to_fit(incoming);
        if global_total_bytes().saturating_add(incoming) > self.budget_bytes {
            return;
        }
        let tick = self.next_tick();
        let prev = self.vram_frames_bytes;
        self.vram_frames_bytes = prev.saturating_add(vram_bytes);
        adjust_atomic(&TELEM_VRAM_FRAMES, prev, self.vram_frames_bytes);
        TELEM_CACHED_FRAMES.fetch_add(1, Ordering::AcqRel);
        self.frames.insert(
            start_pts,
            CachedFrame {
                end_pts,
                visual,
                vram_bytes,
                last_used: tick,
            },
        );
        self.sync_index_bytes();
    }

    pub(crate) fn record_miss(&self) {
        TELEM_CACHE_MISSES.fetch_add(1, Ordering::AcqRel);
        shrimply_profiling::increment("Exact scrub / Frame cache miss");
    }

    pub(crate) fn record_ffmpeg_fallback(&self) {
        TELEM_FFMPEG_FALLBACKS.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn has_native(&self) -> bool {
        !self.native.is_null()
    }

    fn find_cached_gop(&mut self, target_pts: i64) -> Option<i64> {
        let key = self
            .gops
            .range(..=target_pts)
            .next_back()
            .filter(|(_, gop)| gop.next_gop_pts.is_none_or(|next| target_pts < next))
            .map(|(&k, _)| k)?;
        let tick = self.next_tick();
        if let Some(gop) = self.gops.get_mut(&key) {
            gop.last_used = tick;
        }
        Some(key)
    }

    fn read_gop_from_current_position(
        &mut self,
        input: &mut ffmpeg::format::context::Input,
        stream_index: usize,
        carried_key: &mut Option<ffmpeg::Packet>,
    ) -> Result<Option<CachedGop>, String> {
        unsafe { shrimply_nvdec_scrub_flush_bsf(self.native) };

        let mut blob = Vec::with_capacity(256 * 1024);
        let mut packets = Vec::with_capacity(64);
        let mut first_key_pts: Option<i64> = None;
        let mut detected_next_gop_pts: Option<i64> = None;

        loop {
            let raw_packet = if let Some(pkt) = carried_key.take() {
                pkt
            } else {
                let mut pkt = ffmpeg::Packet::empty();
                match pkt.read(input) {
                    Ok(()) => pkt,
                    Err(ffmpeg::Error::Eof) => {
                        detected_next_gop_pts = None;
                        break;
                    }
                    Err(error) => return Err(format!("read video packet for scrub GOP: {error}")),
                }
            };
            if raw_packet.stream() != stream_index {
                continue;
            }
            let raw_ptr = raw_packet.as_ptr();
            let pts = unsafe { (*raw_ptr).pts };
            let dts = unsafe { (*raw_ptr).dts };
            let duration = unsafe { (*raw_ptr).duration };
            let flags = unsafe { (*raw_ptr).flags };
            let is_key = (flags & sys::AV_PKT_FLAG_KEY) != 0;

            if packets.is_empty() {
                if !is_key || pts == sys::AV_NOPTS_VALUE {
                    return Ok(None);
                }
                first_key_pts = Some(pts);
            } else if is_key {
                if pts != sys::AV_NOPTS_VALUE {
                    detected_next_gop_pts = Some(pts);
                }
                *carried_key = Some(raw_packet);
                break;
            }
            if pts == sys::AV_NOPTS_VALUE {
                return Ok(None);
            }

            let mut out_data: *mut u8 = ptr::null_mut();
            let mut out_size: c_int = 0;
            let f_res = unsafe {
                shrimply_nvdec_scrub_filter_packet(
                    self.native,
                    raw_ptr,
                    &mut out_data,
                    &mut out_size,
                )
            };
            if f_res < 0 || out_data.is_null() || out_size <= 0 {
                return Ok(None);
            }
            let len = out_size as usize;
            let offset = blob.len();
            blob.extend_from_slice(unsafe { std::slice::from_raw_parts(out_data, len) });
            packets.push(GopPacketMeta {
                offset,
                len,
                pts,
                dts,
                duration,
            });
            // Guard against degenerate single-GOP files exceeding packet cache limit.
            if packets.len() > MAX_PACKETS_PER_GOP
                || (blob.len() as u64) > MAX_GOP_PACKET_BUDGET_BYTES / 2
            {
                return Ok(None);
            }
        }

        let Some(first_key_pts) = first_key_pts else {
            return Ok(None);
        };
        if packets.is_empty() {
            return Ok(None);
        }
        blob.shrink_to_fit();

        let mut display_order: Vec<(i64, usize)> = packets
            .iter()
            .enumerate()
            .map(|(idx, p)| (p.pts, idx))
            .collect();
        display_order.sort_unstable_by_key(|&(pts, idx)| (pts, idx));
        let has_leading_before_anchor = display_order
            .first()
            .is_some_and(|&(pts, _)| pts < first_key_pts);

        let metadata_bytes = (packets.capacity() * std::mem::size_of::<GopPacketMeta>()
            + display_order.capacity() * std::mem::size_of::<(i64, usize)>()
            + std::mem::size_of::<CachedGop>()) as u64;
        let total_gop_bytes = (blob.capacity() as u64).saturating_add(metadata_bytes);
        let gop_id = self.next_gop_id;
        self.next_gop_id = self.next_gop_id.wrapping_add(1).max(1);
        let tick = self.next_tick();

        Ok(Some(CachedGop {
            gop_id,
            anchor_pts: first_key_pts,
            next_gop_pts: detected_next_gop_pts,
            blob,
            packets,
            display_order,
            has_leading_before_anchor,
            bytes: total_gop_bytes,
            last_used: tick,
        }))
    }

    fn insert_gop(&mut self, gop: CachedGop) -> Option<i64> {
        let key = gop.anchor_pts;
        if self.gops.contains_key(&key) {
            let tick = self.next_tick();
            if let Some(existing) = self.gops.get_mut(&key) {
                existing.last_used = tick;
            }
            return Some(key);
        }
        let total_gop_bytes = gop.bytes;
        let incoming = total_gop_bytes.saturating_add(INDEX_ENTRY_OVERHEAD_BYTES);
        self.sync_staging_bytes();
        self.sync_index_bytes();
        while self.ram_packets_bytes.saturating_add(total_gop_bytes) > MAX_GOP_PACKET_BUDGET_BYTES
            && !self.gops.is_empty()
        {
            self.evict_lru_gop();
        }
        self.evict_to_fit(incoming);
        if global_total_bytes().saturating_add(incoming) > self.budget_bytes {
            return None;
        }
        let prev = self.ram_packets_bytes;
        self.ram_packets_bytes = prev.saturating_add(total_gop_bytes);
        adjust_atomic(&TELEM_RAM_PACKETS, prev, self.ram_packets_bytes);
        TELEM_CACHED_GOPS.fetch_add(1, Ordering::AcqRel);
        self.gops.insert(key, gop);
        self.sync_index_bytes();
        Some(key)
    }

    fn ensure_gop_for_pts(
        &mut self,
        input: &mut ffmpeg::format::context::Input,
        stream_index: usize,
        target_pts: i64,
    ) -> Result<Option<i64>, String> {
        if let Some(key) = self.find_cached_gop(target_pts) {
            return Ok(Some(key));
        }
        if self.native.is_null() {
            return Ok(None);
        }

        let mut seek_target = target_pts;
        for _attempt in 0..MAX_GOP_SEEK_ATTEMPTS {
            let seek_res = unsafe {
                sys::avformat_seek_file(
                    input.as_mut_ptr(),
                    stream_index as c_int,
                    i64::MIN,
                    seek_target,
                    seek_target,
                    sys::AVSEEK_FLAG_BACKWARD,
                )
            };
            if seek_res < 0 {
                return Ok(None);
            }
            self.ffmpeg_cursor_dirty = true;

            let mut carried_key = None;
            for _advance in 0..MAX_GOP_SEEK_ATTEMPTS {
                let Some(gop) =
                    self.read_gop_from_current_position(input, stream_index, &mut carried_key)?
                else {
                    return Ok(None);
                };
                if target_pts < gop.anchor_pts {
                    let next_seek = gop.anchor_pts.saturating_sub(1);
                    if next_seek >= seek_target || gop.anchor_pts <= i64::MIN + 1 {
                        return Ok(None);
                    }
                    seek_target = next_seek;
                    break;
                }
                if gop.next_gop_pts.is_some_and(|next| target_pts >= next) {
                    // Container seek landed slightly early; advance into the next GOP without re-seeking.
                    let _ = self.insert_gop(gop);
                    if carried_key.is_none() {
                        return Ok(None);
                    }
                    continue;
                }
                return Ok(self.insert_gop(gop));
            }
        }
        Ok(None)
    }

    pub(crate) fn decode_direct(
        &mut self,
        input: &mut ffmpeg::format::context::Input,
        stream_index: usize,
        stream_time_base: ffmpeg::Rational,
        stream_start_time: i64,
        frame_duration: Time,
        target_pts: i64,
    ) -> Result<DirectScrubOutcome, String> {
        if self.native.is_null() {
            return Ok(DirectScrubOutcome::Fallback);
        }
        let io_start = Instant::now();
        let gop_key = match self.ensure_gop_for_pts(input, stream_index, target_pts)? {
            Some(key) => key,
            None => return Ok(DirectScrubOutcome::Fallback),
        };
        TELEM_IO_US.fetch_add(io_start.elapsed().as_micros() as u64, Ordering::AcqRel);

        let Some(gop) = self.gops.get(&gop_key) else {
            return Ok(DirectScrubOutcome::Fallback);
        };
        // Open-GOP streams with leading frames before the anchor keyframe fall back to FFmpeg
        // so cross-GOP references are handled identically to sequential decoding.
        if gop.has_leading_before_anchor {
            return Ok(DirectScrubOutcome::Fallback);
        }
        let pos = gop
            .display_order
            .partition_point(|&(pts, _)| pts <= target_pts);
        let Some(display_idx) = pos.checked_sub(1) else {
            return Ok(DirectScrubOutcome::Fallback);
        };
        let (frame_pts, target_pkt_idx) = gop.display_order[display_idx];
        let end_pts = if let Some(&(next_pts, _)) = gop.display_order.get(display_idx + 1) {
            next_pts
        } else if let Some(next_gop) = gop.next_gop_pts {
            let last_dur = gop.packets[target_pkt_idx].duration;
            if last_dur > 0 && frame_pts.saturating_add(last_dur) < next_gop {
                // Gap before the next GOP anchor indicates possible Open-GOP leading frames in GOP N+1.
                let capped = frame_pts.saturating_add(last_dur);
                if target_pts >= capped {
                    return Ok(DirectScrubOutcome::Fallback);
                }
                capped
            } else {
                next_gop
            }
        } else {
            // Final frame of the stream holds until EOF.
            i64::MAX
        };

        // Avoid re-decoding if this exact resolved frame PTS is already cached (e.g. near EOF).
        if self.frames.contains_key(&frame_pts) {
            let tick = self.next_tick();
            if let Some(existing) = self.frames.get_mut(&frame_pts) {
                existing.end_pts = existing.end_pts.max(end_pts);
                existing.last_used = tick;
                TELEM_CACHE_HITS.fetch_add(1, Ordering::AcqRel);
                shrimply_profiling::increment("Exact scrub / Frame cache hit");
                return Ok(DirectScrubOutcome::Frame(existing.visual.clone()));
            }
        }
        self.record_miss();

        let gop = self.gops.get(&gop_key).expect("cached GOP present");
        let gop_id = gop.gop_id;
        let blob_ptr = gop.blob.as_ptr();
        self.ffi_packets.clear();
        self.ffi_packets
            .extend(gop.packets.iter().map(|p| ShrimplyGopPacket {
                data: unsafe { blob_ptr.add(p.offset) },
                size: p.len as c_int,
                pts: p.pts,
                dts: p.dts,
                duration: p.duration,
            }));

        let mut decoded = ffmpeg::frame::Video::empty();
        let mut stats = ShrimplyScrubStats::default();
        let status = unsafe {
            shrimply_nvdec_scrub_decode(
                self.native,
                gop_id,
                self.ffi_packets.as_ptr(),
                self.ffi_packets.len() as c_int,
                target_pkt_idx as c_int,
                decoded.as_mut_ptr(),
                &mut stats,
            )
        };
        // Reconcile any newly allocated NVDEC surface/parsed-GOP staging bytes immediately.
        self.sync_staging_bytes();
        self.evict_to_fit(0);

        if status == SHRIMPLY_SCRUB_FALLBACK {
            return Ok(DirectScrubOutcome::Fallback);
        }
        if status != SHRIMPLY_SCRUB_OK {
            return Err("direct NVDEC scrub decode failed".to_string());
        }

        TELEM_NVDEC_DECODES.fetch_add(1, Ordering::AcqRel);
        TELEM_PICTURES_DECODED.fetch_add(stats.pictures_decoded.max(0) as u64, Ordering::AcqRel);
        TELEM_PARSE_US.fetch_add(stats.parse_us, Ordering::AcqRel);
        TELEM_NVDEC_SUBMIT_US.fetch_add(stats.decode_us, Ordering::AcqRel);
        TELEM_MAP_COPY_US.fetch_add(stats.map_copy_us, Ordering::AcqRel);
        if stats.live_surface_hit != 0 {
            TELEM_LIVE_SURFACE_HITS.fetch_add(1, Ordering::AcqRel);
        }
        if stats.live_dpb_hit != 0 {
            TELEM_LIVE_DPB_HITS.fetch_add(1, Ordering::AcqRel);
        }
        if stats.forward_continued != 0 {
            TELEM_FORWARD_CONT.fetch_add(1, Ordering::AcqRel);
        }

        let position = crate::math::frame_time(
            &decoded,
            stream_time_base,
            stream_start_time,
            0,
            None,
            frame_duration,
            true,
        )?;
        let frame = Arc::new(VideoFrame::new(decoded)?);
        let visual = (position, frame);
        self.insert_frame(frame_pts, end_pts, visual.clone());
        Ok(DirectScrubOutcome::Frame(visual))
    }
}

fn frame_gpu_bytes(frame: &VideoFrame) -> u64 {
    let raw = unsafe { &*frame.native().as_ptr() };
    let y_pitch = raw.linesize[0].max(0) as u64;
    let uv_pitch = raw.linesize[1].max(0) as u64;
    let h = raw.height.max(0) as u64;
    let uv_h = h.div_ceil(2);
    let pitched = y_pitch
        .saturating_mul(h)
        .saturating_add(uv_pitch.saturating_mul(uv_h));
    pitched.max(frame.bytes())
}
