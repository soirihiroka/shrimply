//! The platform supplies codecs missing from bundled Skia; preparation stays shared.
use ndk_sys as ndk;
use skia_safe::{AlphaType, ColorSpace, ColorType, Data, Image, ImageInfo};
use std::{ffi::CStr, ptr::NonNull};

struct Decoder(NonNull<ndk::AImageDecoder>);
impl Drop for Decoder {
    fn drop(&mut self) {
        // The borrowed input bytes outlive this local decoder on every return path.
        unsafe { ndk::AImageDecoder_delete(self.0.as_ptr()) };
    }
}

fn check(status: i32) -> Result<(), String> {
    if status == ndk::ANDROID_IMAGE_DECODER_SUCCESS {
        return Ok(());
    }
    let message = unsafe { ndk::AImageDecoder_resultToString(status) };
    let message = if message.is_null() {
        status.to_string()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    Err(format!("Android image decode failed: {message}"))
}

pub(super) fn decode(bytes: &[u8], max_pixels: usize) -> Result<Option<(Image, [u32; 2])>, String> {
    let mut decoder = std::ptr::null_mut();
    let status = unsafe {
        ndk::AImageDecoder_createFromBuffer(bytes.as_ptr().cast(), bytes.len(), &mut decoder)
    };
    if status == ndk::ANDROID_IMAGE_DECODER_UNSUPPORTED_FORMAT {
        return Ok(None);
    }
    check(status)?;
    let decoder = Decoder(NonNull::new(decoder).ok_or("Android image decoder is null")?);
    let header = unsafe { ndk::AImageDecoder_getHeaderInfo(decoder.0.as_ptr()) };
    let header = NonNull::new(header.cast_mut()).ok_or("Android image header is null")?;
    // Header dimensions describe logical output, including encoded orientation.
    let width = unsafe { ndk::AImageDecoderHeaderInfo_getWidth(header.as_ptr()) };
    let height = unsafe { ndk::AImageDecoderHeaderInfo_getHeight(header.as_ptr()) };
    let size = [
        u32::try_from(width).map_err(|_| "invalid image width")?,
        u32::try_from(height).map_err(|_| "invalid image height")?,
    ];
    super::validate_size(size, max_pixels)?;
    check(unsafe {
        ndk::AImageDecoder_setAndroidBitmapFormat(
            decoder.0.as_ptr(),
            i32::try_from(ndk::AndroidBitmapFormat::ANDROID_BITMAP_FORMAT_RGBA_8888.0)
                .expect("Android bitmap format fits i32"),
        )
    })?;
    check(unsafe { ndk::AImageDecoder_setUnpremultipliedRequired(decoder.0.as_ptr(), true) })?;
    // Convert embedded profiles to the declared output space before handing pixels to Skia.
    check(unsafe {
        ndk::AImageDecoder_setDataSpace(decoder.0.as_ptr(), ndk::ADataSpace::ADATASPACE_SRGB.0)
    })?;
    let info = ImageInfo::new(
        (width, height),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        ColorSpace::new_srgb(),
    );
    let stride = unsafe { ndk::AImageDecoder_getMinimumStride(decoder.0.as_ptr()) };
    if stride < info.min_row_bytes() || !stride.is_multiple_of(info.bytes_per_pixel()) {
        return Err("Android image decoder returned an invalid stride".into());
    }
    let length = stride
        .checked_mul(size[1] as usize)
        .ok_or("image byte size overflow")?;
    let mut pixels = Vec::<u8>::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| "could not allocate decoded image")?;
    pixels.resize(length, 0);
    check(unsafe {
        ndk::AImageDecoder_decodeImage(
            decoder.0.as_ptr(),
            pixels.as_mut_ptr().cast(),
            stride,
            length,
        )
    })?;
    let image = skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), stride)
        .ok_or("could not prepare the Android decoded image")?;
    Ok(Some((image, size)))
}
