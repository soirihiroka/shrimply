//! Still-media metadata and native decoding use the editor's shared media cache.
use sanim_core::media::Asset;
#[cfg(feature = "media")]
use sanim_core::media::Kind;
use std::sync::Arc;
pub fn decode(bytes: Arc<Vec<u8>>, limits: super::Limits) -> Result<Arc<Asset>, String> {
    if bytes.len() > limits.max_bytes || limits.max_values == 0 {
        return Err("media exceeds its source budget".into());
    }
    #[cfg(not(feature = "media"))]
    {
        Err(
            "still-media decoding is unavailable in this build; enable Sanim's media feature"
                .into(),
        )
    }
    #[cfg(feature = "media")]
    {
        let fingerprint = sanim_core::digest(&bytes);
        let prepared = shrimply_drawing_skia::media::prepare_hashed_with_limit(
            &bytes,
            &fingerprint,
            limits.max_bytes / 4,
        )?;
        let [width, height] = prepared.dimensions();
        let kind = match prepared.kind() {
            shrimply_drawing_skia::media::Kind::Bitmap => Kind::Bitmap,
            shrimply_drawing_skia::media::Kind::Svg => Kind::Svg,
        };
        Asset::new(kind, bytes, width, height).map(Arc::new)
    }
}
