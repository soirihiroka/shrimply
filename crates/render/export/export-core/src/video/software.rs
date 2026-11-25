use super::{ExportRateControl, ExportSettings, ExportVideoCodec, configure_video_encoder};
use ffmpeg::format::Pixel;
use ffmpeg::software::scaling::{Context, flag::Flags};
use ffmpeg_next as ffmpeg;
use shrimply_project_document::Project;

pub(super) fn open(
    project: &Project,
    settings: &ExportSettings,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let h264 = settings.video_encoder.codec() == ExportVideoCodec::H264;
    let name = settings.video_encoder.format().ffmpeg;
    let codec = ffmpeg::codec::encoder::find_by_name(name)
        .ok_or_else(|| format!("Software encoder {name} is unavailable"))?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    configure_video_encoder(
        &mut encoder,
        project,
        settings,
        Pixel::YUV420P,
        global_header,
    )?;

    let mut options = ffmpeg::Dictionary::new();
    options.set("preset", "veryfast");
    let mut x265_params = if h264 {
        String::new()
    } else {
        format!(
            "rc-lookahead={}",
            15_u32.max(settings.b_frames.saturating_add(1))
        )
    };
    let peak_kbps = match settings.rate_control {
        ExportRateControl::ConstantQp => {
            encoder.set_bit_rate(0);
            options.set("qp", &settings.constant_qp.to_string());
            0
        }
        ExportRateControl::ConstantBitrate => {
            if h264 {
                options.set("x264-params", "filler=1");
            } else {
                x265_params.push_str(":strict-cbr=1");
            }
            settings.bitrate_kbps
        }
        ExportRateControl::VariableBitrate => settings.max_bitrate_kbps,
        ExportRateControl::VariableBitrateTargetQuality => {
            options.set("crf", &settings.target_quality.to_string());
            settings.max_bitrate_kbps
        }
        ExportRateControl::Lossless => {
            encoder.set_bit_rate(0);
            if h264 {
                options.set("qp", "0");
            } else {
                x265_params.push_str(":lossless=1");
            }
            0
        }
    };
    if !h264 {
        options.set("x265-params", &x265_params);
    }
    if peak_kbps != 0 {
        let peak = i32::try_from(u64::from(peak_kbps) * 1_000)
            .map_err(|_| "Software encoder bitrate exceeds its supported range")?;
        encoder.set_max_bit_rate(peak as usize);
        unsafe { (*encoder.as_mut_ptr()).rc_buffer_size = peak };
    }
    encoder
        .open_as_with(codec, options)
        .map_err(|error| format!("Open software encoder {name}: {error}"))
}

pub fn rgba_to_bt709(
    scaler: &mut Option<Context>,
    rgba: &ffmpeg::frame::Video,
    pixel: Pixel,
) -> Result<ffmpeg::frame::Video, String> {
    const SWS_COLORSPACE_BT709: i32 = 1;
    const SWS_FIXED_POINT_ONE: i32 = 1 << 16;
    let width = rgba.width();
    let height = rgba.height();
    if scaler.is_none() {
        let mut context = Context::get(
            Pixel::RGBA,
            width,
            height,
            pixel,
            width,
            height,
            Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        let coefficients = unsafe { ffmpeg::sys::sws_getCoefficients(SWS_COLORSPACE_BT709) };
        super::ffmpeg_check(
            unsafe {
                ffmpeg::sys::sws_setColorspaceDetails(
                    context.as_mut_ptr(),
                    coefficients,
                    1,
                    coefficients,
                    0,
                    0,
                    SWS_FIXED_POINT_ONE,
                    SWS_FIXED_POINT_ONE,
                )
            },
            "configure BT.709 video conversion",
        )?;
        *scaler = Some(context);
    }
    let mut frame = ffmpeg::frame::Video::new(pixel, width, height);
    scaler
        .as_mut()
        .expect("initialized scaler")
        .run(rgba, &mut frame)
        .map_err(|error| error.to_string())?;
    super::set_bt709_frame_metadata(&mut frame);
    Ok(frame)
}
