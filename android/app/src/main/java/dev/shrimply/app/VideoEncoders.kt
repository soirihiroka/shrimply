package dev.shrimply.app

import android.media.MediaCodecInfo
import android.media.MediaCodecList

internal fun videoEncoders(mime: String, hardware: Boolean): Sequence<Pair<MediaCodecInfo, MediaCodecInfo.CodecCapabilities>> {
    val codecs = runCatching { MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos }.getOrDefault(emptyArray())
    return codecs.asSequence().mapNotNull { codec ->
        val matches = if (hardware) codec.isHardwareAccelerated && !codec.isSoftwareOnly else codec.isSoftwareOnly
        if (!codec.isEncoder || !matches || codec.supportedTypes.none { it.equals(mime, ignoreCase = true) }) {
            return@mapNotNull null
        }
        val capabilities = runCatching { codec.getCapabilitiesForType(mime) }.getOrNull() ?: return@mapNotNull null
        codec to capabilities
    }
}
