#include <ffnvcodec/dynlink_loader.h>
#include <libavcodec/avcodec.h>
#include <libavcodec/bsf.h>
#include <libavutil/buffer.h>
#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_cuda.h>
#include <libavutil/pixfmt.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static uint64_t shrimply_now_us(void) {
#if defined(_WIN32)
    return 0;
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000ULL + (uint64_t)(ts.tv_nsec / 1000L);
#endif
}

#define SHRIMPLY_SCRUB_OK        0
#define SHRIMPLY_SCRUB_FALLBACK  1
#define SHRIMPLY_SCRUB_ERROR    -1

#define SHRIMPLY_MAX_SLOTS         32
#define SHRIMPLY_MAX_REFS          16
#define SHRIMPLY_RBSP_HEADER_BYTES 512
#define SHRIMPLY_EXTRA_SURFACES    4
#define SHRIMPLY_SURFACE_BUDGET_BYTES (140ULL * 1000ULL * 1000ULL)

typedef struct ShrimplyGopPacket {
    const uint8_t *data;
    int size;
    int64_t pts;
    int64_t dts;
    int64_t duration;
} ShrimplyGopPacket;

typedef struct ShrimplyScrubStats {
    int pictures_decoded;
    int live_surface_hit;
    int live_dpb_hit;
    int forward_continued;
    uint64_t parse_us;
    uint64_t decode_us;
    uint64_t map_copy_us;
} ShrimplyScrubStats;

typedef struct ParsedPic {
    CUVIDPICPARAMS params;
    size_t bs_offset;
    size_t slice_offset;
    int packet_index;
    int64_t pts;
    int64_t dts;
    int64_t duration;
    int ref_count;
    int refs[SHRIMPLY_MAX_REFS];
    int ref_slots[SHRIMPLY_MAX_REFS];
} ParsedPic;

typedef struct ShrimplyNvdecScrub {
    CudaFunctions *cu;
    CuvidFunctions *cv;
    AVBufferRef *hw_device_ref;
    AVBufferRef *hw_frames_ref;
    CUcontext cuda_ctx;
    CUstream stream;
    CUvideodecoder decoder;
    CUvideoparser parser;
    AVBSFContext *bsf;
    AVPacket *bsf_pkt;

    enum AVCodecID codec_id;
    cudaVideoCodec nv_codec;
    AVRational time_base;
    int width;
    int height;
    int coded_width;
    int coded_height;
    int bit_depth_minus8;
    int num_surfaces;
    enum AVColorRange color_range;
    enum AVColorSpace color_space;
    enum AVColorPrimaries color_primaries;
    enum AVColorTransferCharacteristic color_trc;
    enum AVChromaLocation chroma_location;
    AVRational sample_aspect_ratio;

    /* Parsed GOP state (cached for the most recently parsed GOP, backed by reusable arenas). */
    uint64_t parsed_gop_id;
    int parsed_packets;
    int parsed_pic_count;
    int parsed_pic_cap;
    ParsedPic *parsed_pics;
    uint8_t *bs_arena;
    size_t bs_arena_len;
    size_t bs_arena_cap;
    unsigned *slice_arena;
    size_t slice_arena_len;
    size_t slice_arena_cap;
    int parse_slot_owner[SHRIMPLY_MAX_SLOTS];
    int current_packet_index;
    int64_t current_pts;
    int64_t current_dts;
    int64_t current_duration;
    int parse_error;
    int parse_fallback;

    /* Live NVDEC surface occupancy & monotonic reference DPB frontier across calls. */
    uint64_t slot_gop_id[SHRIMPLY_MAX_SLOTS];
    int slot_pic_idx[SHRIMPLY_MAX_SLOTS];
    int slot_valid[SHRIMPLY_MAX_SLOTS];
    uint64_t live_dpb_gop_id;
    int live_dpb_last_ref_pic;

    /* Scratch buffers reused across requests (avoid per-seek heap churn). */
    int scratch_cap;
    uint8_t *closure_mark;
    uint8_t *decode_mark;
    int *closure_list;
    uint64_t staging_bytes;
} ShrimplyNvdecScrub;

/* -------------------------------------------------------------------------
 * ITU-T H.264 §8.2.4 RefPicList0 / RefPicList1 slice-header dependency parser
 * ------------------------------------------------------------------------- */

typedef struct BitCursor {
    uint8_t bytes[SHRIMPLY_RBSP_HEADER_BYTES];
    unsigned size;
    unsigned bit;
    int error;
} BitCursor;

static void rbsp_extract(BitCursor *b, const uint8_t *src, unsigned len) {
    unsigned zeroes = 0;
    for (unsigned i = 0; i < len && b->size < sizeof(b->bytes); i++) {
        if (zeroes == 2 && src[i] == 3) {
            zeroes = 0;
            continue;
        }
        b->bytes[b->size++] = src[i];
        zeroes = src[i] ? 0 : zeroes + 1;
    }
}

static unsigned read_bits(BitCursor *b, unsigned count) {
    if (count > 30 || b->bit + count > b->size * 8) {
        b->error = 1;
        return 0;
    }
    unsigned value = 0;
    for (unsigned i = 0; i < count; i++, b->bit++) {
        value = (value << 1) | ((b->bytes[b->bit / 8] >> (7 - b->bit % 8)) & 1U);
    }
    return value;
}

static unsigned read_ue(BitCursor *b) {
    unsigned zeroes = 0;
    while (!read_bits(b, 1) && !b->error) {
        if (++zeroes > 30) {
            b->error = 1;
            return 0;
        }
    }
    return ((1U << zeroes) - 1U) + read_bits(b, zeroes);
}

static int dpb_poc(const CUVIDH264DPBENTRY *e) {
    return e->FieldOrderCnt[0] < e->FieldOrderCnt[1] ? e->FieldOrderCnt[0] : e->FieldOrderCnt[1];
}

static int h264_ordered_before(const CUVIDH264DPBENTRY *a, const CUVIDH264DPBENTRY *b,
                               int bidir, int cur_frame_num, int max_frame_num,
                               int cur_poc, int list) {
    if (a->is_long_term != b->is_long_term)
        return !a->is_long_term;
    if (a->is_long_term)
        return a->FrameIdx < b->FrameIdx;
    if (!bidir) {
        int an = a->FrameIdx - (a->FrameIdx > cur_frame_num ? max_frame_num : 0);
        int bn = b->FrameIdx - (b->FrameIdx > cur_frame_num ? max_frame_num : 0);
        return an > bn;
    }
    int ap = dpb_poc(a), bp = dpb_poc(b);
    int past_a = ap < cur_poc, past_b = bp < cur_poc;
    if (past_a != past_b)
        return list ? !past_a : past_a;
    return past_a ? (ap > bp) : (ap < bp);
}

static int h264_slice_references(const CUVIDPICPARAMS *picture, const uint8_t *nal,
                                 unsigned length, unsigned *required) {
    const CUVIDH264PICPARAMS *p = &picture->CodecSpecific.h264;
    if (picture->field_pic_flag || !p->frame_mbs_only_flag ||
        p->residual_colour_transform_flag || p->log2_max_frame_num_minus4 < 0 ||
        p->log2_max_frame_num_minus4 > 12)
        return 0;

    unsigned start = 0;
    while (start < length && nal[start] == 0) start++;
    if (start < length && nal[start] == 1 && start >= 2) start++;
    if (start >= length) return 0;

    unsigned nal_type = nal[start] & 31U;
    if (nal_type != 1 && nal_type != 5) return 0;

    BitCursor b = {0};
    rbsp_extract(&b, nal + start + 1, length - start - 1);
    (void)read_ue(&b); /* first_mb_in_slice */
    unsigned raw_slice = read_ue(&b);
    if (raw_slice > 9) return 0;
    unsigned slice = raw_slice % 5;
    if (slice == 2) return !b.error; /* I-slice */
    if (slice > 1 || nal_type == 5) return 0; /* SI/SP or inter IDR unsupported */

    (void)read_ue(&b); /* pic_parameter_set_id */
    unsigned frame_bits = (unsigned)(4 + p->log2_max_frame_num_minus4);
    unsigned frame_num = read_bits(&b, frame_bits);
    if ((int)frame_num != p->frame_num) return 0;

    if (p->pic_order_cnt_type == 0) {
        if (p->log2_max_pic_order_cnt_lsb_minus4 < 0 || p->log2_max_pic_order_cnt_lsb_minus4 > 12)
            return 0;
        (void)read_bits(&b, (unsigned)(4 + p->log2_max_pic_order_cnt_lsb_minus4));
        if (p->pic_order_present_flag) (void)read_ue(&b);
    } else if (p->pic_order_cnt_type == 1 && !p->delta_pic_order_always_zero_flag) {
        (void)read_ue(&b);
        if (p->pic_order_present_flag) (void)read_ue(&b);
    }
    if (p->redundant_pic_cnt_present_flag) (void)read_ue(&b);

    int bidir = (slice == 1);
    if (bidir) (void)read_bits(&b, 1); /* direct_spatial_mv_pred_flag */

    unsigned count[2] = {
        (unsigned)(1 + p->num_ref_idx_l0_active_minus1),
        (unsigned)(1 + p->num_ref_idx_l1_active_minus1)
    };
    if (read_bits(&b, 1)) {
        count[0] = 1 + read_ue(&b);
        if (bidir) count[1] = 1 + read_ue(&b);
    }

    int max_pic_num = 1 << frame_bits;
    int cur_poc = p->CurrFieldOrderCnt[0];
    if (p->CurrFieldOrderCnt[1] < cur_poc) cur_poc = p->CurrFieldOrderCnt[1];

    int lists[2][16];
    int entries = 0;
    for (int list = 0; list < 1 + bidir; list++) {
        int n = 0;
        for (int i = 0; i < 16; i++) {
            const CUVIDH264DPBENTRY *e = &p->dpb[i];
            if (!e->used_for_reference) continue;
            int j = n++;
            while (j > 0 && h264_ordered_before(e, &p->dpb[lists[list][j - 1]], bidir,
                                                (int)frame_num, max_pic_num, cur_poc, list)) {
                lists[list][j] = lists[list][j - 1];
                j--;
            }
            lists[list][j] = i;
        }
        entries = n;
    }
    if (bidir && entries > 1 && memcmp(lists[0], lists[1], (size_t)entries * sizeof(int)) == 0) {
        int swap = lists[1][0];
        lists[1][0] = lists[1][1];
        lists[1][1] = swap;
    }

    for (int list = 0; list < 1 + bidir; list++) {
        if ((int)count[list] > entries) return 0;
        if (read_bits(&b, 1)) {
            unsigned pred = frame_num;
            for (unsigned index = 0;; index++) {
                unsigned mod_type = read_ue(&b);
                if (mod_type == 3) break;
                if (b.error || mod_type > 2 || index >= count[list]) return 0;
                unsigned val = read_ue(&b);
                int id;
                if (mod_type == 2) {
                    id = (int)val;
                } else {
                    if (val >= (unsigned)max_pic_num) return 0;
                    pred = (pred + (mod_type == 0 ? (0U - (val + 1U)) : (val + 1U))) & (unsigned)(max_pic_num - 1);
                    id = (int)pred;
                }
                int match = -1;
                for (int j = 0; j < 16; j++) {
                    if (p->dpb[j].used_for_reference &&
                        (!!p->dpb[j].is_long_term == (mod_type == 2)) &&
                        p->dpb[j].FrameIdx == id) {
                        match = j;
                        break;
                    }
                }
                if (match < 0) return 0;
                int j = entries - 1;
                for (int k = (int)index; k < entries; k++) {
                    if (lists[list][k] == match) {
                        j = k;
                        break;
                    }
                }
                for (; j > (int)index; j--) lists[list][j] = lists[list][j - 1];
                lists[list][index] = match;
            }
        }
        for (unsigned i = 0; i < count[list]; i++) {
            *required |= (1U << lists[list][i]);
        }
    }
    return !b.error;
}

static int h264_picture_references(const CUVIDPICPARAMS *picture, unsigned *required) {
    *required = 0;
    if (!picture->nNumSlices) return 0;
    for (unsigned s = 0; s < picture->nNumSlices; s++) {
        unsigned start = picture->pSliceDataOffsets[s];
        unsigned end = (s + 1 < picture->nNumSlices)
                           ? picture->pSliceDataOffsets[s + 1]
                           : picture->nBitstreamDataLen;
        if (start >= end || end > picture->nBitstreamDataLen ||
            !h264_slice_references(picture, picture->pBitstreamData + start, end - start, required))
            return 0;
    }
    return 1;
}

static int hevc_picture_ref_slots(const CUVIDPICPARAMS *picture, int *out_slots, int *out_count) {
    const CUVIDHEVCPICPARAMS *h = &picture->CodecSpecific.hevc;
    int count = 0;
    if (picture->intra_pic_flag) {
        *out_count = 0;
        return 1;
    }
    const unsigned char *sets[3] = {
        h->RefPicSetStCurrBefore,
        h->RefPicSetStCurrAfter,
        h->RefPicSetLtCurr,
    };
    const int lens[3] = {
        h->NumPocStCurrBefore,
        h->NumPocStCurrAfter,
        h->NumPocLtCurr,
    };
    for (int s = 0; s < 3; s++) {
        if (lens[s] < 0 || lens[s] > 8) return 0;
        for (int i = 0; i < lens[s]; i++) {
            unsigned r = sets[s][i];
            if (r >= 16) return 0;
            int slot = h->RefPicIdx[r];
            if (slot < 0 || slot >= SHRIMPLY_MAX_SLOTS) return 0;
            int dup = 0;
            for (int j = 0; j < count; j++) {
                if (out_slots[j] == slot) {
                    dup = 1;
                    break;
                }
            }
            if (!dup) {
                if (count >= SHRIMPLY_MAX_REFS) return 0;
                out_slots[count++] = slot;
            }
        }
    }
    *out_count = count;
    return 1;
}

/* -------------------------------------------------------------------------
 * Decoder & Parser lifecycle
 * ------------------------------------------------------------------------- */

static void invalidate_live_slots(ShrimplyNvdecScrub *ctx) {
    memset(ctx->slot_valid, 0, sizeof(ctx->slot_valid));
    memset(ctx->slot_gop_id, 0, sizeof(ctx->slot_gop_id));
    for (int i = 0; i < SHRIMPLY_MAX_SLOTS; i++) {
        ctx->slot_pic_idx[i] = -1;
    }
    ctx->live_dpb_gop_id = 0;
    ctx->live_dpb_last_ref_pic = -1;
}

static void clear_parsed_gop(ShrimplyNvdecScrub *ctx) {
    ctx->parsed_pic_count = 0;
    ctx->parsed_packets = 0;
    ctx->parsed_gop_id = 0;
    ctx->bs_arena_len = 0;
    ctx->slice_arena_len = 0;
    for (int i = 0; i < SHRIMPLY_MAX_SLOTS; i++) {
        ctx->parse_slot_owner[i] = -1;
    }
}

static int ensure_arenas(ShrimplyNvdecScrub *ctx, size_t bs_add, size_t slice_add) {
    if (ctx->bs_arena_len + bs_add > ctx->bs_arena_cap) {
        size_t next_cap = ctx->bs_arena_cap ? ctx->bs_arena_cap : (256U * 1024U);
        while (next_cap < ctx->bs_arena_len + bs_add) {
            next_cap *= 2U;
        }
        uint8_t *next = (uint8_t *)realloc(ctx->bs_arena, next_cap);
        if (!next) return -1;
        ctx->bs_arena = next;
        ctx->bs_arena_cap = next_cap;
    }
    if (ctx->slice_arena_len + slice_add > ctx->slice_arena_cap) {
        size_t next_cap = ctx->slice_arena_cap ? ctx->slice_arena_cap : 256U;
        while (next_cap < ctx->slice_arena_len + slice_add) {
            next_cap *= 2U;
        }
        unsigned *next = (unsigned *)realloc(ctx->slice_arena, next_cap * sizeof(unsigned));
        if (!next) return -1;
        ctx->slice_arena = next;
        ctx->slice_arena_cap = next_cap;
    }
    return 0;
}

static int ensure_hw_frames(ShrimplyNvdecScrub *ctx) {
    enum AVPixelFormat sw_fmt = ctx->bit_depth_minus8 ? AV_PIX_FMT_P010LE : AV_PIX_FMT_NV12;
    if (ctx->hw_frames_ref) {
        AVHWFramesContext *fc = (AVHWFramesContext *)ctx->hw_frames_ref->data;
        if (fc->width == ctx->width && fc->height == ctx->height && fc->sw_format == sw_fmt) {
            return 0;
        }
        av_buffer_unref(&ctx->hw_frames_ref);
    }
    AVBufferRef *ref = av_hwframe_ctx_alloc(ctx->hw_device_ref);
    if (!ref) return -1;
    AVHWFramesContext *fc = (AVHWFramesContext *)ref->data;
    fc->format = AV_PIX_FMT_CUDA;
    fc->sw_format = sw_fmt;
    fc->width = ctx->width;
    fc->height = ctx->height;
    fc->initial_pool_size = 0;
    if (av_hwframe_ctx_init(ref) < 0) {
        av_buffer_unref(&ref);
        return -1;
    }
    ctx->hw_frames_ref = ref;
    return 0;
}

/* Returns allocated surface count (>0), 0 on unsupported format (fallback), or -1 on error. */
static int ensure_decoder(ShrimplyNvdecScrub *ctx, int coded_w, int coded_h,
                          int disp_left, int disp_top, int disp_right, int disp_bottom,
                          int bit_depth_minus8, int min_surf) {
    int disp_w = disp_right - disp_left;
    int disp_h = disp_bottom - disp_top;
    if (disp_w <= 0) disp_w = coded_w;
    if (disp_h <= 0) disp_h = coded_h;
    if (coded_w <= 0 || coded_h <= 0 || disp_w <= 0 || disp_h <= 0) {
        return 0;
    }

    if (ctx->decoder &&
        ctx->coded_width == coded_w &&
        ctx->coded_height == coded_h &&
        ctx->width == disp_w &&
        ctx->height == disp_h &&
        ctx->bit_depth_minus8 == bit_depth_minus8 &&
        ctx->num_surfaces >= min_surf + 2) {
        return ctx->num_surfaces;
    }

    uint64_t bytes_per_sample = bit_depth_minus8 ? 2ULL : 1ULL;
    uint64_t one_surface_bytes =
        (uint64_t)coded_w * (uint64_t)coded_h * 3ULL / 2ULL * bytes_per_sample;
    int desired = one_surface_bytes > 0
        ? (int)(SHRIMPLY_SURFACE_BUDGET_BYTES / one_surface_bytes)
        : (min_surf + SHRIMPLY_EXTRA_SURFACES);
    if (desired < min_surf + SHRIMPLY_EXTRA_SURFACES) {
        desired = min_surf + SHRIMPLY_EXTRA_SURFACES;
    }
    if (desired > SHRIMPLY_MAX_SLOTS) desired = SHRIMPLY_MAX_SLOTS;

    if (ctx->decoder) {
        ctx->cv->cuvidDestroyDecoder(ctx->decoder);
        ctx->decoder = NULL;
        invalidate_live_slots(ctx);
    }

    CUVIDDECODECAPS caps = {0};
    caps.eCodecType = ctx->nv_codec;
    caps.eChromaFormat = cudaVideoChromaFormat_420;
    caps.nBitDepthMinus8 = bit_depth_minus8;
    if (ctx->cv->cuvidGetDecoderCaps(&caps) != CUDA_SUCCESS || !caps.bIsSupported) {
        return 0;
    }

    CUVIDDECODECREATEINFO cfg = {0};
    cfg.ulWidth = (unsigned long)coded_w;
    cfg.ulHeight = (unsigned long)coded_h;
    cfg.ulNumDecodeSurfaces = (unsigned long)desired;
    cfg.CodecType = ctx->nv_codec;
    cfg.ChromaFormat = cudaVideoChromaFormat_420;
    cfg.ulCreationFlags = cudaVideoCreate_PreferCUVID;
    cfg.bitDepthMinus8 = bit_depth_minus8;
    cfg.display_area.left = (short)disp_left;
    cfg.display_area.top = (short)disp_top;
    cfg.display_area.right = (short)disp_right;
    cfg.display_area.bottom = (short)disp_bottom;
    cfg.OutputFormat = bit_depth_minus8 ? cudaVideoSurfaceFormat_P016 : cudaVideoSurfaceFormat_NV12;
    cfg.DeinterlaceMode = cudaVideoDeinterlaceMode_Weave;
    cfg.ulTargetWidth = (unsigned long)disp_w;
    cfg.ulTargetHeight = (unsigned long)disp_h;
    cfg.ulNumOutputSurfaces = 1;

    if (ctx->cv->cuvidCreateDecoder(&ctx->decoder, &cfg) != CUDA_SUCCESS) {
        return -1;
    }

    ctx->coded_width = coded_w;
    ctx->coded_height = coded_h;
    ctx->width = disp_w;
    ctx->height = disp_h;
    ctx->bit_depth_minus8 = bit_depth_minus8;
    ctx->num_surfaces = desired;
    invalidate_live_slots(ctx);

    if (ensure_hw_frames(ctx) < 0) {
        return -1;
    }

    ctx->staging_bytes = one_surface_bytes * (uint64_t)(ctx->num_surfaces + 1);
    return desired;
}

static int CUDAAPI on_sequence(void *opaque, CUVIDEOFORMAT *fmt) {
    ShrimplyNvdecScrub *ctx = (ShrimplyNvdecScrub *)opaque;
    if (!fmt->progressive_sequence ||
        fmt->chroma_format != cudaVideoChromaFormat_420 ||
        (fmt->bit_depth_luma_minus8 != 0 && fmt->bit_depth_luma_minus8 != 2)) {
        ctx->parse_fallback = 1;
        return 0;
    }

    int surfaces = ensure_decoder(
        ctx,
        (int)fmt->coded_width,
        (int)fmt->coded_height,
        fmt->display_area.left,
        fmt->display_area.top,
        fmt->display_area.right,
        fmt->display_area.bottom,
        fmt->bit_depth_luma_minus8,
        fmt->min_num_decode_surfaces
    );
    if (surfaces == 0) {
        ctx->parse_fallback = 1;
        return 0;
    }
    if (surfaces < 0) {
        ctx->parse_error = 1;
        return 0;
    }

    int has_vui_signal =
        fmt->video_signal_description.video_full_range_flag != 0 ||
        (fmt->video_signal_description.color_primaries != 0 &&
         fmt->video_signal_description.color_primaries != 2) ||
        (fmt->video_signal_description.transfer_characteristics != 0 &&
         fmt->video_signal_description.transfer_characteristics != 2) ||
        fmt->video_signal_description.matrix_coefficients != 2;
    if (ctx->color_range == AVCOL_RANGE_UNSPECIFIED && has_vui_signal) {
        ctx->color_range = fmt->video_signal_description.video_full_range_flag
                               ? AVCOL_RANGE_JPEG
                               : AVCOL_RANGE_MPEG;
    }
    if (ctx->color_primaries == AVCOL_PRI_UNSPECIFIED &&
        fmt->video_signal_description.color_primaries != 0 &&
        fmt->video_signal_description.color_primaries != 2) {
        ctx->color_primaries = (enum AVColorPrimaries)fmt->video_signal_description.color_primaries;
    }
    if (ctx->color_trc == AVCOL_TRC_UNSPECIFIED &&
        fmt->video_signal_description.transfer_characteristics != 0 &&
        fmt->video_signal_description.transfer_characteristics != 2) {
        ctx->color_trc = (enum AVColorTransferCharacteristic)fmt->video_signal_description.transfer_characteristics;
    }
    if (ctx->color_space == AVCOL_SPC_UNSPECIFIED &&
        fmt->video_signal_description.matrix_coefficients != 2) {
        ctx->color_space = (enum AVColorSpace)fmt->video_signal_description.matrix_coefficients;
    }
    if (ctx->chroma_location == AVCHROMA_LOC_UNSPECIFIED) {
        ctx->chroma_location = AVCHROMA_LOC_LEFT;
    }
    return surfaces;
}

static int CUDAAPI on_parse_decode(void *opaque, CUVIDPICPARAMS *pic) {
    ShrimplyNvdecScrub *ctx = (ShrimplyNvdecScrub *)opaque;
    if (ctx->parse_error || ctx->parse_fallback) return 0;
    if (pic->field_pic_flag || pic->CurrPicIdx < 0 || pic->CurrPicIdx >= ctx->num_surfaces) {
        ctx->parse_fallback = 1;
        return 0;
    }

    if (ctx->parsed_pic_count >= ctx->parsed_pic_cap) {
        int next_cap = ctx->parsed_pic_cap ? ctx->parsed_pic_cap * 2 : 128;
        ParsedPic *next = (ParsedPic *)realloc(ctx->parsed_pics, (size_t)next_cap * sizeof(ParsedPic));
        if (!next) {
            ctx->parse_error = 1;
            return 0;
        }
        ctx->parsed_pics = next;
        ctx->parsed_pic_cap = next_cap;
    }

    int idx = ctx->parsed_pic_count;
    ParsedPic *dst = &ctx->parsed_pics[idx];
    memset(dst, 0, sizeof(*dst));
    dst->params = *pic;
    dst->packet_index = ctx->current_packet_index;
    dst->pts = ctx->current_pts;
    dst->dts = ctx->current_dts;
    dst->duration = ctx->current_duration;

    if (pic->nBitstreamDataLen == 0 || pic->nNumSlices == 0) {
        ctx->parse_fallback = 1;
        return 0;
    }
    if (ensure_arenas(ctx, pic->nBitstreamDataLen, pic->nNumSlices) < 0) {
        ctx->parse_error = 1;
        return 0;
    }
    dst->bs_offset = ctx->bs_arena_len;
    dst->slice_offset = ctx->slice_arena_len;
    memcpy(ctx->bs_arena + dst->bs_offset, pic->pBitstreamData, pic->nBitstreamDataLen);
    memcpy(ctx->slice_arena + dst->slice_offset, pic->pSliceDataOffsets,
           pic->nNumSlices * sizeof(unsigned));
    ctx->bs_arena_len += pic->nBitstreamDataLen;
    ctx->slice_arena_len += pic->nNumSlices;
    dst->params.pBitstreamData = ctx->bs_arena + dst->bs_offset;
    dst->params.pSliceDataOffsets = ctx->slice_arena + dst->slice_offset;

    if (ctx->codec_id == AV_CODEC_ID_H264) {
        unsigned required_mask = 0;
        if (!h264_picture_references(&dst->params, &required_mask)) goto fallback;
        for (int i = 0; i < 16; i++) {
            if (!(required_mask & (1U << i))) continue;
            int slot = dst->params.CodecSpecific.h264.dpb[i].PicIdx;
            if (slot < 0 || slot >= SHRIMPLY_MAX_SLOTS || ctx->parse_slot_owner[slot] < 0) {
                /* Open-GOP leading picture or missing prior reference -> safe fallback. */
                goto fallback;
            }
            int dep_pic = ctx->parse_slot_owner[slot];
            int dup = 0;
            for (int j = 0; j < dst->ref_count; j++) {
                if (dst->refs[j] == dep_pic) {
                    dup = 1;
                    break;
                }
            }
            if (!dup && dst->ref_count < SHRIMPLY_MAX_REFS) {
                dst->refs[dst->ref_count] = dep_pic;
                dst->ref_slots[dst->ref_count] = slot;
                dst->ref_count++;
            }
        }
    } else if (ctx->codec_id == AV_CODEC_ID_HEVC) {
        int slots[SHRIMPLY_MAX_REFS];
        int scount = 0;
        if (!hevc_picture_ref_slots(&dst->params, slots, &scount)) goto fallback;
        for (int i = 0; i < scount; i++) {
            int slot = slots[i];
            if (ctx->parse_slot_owner[slot] < 0) goto fallback;
            dst->refs[dst->ref_count] = ctx->parse_slot_owner[slot];
            dst->ref_slots[dst->ref_count] = slot;
            dst->ref_count++;
        }
    } else {
        goto fallback;
    }

    ctx->parse_slot_owner[pic->CurrPicIdx] = idx;
    ctx->parsed_pic_count++;
    return 1;

fallback:
    ctx->parse_fallback = 1;
    ctx->bs_arena_len = dst->bs_offset;
    ctx->slice_arena_len = dst->slice_offset;
    return 0;
}

static int CUDAAPI on_display(void *opaque, CUVIDPARSERDISPINFO *disp) {
    (void)opaque;
    (void)disp;
    return 1;
}

static int reset_parser(ShrimplyNvdecScrub *ctx) {
    if (ctx->parser) {
        ctx->cv->cuvidDestroyVideoParser(ctx->parser);
        ctx->parser = NULL;
    }
    CUVIDPARSERPARAMS params = {0};
    params.CodecType = ctx->nv_codec;
    params.ulMaxNumDecodeSurfaces = ctx->num_surfaces > 0 ? (unsigned)ctx->num_surfaces : (unsigned)SHRIMPLY_MAX_SLOTS;
    params.ulMaxDisplayDelay = 0;
    params.pUserData = ctx;
    params.pfnSequenceCallback = on_sequence;
    params.pfnDecodePicture = on_parse_decode;
    params.pfnDisplayPicture = on_display;
    if (ctx->cv->cuvidCreateVideoParser(&ctx->parser, &params) != CUDA_SUCCESS) {
        return -1;
    }
    return 0;
}

static int ensure_scratch(ShrimplyNvdecScrub *ctx, int n) {
    if (n <= ctx->scratch_cap) return 0;
    int cap = ctx->scratch_cap ? ctx->scratch_cap : 128;
    while (cap < n) cap *= 2;
    uint8_t *cm = (uint8_t *)realloc(ctx->closure_mark, (size_t)cap);
    if (!cm) return -1;
    ctx->closure_mark = cm;
    uint8_t *dm = (uint8_t *)realloc(ctx->decode_mark, (size_t)cap);
    if (!dm) return -1;
    ctx->decode_mark = dm;
    int *cl = (int *)realloc(ctx->closure_list, (size_t)cap * sizeof(int));
    if (!cl) return -1;
    ctx->closure_list = cl;
    ctx->scratch_cap = cap;
    return 0;
}

static int map_and_copy_surface(ShrimplyNvdecScrub *ctx, int pic_idx, const ParsedPic *pic,
                                AVFrame *out_frame) {
    av_frame_unref(out_frame);
    if (av_hwframe_get_buffer(ctx->hw_frames_ref, out_frame, 0) < 0) {
        return -1;
    }

    CUVIDPROCPARAMS vpp = {0};
    vpp.progressive_frame = 1;
    vpp.output_stream = ctx->stream;

    CUdeviceptr src_ptr = 0;
    unsigned src_pitch = 0;
    if (ctx->cv->cuvidMapVideoFrame(ctx->decoder, pic_idx, &src_ptr, &src_pitch, &vpp) != CUDA_SUCCESS) {
        av_frame_unref(out_frame);
        return -1;
    }

    CUVIDGETDECODESTATUS status = {0};
    CUresult status_res = ctx->cv->cuvidGetDecodeStatus(ctx->decoder, pic_idx, &status);
    if (status_res == CUDA_SUCCESS &&
        status.decodeStatus != cuvidDecodeStatus_Success &&
        status.decodeStatus != cuvidDecodeStatus_InProgress) {
        ctx->cv->cuvidUnmapVideoFrame(ctx->decoder, src_ptr);
        av_frame_unref(out_frame);
        return -1;
    }

    size_t row_bytes = (size_t)ctx->width * (ctx->bit_depth_minus8 ? 2U : 1U);
    size_t y_h = (size_t)ctx->height;
    size_t uv_h = (size_t)((ctx->height + 1) / 2);

    CUDA_MEMCPY2D copy_y = {0};
    copy_y.srcMemoryType = CU_MEMORYTYPE_DEVICE;
    copy_y.srcDevice = src_ptr;
    copy_y.srcPitch = src_pitch;
    copy_y.dstMemoryType = CU_MEMORYTYPE_DEVICE;
    copy_y.dstDevice = (CUdeviceptr)(uintptr_t)out_frame->data[0];
    copy_y.dstPitch = (size_t)out_frame->linesize[0];
    copy_y.WidthInBytes = row_bytes;
    copy_y.Height = y_h;

    CUDA_MEMCPY2D copy_uv = {0};
    copy_uv.srcMemoryType = CU_MEMORYTYPE_DEVICE;
    copy_uv.srcDevice = src_ptr + (CUdeviceptr)src_pitch * (CUdeviceptr)ctx->height;
    copy_uv.srcPitch = src_pitch;
    copy_uv.dstMemoryType = CU_MEMORYTYPE_DEVICE;
    copy_uv.dstDevice = (CUdeviceptr)(uintptr_t)out_frame->data[1];
    copy_uv.dstPitch = (size_t)out_frame->linesize[1];
    copy_uv.WidthInBytes = row_bytes;
    copy_uv.Height = uv_h;

    CUresult c1 = ctx->cu->cuMemcpy2DAsync(&copy_y, ctx->stream);
    CUresult c2 = (c1 == CUDA_SUCCESS) ? ctx->cu->cuMemcpy2DAsync(&copy_uv, ctx->stream) : c1;
    CUresult c3 = (c2 == CUDA_SUCCESS) ? ctx->cu->cuStreamSynchronize(ctx->stream) : c2;
    ctx->cv->cuvidUnmapVideoFrame(ctx->decoder, src_ptr);

    if (c3 != CUDA_SUCCESS) {
        av_frame_unref(out_frame);
        return -1;
    }

    out_frame->width = ctx->width;
    out_frame->height = ctx->height;
    out_frame->pts = pic->pts;
    out_frame->best_effort_timestamp = pic->pts;
    out_frame->pkt_dts = pic->dts;
    out_frame->duration = pic->duration;
    out_frame->color_range = ctx->color_range;
    out_frame->colorspace = ctx->color_space;
    out_frame->color_primaries = ctx->color_primaries;
    out_frame->color_trc = ctx->color_trc;
    out_frame->chroma_location = ctx->chroma_location;
    out_frame->sample_aspect_ratio = ctx->sample_aspect_ratio;
    out_frame->time_base = ctx->time_base;
    return 0;
}

/* -------------------------------------------------------------------------
 * Public C API
 * ------------------------------------------------------------------------- */

void shrimply_nvdec_scrub_free(ShrimplyNvdecScrub *ctx);

ShrimplyNvdecScrub *shrimply_nvdec_scrub_new(AVBufferRef *hw_device_ctx,
                                             const AVCodecParameters *codecpar,
                                             AVRational time_base) {
    if (!hw_device_ctx || !codecpar) return NULL;

    cudaVideoCodec nv_codec;
    const char *bsf_name = NULL;
    if (codecpar->codec_id == AV_CODEC_ID_H264) {
        nv_codec = cudaVideoCodec_H264;
        bsf_name = "h264_mp4toannexb";
    } else if (codecpar->codec_id == AV_CODEC_ID_HEVC) {
        nv_codec = cudaVideoCodec_HEVC;
        bsf_name = "hevc_mp4toannexb";
    } else {
        return NULL;
    }

    if (codecpar->format != AV_PIX_FMT_NONE &&
        codecpar->format != AV_PIX_FMT_YUV420P &&
        codecpar->format != AV_PIX_FMT_YUVJ420P &&
        codecpar->format != AV_PIX_FMT_YUV420P10LE) {
        return NULL;
    }
    if (codecpar->field_order != AV_FIELD_UNKNOWN &&
        codecpar->field_order != AV_FIELD_PROGRESSIVE) {
        return NULL;
    }

    AVHWDeviceContext *dev_ctx = (AVHWDeviceContext *)hw_device_ctx->data;
    if (!dev_ctx || dev_ctx->type != AV_HWDEVICE_TYPE_CUDA || !dev_ctx->hwctx) return NULL;
    AVCUDADeviceContext *cuda_hw = (AVCUDADeviceContext *)dev_ctx->hwctx;
    if (!cuda_hw->cuda_ctx) return NULL;

    ShrimplyNvdecScrub *ctx = (ShrimplyNvdecScrub *)calloc(1, sizeof(*ctx));
    if (!ctx) return NULL;

    if (cuda_load_functions(&ctx->cu, NULL) < 0 || cuvid_load_functions(&ctx->cv, NULL) < 0) {
        cuvid_free_functions(&ctx->cv);
        cuda_free_functions(&ctx->cu);
        free(ctx);
        return NULL;
    }

    ctx->hw_device_ref = av_buffer_ref(hw_device_ctx);
    if (!ctx->hw_device_ref) {
        cuvid_free_functions(&ctx->cv);
        cuda_free_functions(&ctx->cu);
        free(ctx);
        return NULL;
    }

    ctx->cuda_ctx = cuda_hw->cuda_ctx;
    ctx->codec_id = codecpar->codec_id;
    ctx->nv_codec = nv_codec;
    ctx->time_base = time_base;
    ctx->width = codecpar->width;
    ctx->height = codecpar->height;
    ctx->color_range = codecpar->color_range;
    ctx->color_space = codecpar->color_space;
    ctx->color_primaries = codecpar->color_primaries;
    ctx->color_trc = codecpar->color_trc;
    ctx->chroma_location = codecpar->chroma_location;
    ctx->sample_aspect_ratio = codecpar->sample_aspect_ratio;

    const AVBitStreamFilter *filter = av_bsf_get_by_name(bsf_name);
    if (!filter || av_bsf_alloc(filter, &ctx->bsf) < 0 ||
        avcodec_parameters_copy(ctx->bsf->par_in, codecpar) < 0) {
        goto fail;
    }
    ctx->bsf->time_base_in = time_base;
    if (av_bsf_init(ctx->bsf) < 0) goto fail;
    ctx->bsf_pkt = av_packet_alloc();
    if (!ctx->bsf_pkt) goto fail;

    CUcontext dummy = NULL;
    if (ctx->cu->cuCtxPushCurrent(ctx->cuda_ctx) != CUDA_SUCCESS) goto fail;
    CUresult sres = ctx->cu->cuStreamCreate(&ctx->stream, CU_STREAM_NON_BLOCKING);
    ctx->cu->cuCtxPopCurrent(&dummy);
    if (sres != CUDA_SUCCESS) goto fail;

    invalidate_live_slots(ctx);
    clear_parsed_gop(ctx);

    /* Pre-create the persistent CUvideodecoder from codecpar dimensions (0 packets read,
     * 0 pictures decoded) so the first scrub seek avoids ~30 ms NVDEC session init. */
    if (codecpar->width > 0 && codecpar->height > 0 && codecpar->format != AV_PIX_FMT_NONE) {
        int align = (codecpar->codec_id == AV_CODEC_ID_H264) ? 16 : 8;
        int coded_w = (codecpar->width + align - 1) & ~(align - 1);
        int coded_h = (codecpar->height + align - 1) & ~(align - 1);
        int bit_depth_minus8 = (codecpar->format == AV_PIX_FMT_YUV420P10LE) ? 2 : 0;
        if (ctx->cu->cuCtxPushCurrent(ctx->cuda_ctx) == CUDA_SUCCESS) {
            int surfaces = ensure_decoder(
                ctx,
                coded_w,
                coded_h,
                0,
                0,
                codecpar->width,
                codecpar->height,
                bit_depth_minus8,
                8
            );
            ctx->cu->cuCtxPopCurrent(&dummy);
            if (surfaces == 0) {
                shrimply_nvdec_scrub_free(ctx);
                return NULL;
            }
        }
    }
    return ctx;

fail:
    if (ctx->bsf_pkt) av_packet_free(&ctx->bsf_pkt);
    if (ctx->bsf) av_bsf_free(&ctx->bsf);
    av_buffer_unref(&ctx->hw_device_ref);
    cuvid_free_functions(&ctx->cv);
    cuda_free_functions(&ctx->cu);
    free(ctx);
    return NULL;
}

void shrimply_nvdec_scrub_free(ShrimplyNvdecScrub *ctx) {
    if (!ctx) return;
    CUcontext dummy = NULL;
    int pushed = (ctx->cu && ctx->cuda_ctx && ctx->cu->cuCtxPushCurrent(ctx->cuda_ctx) == CUDA_SUCCESS);
    if (ctx->stream && ctx->cu) {
        ctx->cu->cuStreamSynchronize(ctx->stream);
    }
    if (ctx->parser && ctx->cv) {
        ctx->cv->cuvidDestroyVideoParser(ctx->parser);
    }
    if (ctx->decoder && ctx->cv) {
        ctx->cv->cuvidDestroyDecoder(ctx->decoder);
    }
    if (ctx->stream && ctx->cu) {
        ctx->cu->cuStreamDestroy(ctx->stream);
    }
    if (pushed) {
        ctx->cu->cuCtxPopCurrent(&dummy);
    }
    clear_parsed_gop(ctx);
    free(ctx->parsed_pics);
    free(ctx->bs_arena);
    free(ctx->slice_arena);
    free(ctx->closure_mark);
    free(ctx->decode_mark);
    free(ctx->closure_list);
    if (ctx->bsf_pkt) av_packet_free(&ctx->bsf_pkt);
    if (ctx->bsf) av_bsf_free(&ctx->bsf);
    av_buffer_unref(&ctx->hw_frames_ref);
    av_buffer_unref(&ctx->hw_device_ref);
    cuvid_free_functions(&ctx->cv);
    cuda_free_functions(&ctx->cu);
    free(ctx);
}

void shrimply_nvdec_scrub_flush_bsf(ShrimplyNvdecScrub *ctx) {
    if (ctx && ctx->bsf) {
        av_bsf_flush(ctx->bsf);
    }
}

int shrimply_nvdec_scrub_filter_packet(ShrimplyNvdecScrub *ctx, const AVPacket *in_pkt,
                                       uint8_t **out_data, int *out_size) {
    if (!ctx || !ctx->bsf || !in_pkt || !out_data || !out_size) return -1;
    *out_data = NULL;
    *out_size = 0;

    av_packet_unref(ctx->bsf_pkt);
    if (av_packet_ref(ctx->bsf_pkt, in_pkt) < 0) return -1;
    if (av_bsf_send_packet(ctx->bsf, ctx->bsf_pkt) < 0) {
        av_packet_unref(ctx->bsf_pkt);
        return -1;
    }
    int r = av_bsf_receive_packet(ctx->bsf, ctx->bsf_pkt);
    if (r < 0) {
        av_packet_unref(ctx->bsf_pkt);
        return -1;
    }
    *out_data = ctx->bsf_pkt->data;
    *out_size = ctx->bsf_pkt->size;
    return 0;
}

uint64_t shrimply_nvdec_scrub_staging_bytes(const ShrimplyNvdecScrub *ctx) {
    if (!ctx) return 0;
    return ctx->staging_bytes
        + (uint64_t)ctx->bs_arena_cap
        + (uint64_t)ctx->slice_arena_cap * sizeof(unsigned)
        + (uint64_t)ctx->parsed_pic_cap * sizeof(ParsedPic);
}

int shrimply_nvdec_scrub_decode(ShrimplyNvdecScrub *ctx, uint64_t gop_id,
                                const ShrimplyGopPacket *packets, int packet_count,
                                int target_packet_index, AVFrame *out_frame,
                                ShrimplyScrubStats *stats) {
    if (!ctx || !packets || packet_count <= 0 || target_packet_index < 0 ||
        target_packet_index >= packet_count || !out_frame) {
        return SHRIMPLY_SCRUB_ERROR;
    }
    if (stats) {
        memset(stats, 0, sizeof(*stats));
    }

    CUcontext dummy = NULL;
    if (ctx->cu->cuCtxPushCurrent(ctx->cuda_ctx) != CUDA_SUCCESS) {
        return SHRIMPLY_SCRUB_ERROR;
    }

    int ret = SHRIMPLY_SCRUB_OK;
    int forward_continued = 0;
    uint64_t t_parse0 = shrimply_now_us();

    /* 1. Ensure GOP packets up through target_packet_index are parsed into ctx->parsed_pics[]. */
    if (ctx->parsed_gop_id != gop_id || !ctx->parser) {
        clear_parsed_gop(ctx);
        if (reset_parser(ctx) < 0) {
            ret = SHRIMPLY_SCRUB_ERROR;
            goto done;
        }
        ctx->parsed_gop_id = gop_id;
    }

    ctx->parse_error = 0;
    ctx->parse_fallback = 0;

    if (target_packet_index >= ctx->parsed_packets) {
        int start_pkt = ctx->parsed_packets;
        if (start_pkt > 0) {
            forward_continued = 1;
        }
        for (int i = start_pkt; i <= target_packet_index; i++) {
            ctx->current_packet_index = i;
            ctx->current_pts = packets[i].pts;
            ctx->current_dts = packets[i].dts;
            ctx->current_duration = packets[i].duration;
            CUVIDSOURCEDATAPACKET sp = {0};
            sp.flags = CUVID_PKT_TIMESTAMP | CUVID_PKT_ENDOFPICTURE;
            sp.payload_size = (unsigned long)packets[i].size;
            sp.payload = packets[i].data;
            sp.timestamp = packets[i].pts;
            if (ctx->cv->cuvidParseVideoData(ctx->parser, &sp) != CUDA_SUCCESS) {
                ctx->parse_error = 1;
                break;
            }
            if (ctx->parse_error || ctx->parse_fallback) break;
        }
        ctx->parsed_packets = target_packet_index + 1;
        if (ctx->parse_fallback) {
            clear_parsed_gop(ctx);
            ret = SHRIMPLY_SCRUB_FALLBACK;
            goto done;
        }
        if (ctx->parse_error || !ctx->decoder) {
            clear_parsed_gop(ctx);
            ret = SHRIMPLY_SCRUB_ERROR;
            goto done;
        }
    }

    /* Locate parsed picture corresponding to target_packet_index. */
    int target_pic = -1;
    for (int i = ctx->parsed_pic_count - 1; i >= 0; i--) {
        if (ctx->parsed_pics[i].packet_index == target_packet_index) {
            target_pic = i;
            break;
        }
    }
    if (target_pic < 0) {
        clear_parsed_gop(ctx);
        ret = SHRIMPLY_SCRUB_FALLBACK;
        goto done;
    }

    ParsedPic *target = &ctx->parsed_pics[target_pic];
    int target_slot = target->params.CurrPicIdx;
    uint64_t parse_us = shrimply_now_us() - t_parse0;

    /* 2. Path A: Target is already live and untouched in NVDEC's decode surface! */
    if (ctx->slot_valid[target_slot] &&
        ctx->slot_gop_id[target_slot] == gop_id &&
        ctx->slot_pic_idx[target_slot] == target_pic) {
        uint64_t t_map0 = shrimply_now_us();
        if (map_and_copy_surface(ctx, target_slot, target, out_frame) < 0) {
            invalidate_live_slots(ctx);
            ret = SHRIMPLY_SCRUB_ERROR;
            goto done;
        }
        uint64_t map_us = shrimply_now_us() - t_map0;
        if (stats) {
            stats->pictures_decoded = 0;
            stats->live_surface_hit = 1;
            stats->parse_us = parse_us;
            stats->decode_us = 0;
            stats->map_copy_us = map_us;
        }
        goto done;
    }

    /* 3. Compute exact transitive dependency closure C of target_pic. */
    if (ensure_scratch(ctx, ctx->parsed_pic_count) < 0) {
        ret = SHRIMPLY_SCRUB_ERROR;
        goto done;
    }
    memset(ctx->closure_mark, 0, (size_t)ctx->parsed_pic_count);
    ctx->closure_mark[target_pic] = 1;
    for (int i = target_pic; i >= 0; i--) {
        if (!ctx->closure_mark[i]) continue;
        const ParsedPic *p = &ctx->parsed_pics[i];
        for (int r = 0; r < p->ref_count; r++) {
            int dep = p->refs[r];
            if (dep >= 0 && dep < i) {
                ctx->closure_mark[dep] = 1;
            }
        }
    }
    int closure_len = 0;
    for (int i = 0; i <= target_pic; i++) {
        if (ctx->closure_mark[i]) {
            ctx->closure_list[closure_len++] = i;
        }
    }

    /* 4. Unified minimal live-DPB / forward-continuation / sparse-closure planner:
     *    NVDEC's internal DPB/colocated-MV state is only valid monotonically forward
     *    across reference pictures within the same GOP; if jumping backward or if any
     *    required dependency precedes live_dpb_last_ref_pic (was skipped earlier), replay
     *    the clean transitive closure from the IDR so internal MV tables match. */
    memset(ctx->decode_mark, 0, (size_t)ctx->parsed_pic_count);
    int can_reuse_dpb = (ctx->live_dpb_gop_id == gop_id && target_pic >= ctx->live_dpb_last_ref_pic);
    if (can_reuse_dpb) {
        ctx->decode_mark[target_pic] = 1;
        int converged = 0;
        for (int iter = 0; iter <= closure_len; iter++) {
            int changed = 0;
            /* Backward pull: any needed pic whose direct ref is not currently live pulls that ref. */
            for (int ci = closure_len - 1; ci >= 0; ci--) {
                int k = ctx->closure_list[ci];
                if (!ctx->decode_mark[k]) continue;
                const ParsedPic *p = &ctx->parsed_pics[k];
                for (int r = 0; r < p->ref_count; r++) {
                    int dep = p->refs[r];
                    int slot = p->ref_slots[r];
                    if (!(ctx->slot_valid[slot] &&
                          ctx->slot_gop_id[slot] == gop_id &&
                          ctx->slot_pic_idx[slot] == dep)) {
                        if (!ctx->decode_mark[dep]) {
                            ctx->decode_mark[dep] = 1;
                            changed = 1;
                        }
                    }
                }
            }
            /* Forward simulation: verify no earlier decoded pic clobbers a slot before a later pic needs it. */
            int sim_valid[SHRIMPLY_MAX_SLOTS];
            uint64_t sim_gop[SHRIMPLY_MAX_SLOTS];
            int sim_pic[SHRIMPLY_MAX_SLOTS];
            memcpy(sim_valid, ctx->slot_valid, sizeof(sim_valid));
            memcpy(sim_gop, ctx->slot_gop_id, sizeof(sim_gop));
            memcpy(sim_pic, ctx->slot_pic_idx, sizeof(sim_pic));
            for (int ci = 0; ci < closure_len; ci++) {
                int k = ctx->closure_list[ci];
                if (!ctx->decode_mark[k]) continue;
                if (k < ctx->live_dpb_last_ref_pic) {
                    /* Would submit a picture older than NVDEC's monotonic reference DPB frontier. */
                    can_reuse_dpb = 0;
                    break;
                }
                const ParsedPic *p = &ctx->parsed_pics[k];
                for (int r = 0; r < p->ref_count; r++) {
                    int dep = p->refs[r];
                    int slot = p->ref_slots[r];
                    if (!(sim_valid[slot] && sim_gop[slot] == gop_id && sim_pic[slot] == dep)) {
                        if (!ctx->decode_mark[dep]) {
                            ctx->decode_mark[dep] = 1;
                            changed = 1;
                        }
                    }
                }
                int dst_slot = p->params.CurrPicIdx;
                sim_valid[dst_slot] = 1;
                sim_gop[dst_slot] = gop_id;
                sim_pic[dst_slot] = k;
            }
            if (!can_reuse_dpb) break;
            if (!changed) {
                converged = 1;
                break;
            }
        }
        if (!converged) {
            can_reuse_dpb = 0;
        }
    }
    if (!can_reuse_dpb) {
        invalidate_live_slots(ctx);
        memset(ctx->decode_mark, 0, (size_t)ctx->parsed_pic_count);
        for (int ci = 0; ci < closure_len; ci++) {
            ctx->decode_mark[ctx->closure_list[ci]] = 1;
        }
    }

    /* 5. Submit ONLY the planned subset D in ascending decoding order. */
    uint64_t t_dec0 = shrimply_now_us();
    int decoded = 0;
    for (int ci = 0; ci < closure_len; ci++) {
        int k = ctx->closure_list[ci];
        if (!ctx->decode_mark[k]) continue;
        ParsedPic *p = &ctx->parsed_pics[k];
        p->params.pBitstreamData = ctx->bs_arena + p->bs_offset;
        p->params.pSliceDataOffsets = ctx->slice_arena + p->slice_offset;
        if (ctx->cv->cuvidDecodePicture(ctx->decoder, &p->params) != CUDA_SUCCESS) {
            invalidate_live_slots(ctx);
            ret = SHRIMPLY_SCRUB_ERROR;
            goto done;
        }
        int dst_slot = p->params.CurrPicIdx;
        ctx->slot_valid[dst_slot] = 1;
        ctx->slot_gop_id[dst_slot] = gop_id;
        ctx->slot_pic_idx[dst_slot] = k;
        if (p->params.ref_pic_flag) {
            ctx->live_dpb_last_ref_pic = k;
        }
        decoded++;
    }
    ctx->live_dpb_gop_id = gop_id;
    uint64_t t_map0 = shrimply_now_us();
    uint64_t decode_us = t_map0 - t_dec0;

    /* 6. Map & D2D-copy ONLY the target picture into out_frame on our dedicated stream. */
    if (map_and_copy_surface(ctx, target_slot, target, out_frame) < 0) {
        invalidate_live_slots(ctx);
        ret = SHRIMPLY_SCRUB_ERROR;
        goto done;
    }
    uint64_t map_us = shrimply_now_us() - t_map0;

    if (stats) {
        stats->pictures_decoded = decoded;
        stats->live_surface_hit = 0;
        stats->live_dpb_hit = (decoded == 1 && !target->params.ref_pic_flag && closure_len > 1);
        stats->forward_continued = forward_continued && decoded < closure_len;
        stats->parse_us = parse_us;
        stats->decode_us = decode_us;
        stats->map_copy_us = map_us;
    }

done:
    ctx->cu->cuCtxPopCurrent(&dummy);
    return ret;
}
