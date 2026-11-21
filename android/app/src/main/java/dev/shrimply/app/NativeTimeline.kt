package dev.shrimply.app

import android.os.Handler
import android.os.Looper
import android.view.Surface
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.unit.DpOffset
import kotlinx.coroutines.suspendCancellableCoroutine
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicLong
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

internal fun interface TimelineListener { fun changed(payload: String) }

internal object NativeTimeline {
    init { System.loadLibrary("shrimply_android") }
    external fun initialize(application: android.content.Context, config: String, cache: String, fonts: String)
    external fun throughput(path: String, scene: String, frames: Int): String
    external fun create(stress: Boolean, benchmark: Boolean, files: ProjectFiles, listener: TimelineListener): Long
    external fun request(handle: Long, request: String, descriptor: Int): String
    external fun surface(handle: Long, role: Int, surface: Surface?, width: Int, height: Int, density: Float)
    external fun visible(handle: Long, role: Int, visible: Boolean)
    external fun resize(handle: Long, role: Int, width: Int, height: Int)
    external fun active(handle: Long, active: Boolean)
    external fun input(handle: Long, role: Int, batch: FloatArray): Boolean
    external fun destroy(handle: Long)
}

/** Project I/O runs on one executor. Rendering and exact playback belong to Rust.
 * Lifecycle surface release remains synchronous; a closed handle is never reused.
 */
internal class TimelineSession(stress: Boolean, benchmark: Boolean, projects: ProjectFiles) : AutoCloseable {
    val identity = java.util.UUID.randomUUID().toString()
    val snapshot = mutableStateOf(JSONObject())
    var menuOrigin = DpOffset.Zero
    private val main = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()
    private val closed = AtomicBoolean(false)
    private val surfaceRoles = java.util.concurrent.ConcurrentHashMap.newKeySet<Int>()
    private val latest = AtomicReference<String?>(null)
    private val posted = AtomicBoolean(false)
    private val update = object : Runnable {
        override fun run() {
            if (closed.get()) { latest.set(null); posted.set(false); return }
            latest.getAndSet(null)?.let { snapshot.value = JSONObject(it) }
            posted.set(false)
            // A producer can race with the clear above; exactly one side posts.
            if (latest.get() != null && posted.compareAndSet(false, true)) main.post(this)
        }
    }
    private val handle = AtomicLong(NativeTimeline.create(stress, benchmark, projects, TimelineListener { payload ->
        if (!closed.get()) {
            latest.set(payload)
            if (posted.compareAndSet(false, true)) main.post(update)
        }
    }))

    suspend fun request(command: JSONObject): JSONObject = request(command.getString("op"),
        *command.keys().asSequence().filter { it != "op" }.map { it to command.get(it) }.toList().toTypedArray())

    suspend fun request(operation: String, vararg arguments: Pair<String, Any>, descriptor: java.io.FileDescriptor? = null): JSONObject = suspendCancellableCoroutine { continuation ->
        val request = JSONObject().put("op", operation)
        arguments.forEach { (key, value) -> request.put(key, value) }
        executor.execute {
            try {
                val current = handle.get()
                check(current != 0L) { "The editor session is closed." }
                // Transfer a duplicate: scoped-storage grants permit reading the
                // descriptor but need not permit reopening its /proc path.
                val owned = descriptor?.let { android.os.ParcelFileDescriptor.dup(it) }
                val response = JSONObject(NativeTimeline.request(current, request.toString(), owned?.detachFd() ?: -1))
                check(response.getBoolean("ok")) { response.optString("error", "The project operation failed.") }
                val result = response.optJSONObject("result") ?: JSONObject()
                if (continuation.isActive) continuation.resume(result)
            } catch (error: Exception) {
                if (continuation.isActive) continuation.resumeWithException(error)
            }
        }
    }
    fun surface(role: Int, surface: Surface?, width: Int, height: Int, density: Float) {
        val current = handle.get()
        if (current != 0L) {
            if (surface == null) surfaceRoles.remove(role) else surfaceRoles.add(role)
            NativeTimeline.surface(current, role, surface, width, height, density)
        }
    }
    fun visible(role: Int, visible: Boolean) {
        val current = handle.get()
        if (current != 0L) NativeTimeline.visible(current, role, visible)
    }
    fun resize(role: Int, width: Int, height: Int) {
        val current = handle.get()
        if (current != 0L) NativeTimeline.resize(current, role, width, height)
    }
    fun active(active: Boolean) {
        val current = handle.get()
        if (current != 0L) NativeTimeline.active(current, active)
    }
    fun input(batch: FloatArray, role: Int = 0): Boolean {
        val current = handle.get()
        return current != 0L && NativeTimeline.input(current, role, batch)
    }
    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        ScreenRecordingService.stopOwner(identity)
        latest.set(null)
        main.removeCallbacks(update)
        val closing = handle.getAndSet(0)
        if (closing == 0L) return
        // Surface release acknowledges on the Vulkan worker, independently of queued imports.
        for (role in surfaceRoles) NativeTimeline.surface(closing, role, null, 0, 0, 1f)
        surfaceRoles.clear()
        executor.execute { NativeTimeline.destroy(closing) }
        executor.shutdown()
    }
}
