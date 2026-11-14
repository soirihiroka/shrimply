package dev.shrimply.tablet

import android.view.Surface

internal object NativeTimeline {
    init { System.loadLibrary("shrimply_android") }
    external fun create(stress: Boolean, benchmark: Boolean): Long
    external fun surface(handle: Long, surface: Surface?, width: Int, height: Int, density: Float)
    external fun resize(handle: Long, width: Int, height: Int)
    external fun active(handle: Long, active: Boolean)
    external fun input(handle: Long, batch: FloatArray)
    external fun destroy(handle: Long)
}

// The Activity and Compose callbacks run on the main thread. Closing clears the
// handle before callbacks can observe it again; surface release is synchronous.
internal class TimelineSession(stress: Boolean, benchmark: Boolean) {
    private var handle = NativeTimeline.create(stress, benchmark)
    fun surface(surface: Surface?, width: Int, height: Int, density: Float) {
        if (handle != 0L) NativeTimeline.surface(handle, surface, width, height, density)
    }
    fun resize(width: Int, height: Int) {
        if (handle != 0L) NativeTimeline.resize(handle, width, height)
    }
    fun active(active: Boolean) { if (handle != 0L) NativeTimeline.active(handle, active) }
    fun input(batch: FloatArray) { if (handle != 0L) NativeTimeline.input(handle, batch) }
    fun close() {
        val closing = handle
        handle = 0
        if (closing != 0L) NativeTimeline.destroy(closing)
    }
}
