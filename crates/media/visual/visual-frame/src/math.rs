use ffmpeg_next::{self as ffmpeg, sys};

const HD_MINIMUM_HEIGHT: u32 = 720;

pub fn video_color_space(frame: &ffmpeg::frame::Video) -> Result<ffmpeg::color::Space, String> {
    use ffmpeg::color::Space;
    match frame.color_space() {
        Space::Unspecified if frame.height() >= HD_MINIMUM_HEIGHT => Ok(Space::BT709),
        Space::Unspecified => Ok(Space::SMPTE170M),
        space @ (Space::BT709
        | Space::BT470BG
        | Space::SMPTE170M
        | Space::FCC
        | Space::SMPTE240M) => Ok(space),
        space => Err(format!("Unsupported video color matrix: {space:?}")),
    }
}

pub struct VideoColor {
    pub coefficients: [i32; 4],
    pub chroma_offset: [f32; 2],
    pub full_range: bool,
}

pub fn video_color(frame: &ffmpeg::frame::Video) -> Result<VideoColor, String> {
    let space = video_color_space(frame)?;
    let coefficients = unsafe {
        sys::sws_getCoefficients(sys::AVColorSpace::from(space) as i32)
            .cast::<[i32; 4]>()
            .as_ref()
    }
    .copied()
    .ok_or("video color matrix is unavailable")?;
    let mut x = 0;
    let mut y = 0;
    let location = unsafe { (*frame.as_ptr()).chroma_location };
    let location = if location == sys::AVChromaLocation::AVCHROMA_LOC_UNSPECIFIED {
        sys::AVChromaLocation::AVCHROMA_LOC_LEFT
    } else {
        location
    };
    if unsafe { sys::av_chroma_location_enum_to_pos(&mut x, &mut y, location) } < 0 {
        return Err(format!("unsupported video chroma location: {location:?}"));
    }
    // FFmpeg measures chroma positions in 1/256 luma pixels.
    const CHROMA_POSITION_SCALE: f32 = 256.0;
    Ok(VideoColor {
        coefficients,
        chroma_offset: [x, y].map(|position| position as f32 / CHROMA_POSITION_SCALE),
        full_range: frame.color_range() == ffmpeg::color::Range::JPEG,
    })
}
